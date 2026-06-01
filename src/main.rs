use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::Router;
use clap::Parser;
use litellm_rust::{
    app::state::AppState, config::loader::load_config, http::routes::router,
    models::registry::ModelRegistry, telemetry::logging::init_tracing,
};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

#[derive(Debug, Parser)]
#[command(name = "litellm-rust")]
#[command(about = "Low-overhead LiteLLM-compatible gateway")]
struct Args {
    #[arg(long, env = "LITELLM_CONFIG", default_value = "config.yaml")]
    config: PathBuf,

    #[arg(long, env = "HOST", default_value = "127.0.0.1")]
    host: String,

    #[arg(long, env = "PORT", default_value_t = 4000)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let args = Args::parse();
    let config = load_config(&args.config)?;
    let registry = ModelRegistry::from_config(&config)?;
    let state = Arc::new(AppState::new(config, registry)?);
    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;

    let app: Router = router(state).layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "litellm-rust listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
