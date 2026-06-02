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
    pub fn new(
        config: GatewayConfig,
        router: Router,
        model_cost_map: ModelCostMap,
    ) -> Result<Self, GatewayError> {
        let http = Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_nodelay(true)
            .http2_adaptive_window(true)
            .build()
            .map_err(GatewayError::HttpClient)?;

        Ok(Self {
            config,
            router,
            http,
            model_cost_map,
        })
    }
}
