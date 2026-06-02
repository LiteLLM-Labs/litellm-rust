use reqwest::Client;
use sqlx::PgPool;

use crate::{
    db::managed_agents::mcp_credentials::crypto, errors::GatewayError,
    mcp::registry::McpServerRegistry, model_prices::ModelCostMap, proxy::config::GatewayConfig,
    sdk::router::Router,
};

#[derive(Debug)]
pub struct AppState {
    pub config: GatewayConfig,
    pub router: Router,
    pub mcp_servers: McpServerRegistry,
    pub http: Client,
    pub model_cost_map: ModelCostMap,
    pub db: Option<PgPool>,
    /// AES key for encrypting user MCP credentials, derived from the master key.
    /// `None` when no master key is configured.
    pub enc_key: Option<[u8; 32]>,
}

impl AppState {
    pub fn build_http_client() -> Result<Client, GatewayError> {
        Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_nodelay(true)
            .http2_adaptive_window(true)
            .build()
            .map_err(GatewayError::HttpClient)
    }

    pub fn new(
        config: GatewayConfig,
        router: Router,
        http: Client,
        model_cost_map: ModelCostMap,
        db: Option<PgPool>,
    ) -> Result<Self, GatewayError> {
        let enc_key = config
            .general_settings
            .master_key
            .as_deref()
            .map(crypto::derive_key);
        Ok(Self {
            mcp_servers: McpServerRegistry::from_config(&config)?,
            config,
            router,
            http,
            model_cost_map,
            db,
            enc_key,
        })
    }
}
