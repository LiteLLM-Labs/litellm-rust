//! Config loader: model_list, upstream litellm cache shim, env expansion, and
//! config-defined agents. (MCP server parsing lives in `config_loader_mcp.rs`.)

use std::io::Write;

use litellm_rust::proxy::config::{load_config, CacheBackendKind};
use tempfile::NamedTempFile;

#[path = "config_loader_support/mod.rs"]
mod support;
use support::write_config;

#[test]
fn loads_litellm_style_anthropic_config() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        r#"
model_list:
  - model_name: claude
    litellm_params:
      model: anthropic/claude-sonnet-4-5
      api_key: sk-ant-test
general_settings:
  master_key: sk-local
"#
    )
    .unwrap();

    let config = load_config(file.path()).unwrap();
    assert_eq!(config.model_list[0].model_name, "claude");
    assert_eq!(
        config.model_list[0].litellm_params.model,
        "anthropic/claude-sonnet-4-5"
    );
}

/// Golden compatibility test: an existing upstream litellm cache stanza
/// (`litellm_settings.cache` + `cache_params`) is honoured on a drop-in config,
/// translated onto the native `general_settings.cache` (type→backend, ttl, dir).
/// Without the shim the whole block is silently dropped and caching stays off.
#[test]
fn honours_upstream_litellm_settings_cache_block() {
    let file = write_config(
        r#"
model_list:
  - model_name: claude
    litellm_params:
      model: anthropic/claude-sonnet-4-5
      api_key: sk-ant-test
litellm_settings:
  cache: true
  cache_params:
    type: disk
    disk_cache_dir: /tmp/litellm-rust-cache
    ttl: 120
"#,
    );
    let config = load_config(file.path()).unwrap();
    let cache = &config.general_settings.cache;
    assert!(
        cache.enabled,
        "upstream litellm_settings.cache: true should enable caching"
    );
    assert_eq!(cache.backend, CacheBackendKind::Redb);
    assert_eq!(
        cache.redb_path.as_deref(),
        Some("/tmp/litellm-rust-cache/litellm-cache.redb")
    );
    assert_eq!(cache.ttl_secs, 120);
}

#[test]
fn expands_database_url_from_general_settings() {
    std::env::set_var(
        "TEST_LITELLM_DATABASE_URL",
        "postgres:///litellm_rust_managed_agents_test",
    );
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        r#"
general_settings:
  database_url: os.environ/TEST_LITELLM_DATABASE_URL
"#
    )
    .unwrap();

    let config = load_config(file.path()).unwrap();
    assert_eq!(
        config.general_settings.database_url.as_deref(),
        Some("postgres:///litellm_rust_managed_agents_test")
    );
}

#[test]
fn loads_config_defined_agent_and_expands_e2b_key() {
    std::env::set_var("E2B_API_KEY", "e2b-test");
    std::env::set_var("ANTHROPIC_API_KEY", "anthropic-test");

    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        r#"
general_settings:
  sandbox_choice: e2b
  e2b_sandbox_params:
    e2b_api_key: os.environ/E2B_API_KEY
    e2b_template: litellm-4gb
    envs:
      ANTHROPIC_API_KEY: os.environ/ANTHROPIC_API_KEY
agents:
  - name: Untitled agent
    description: A blank starting point with the core toolset.
    model: claude-sonnet-4-6
    system: You are a general-purpose agent that can research, write code, run commands, and use connected tools to complete the user's task end to end.
    mcp_servers: []
    tools:
      - type: agent_toolset_20260401
    skills: []
"#
    )
    .unwrap();

    let config = load_config(file.path()).unwrap();
    assert!(config.model_list.is_empty());
    assert_eq!(config.agents[0].id(), "untitled-agent");
    assert_eq!(config.agents[0].model, "claude-sonnet-4-6");
    assert_eq!(
        config
            .general_settings
            .e2b_sandbox_params
            .e2b_api_key
            .as_deref(),
        Some("e2b-test")
    );
    assert_eq!(
        config.general_settings.e2b_sandbox_params.e2b_template,
        "litellm-4gb"
    );
    assert_eq!(
        config
            .general_settings
            .e2b_sandbox_params
            .envs
            .get("ANTHROPIC_API_KEY")
            .map(String::as_str),
        Some("anthropic-test")
    );
}
