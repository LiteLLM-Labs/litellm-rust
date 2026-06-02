use std::io::Write;

use litellm_rust::proxy::config::load_config;
use tempfile::NamedTempFile;

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

#[test]
fn loads_litellm_style_mcp_servers() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        r#"
mcp_servers:
  - id: linear
    url: https://mcp.linear.app/mcp
    api_key: sk-linear
    headers:
      x-workspace: test
general_settings:
  master_key: sk-local
"#
    )
    .unwrap();

    let config = load_config(file.path()).unwrap();
    assert!(config.model_list.is_empty());
    assert_eq!(config.mcp_servers[0].id, "linear");
    assert_eq!(config.mcp_servers[0].url, "https://mcp.linear.app/mcp");
    assert_eq!(config.mcp_servers[0].api_key.as_deref(), Some("sk-linear"));
    assert_eq!(
        config.mcp_servers[0].headers.get("x-workspace").unwrap(),
        "test"
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
