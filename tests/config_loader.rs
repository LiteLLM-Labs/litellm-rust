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
