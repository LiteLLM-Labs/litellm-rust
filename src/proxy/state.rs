use reqwest::Client;

use crate::{
    errors::GatewayError,
    model_prices::ModelCostMap,
    providers::router::Router,
    proxy::config::GatewayConfig,
};

#[derive(Debug)]
pub struct AppState {
    pub config: GatewayConfig,
    pub router: Router,
    pub http: Client,
    pub model_cost_map: ModelCostMap,
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
    ) -> Self {
        Self {
            config,
            router,
            http,
            model_cost_map,
        }
    }
}
