use reqwest::Client;

use crate::{
    app::errors::GatewayError, config::schema::GatewayConfig, providers::router::Router,
};

#[derive(Debug)]
pub struct AppState {
    pub config: GatewayConfig,
    pub router: Router,
    pub http: Client,
}

impl AppState {
    pub fn new(config: GatewayConfig, router: Router) -> Result<Self, GatewayError> {
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
        })
    }
}
