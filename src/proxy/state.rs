use reqwest::Client;

use crate::{
    agents::runs::AgentRunStore, errors::GatewayError, mcp::registry::McpServerRegistry,
    model_prices::ModelCostMap, proxy::config::GatewayConfig, sdk::router::Router,
};

#[derive(Debug)]
pub struct AppState {
    pub config: GatewayConfig,
    pub router: Router,
    pub mcp_servers: McpServerRegistry,
    pub http: Client,
    pub model_cost_map: ModelCostMap,
    pub agent_runs: AgentRunStore,
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
    ) -> Result<Self, GatewayError> {
        Ok(Self {
            mcp_servers: McpServerRegistry::from_config(&config)?,
            config,
            router,
            http,
            model_cost_map,
            agent_runs: AgentRunStore::default(),
        })
    }
}
