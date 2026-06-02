use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::Router as AxumRouter;
use clap::{Args as ClapArgs, Parser, Subcommand};
use litellm_rust::{
    app::state::AppState,
    config::loader::load_config,
    http::routes::router,
    providers::{self, router::Router, transform::ProviderRegistry},
    telemetry::logging::init_tracing,
};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

mod cli;

#[derive(Debug, Parser)]
#[command(about = "Low-overhead LiteLLM-compatible gateway")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    serve: ServeArgs,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve(ServeArgs),
}

#[derive(Debug, Clone, ClapArgs)]
struct ServeArgs {
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

    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("claude")) {
        let claude_args = cli::parse_claude_args(std::env::args_os().skip(2))?;
        std::process::exit(cli::run_claude_wizard(claude_args)?);
    }

    let args = Args::parse();
    match args.command {
        Some(Command::Serve(serve)) => serve_gateway(serve).await,
        None => serve_gateway(args.serve).await,
    }
}

async fn serve_gateway(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config(&args.config)?;

    let mut providers = ProviderRegistry::new();
    providers::register_all(&mut providers);

    let model_router = Router::from_config(&config, &providers)?;
    let state = Arc::new(AppState::new(config, model_router)?);

    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let app: AxumRouter = router(state).layer(TraceLayer::new_for_http());
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
