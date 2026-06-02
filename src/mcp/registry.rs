use std::collections::HashMap;

use reqwest::Url;

use crate::{
    errors::GatewayError,
    proxy::config::{GatewayConfig, McpServerEntry},
};

#[derive(Debug, Clone, Default)]
pub struct McpServerRegistry {
    servers: HashMap<String, McpServer>,
}

#[derive(Debug, Clone)]
pub struct McpServer {
    pub url: Url,
    pub api_key: Option<String>,
    pub headers: HashMap<String, String>,
}

impl McpServerRegistry {
    pub fn from_config(config: &GatewayConfig) -> Result<Self, GatewayError> {
        let mut servers = HashMap::with_capacity(config.mcp_servers.len());
        for entry in &config.mcp_servers {
            servers.insert(entry.id.clone(), McpServer::from_entry(entry)?);
        }
        Ok(Self { servers })
    }

    pub fn resolve(&self, server_id: &str) -> Result<&McpServer, GatewayError> {
        self.servers
            .get(server_id)
            .ok_or_else(|| GatewayError::UnknownMcpServer(server_id.to_owned()))
    }

    pub fn only_server_id(&self) -> Option<&str> {
        let mut keys = self.servers.keys();
        let first = keys.next()?;
        keys.next().is_none().then_some(first.as_str())
    }
}

impl McpServer {
    fn from_entry(entry: &McpServerEntry) -> Result<Self, GatewayError> {
        Ok(Self {
            url: entry.url.parse().map_err(|error| {
                GatewayError::InvalidConfig(format!(
                    "{} has invalid mcp_servers.url: {error}",
                    entry.id
                ))
            })?,
            api_key: entry.api_key.clone(),
            headers: entry.headers.clone(),
        })
    }
}
