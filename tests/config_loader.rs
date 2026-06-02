use std::io::Write;

use litellm_rust::proxy::config::{load_config, McpAuthType, McpTransport};
use tempfile::NamedTempFile;

fn write_config(contents: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    file
}

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

// --- MCP config (LiteLLM-compatible dict-keyed `mcp_servers`) -------------

#[test]
fn loads_litellm_dict_mcp_servers() {
    let file = write_config(
        r#"
mcp_servers:
  linear:
    url: https://mcp.linear.app/mcp
    auth_type: bearer_token
    auth_value: sk-linear
  docs:
    url: https://mcp.example.com/mcp
    auth_type: api_key
    auth_value: sk-docs
    static_headers:
      x-workspace: prod
    extra_headers: ["x-trace-id"]
general_settings:
  master_key: sk-local
"#,
    );

    let config = load_config(file.path()).unwrap();
    assert!(config.model_list.is_empty());
    assert_eq!(config.mcp_servers.len(), 2);

    let linear = &config.mcp_servers["linear"];
    assert_eq!(linear.url, "https://mcp.linear.app/mcp");
    assert_eq!(linear.auth_value.as_deref(), Some("sk-linear"));
    assert_eq!(linear.transport, McpTransport::Http);

    let docs = &config.mcp_servers["docs"];
    assert_eq!(docs.auth_type, McpAuthType::ApiKey);
    assert_eq!(docs.static_headers["x-workspace"], "prod");
    assert_eq!(docs.extra_headers, vec!["x-trace-id".to_owned()]);
}

#[test]
fn defaults_transport_http_and_auth_none() {
    let file = write_config(
        r#"
mcp_servers:
  deepwiki:
    url: https://mcp.deepwiki.com/mcp
"#,
    );
    let config = load_config(file.path()).unwrap();
    let s = &config.mcp_servers["deepwiki"];
    assert_eq!(s.transport, McpTransport::Http);
    assert_eq!(s.auth_type, McpAuthType::None);
    assert!(s.auth_value.is_none());
}

#[test]
fn expands_env_in_mcp_fields() {
    std::env::set_var("TEST_MCP_URL", "https://env.example.com/mcp");
    std::env::set_var("TEST_MCP_KEY", "sk-from-env");
    let file = write_config(
        r#"
mcp_servers:
  s:
    url: os.environ/TEST_MCP_URL
    auth_type: bearer_token
    auth_value: os.environ/TEST_MCP_KEY
"#,
    );
    let config = load_config(file.path()).unwrap();
    let s = &config.mcp_servers["s"];
    assert_eq!(s.url, "https://env.example.com/mcp");
    assert_eq!(s.auth_value.as_deref(), Some("sk-from-env"));
}

#[test]
fn accepts_authentication_token_alias() {
    let file = write_config(
        r#"
mcp_servers:
  s:
    url: https://mcp.example.com/mcp
    auth_type: bearer_token
    authentication_token: sk-alias
"#,
    );
    let config = load_config(file.path()).unwrap();
    assert_eq!(
        config.mcp_servers["s"].auth_value.as_deref(),
        Some("sk-alias")
    );
}

#[test]
fn rejects_old_list_format_with_migration_hint() {
    let file = write_config(
        r#"
mcp_servers:
  - id: linear
    url: https://mcp.linear.app/mcp
"#,
    );
    let err = load_config(file.path()).unwrap_err().to_string();
    assert!(err.contains("dict keyed by server name"), "got: {err}");
}

#[test]
fn rejects_unsupported_transport() {
    for transport in ["sse", "stdio"] {
        let file = write_config(&format!(
            "mcp_servers:\n  s:\n    url: https://mcp.example.com/mcp\n    transport: {transport}\n"
        ));
        assert!(
            load_config(file.path()).is_err(),
            "{transport} should reject"
        );
    }
}

#[test]
fn rejects_unsupported_auth_types() {
    for auth in ["oauth2", "oauth2_token_exchange", "aws_sigv4"] {
        let file = write_config(&format!(
            "mcp_servers:\n  s:\n    url: https://mcp.example.com/mcp\n    auth_type: {auth}\n    auth_value: x\n"
        ));
        assert!(load_config(file.path()).is_err(), "{auth} should reject");
    }
}

#[test]
fn rejects_auth_type_without_value() {
    let file = write_config(
        r#"
mcp_servers:
  s:
    url: https://mcp.example.com/mcp
    auth_type: bearer_token
"#,
    );
    assert!(load_config(file.path()).is_err());
}

/// Golden compatibility test: the published LiteLLM docs `mcp_servers` HTTP
/// example parses. If LiteLLM changes their schema, this breaks and tells us.
#[test]
fn parses_litellm_docs_http_example() {
    let file = write_config(
        r#"
mcp_servers:
  deepwiki_mcp:
    url: "https://mcp.deepwiki.com/mcp"
  my_http_server:
    url: "https://my-mcp-server.com/mcp"
    transport: "http"
    description: "My custom MCP server"
    auth_type: "api_key"
    auth_value: "abc123"
"#,
    );
    let config = load_config(file.path()).unwrap();
    assert_eq!(config.mcp_servers.len(), 2);
    assert_eq!(
        config.mcp_servers["my_http_server"].auth_type,
        McpAuthType::ApiKey
    );
    assert_eq!(
        config.mcp_servers["my_http_server"].description.as_deref(),
        Some("My custom MCP server")
    );
}

/// The CircleCI stdio example from LiteLLM docs — not yet supported.
#[test]
fn rejects_litellm_docs_stdio_example() {
    let file = write_config(
        r#"
mcp_servers:
  circleci_mcp:
    transport: "stdio"
    url: ""
    command: "npx"
    args: ["-y", "@circleci/mcp-server-circleci"]
"#,
    );
    assert!(load_config(file.path()).is_err());
}

// --- BYOK (bring-your-own-key) per-user MCP servers ----------------------

#[test]
fn loads_byok_mcp_server() {
    let file = write_config(
        r#"
mcp_servers:
  gmail:
    url: https://gmail-mcp.example.com/mcp
    auth_type: bearer_token
    is_byok: true
    byok_description: ["Gmail OAuth token"]
general_settings:
  master_key: sk-local
  database_url: postgres://localhost/litellm
"#,
    );

    let config = load_config(file.path()).unwrap();
    let gmail = &config.mcp_servers["gmail"];
    assert!(gmail.is_byok);
    assert_eq!(gmail.auth_type, McpAuthType::BearerToken);
    assert!(gmail.auth_value.is_none());
    assert_eq!(gmail.byok_description, vec!["Gmail OAuth token".to_owned()]);
}

#[test]
fn byok_with_shared_auth_value_is_rejected() {
    let file = write_config(
        r#"
mcp_servers:
  gmail:
    url: https://gmail-mcp.example.com/mcp
    auth_type: bearer_token
    is_byok: true
    auth_value: sk-shared
general_settings:
  master_key: sk-local
  database_url: postgres://localhost/litellm
"#,
    );
    let error = load_config(file.path()).unwrap_err().to_string();
    assert!(
        error.contains("cannot be combined with a shared auth_value"),
        "{error}"
    );
}

#[test]
fn byok_without_master_key_is_rejected() {
    let file = write_config(
        r#"
mcp_servers:
  gmail:
    url: https://gmail-mcp.example.com/mcp
    auth_type: bearer_token
    is_byok: true
general_settings:
  database_url: postgres://localhost/litellm
"#,
    );
    let error = load_config(file.path()).unwrap_err().to_string();
    assert!(
        error.contains("requires general_settings.master_key"),
        "{error}"
    );
}

#[test]
fn byok_without_database_url_is_rejected() {
    let file = write_config(
        r#"
mcp_servers:
  gmail:
    url: https://gmail-mcp.example.com/mcp
    auth_type: bearer_token
    is_byok: true
general_settings:
  master_key: sk-local
"#,
    );
    let error = load_config(file.path()).unwrap_err().to_string();
    assert!(
        error.contains("requires general_settings.database_url"),
        "{error}"
    );
}

#[test]
fn byok_with_auth_type_none_is_rejected() {
    let file = write_config(
        r#"
mcp_servers:
  gmail:
    url: https://gmail-mcp.example.com/mcp
    is_byok: true
general_settings:
  master_key: sk-local
  database_url: postgres://localhost/litellm
"#,
    );
    let error = load_config(file.path()).unwrap_err().to_string();
    assert!(error.contains("requires an auth_type"), "{error}");
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
