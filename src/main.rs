use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::Router as AxumRouter;
use clap::{Args as ClapArgs, Parser, Subcommand};
use litellm_rust::{
    http::routes::router,
    model_prices,
    proxy::{config::load_config, state::AppState},
    sdk::{
        providers::{self, transform::ProviderRegistry},
        router::Router,
    },
};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

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
    Logout,
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
    match std::env::args_os().nth(1).as_deref() {
        Some(arg) if arg == std::ffi::OsStr::new("claude") => {
            let claude_args = cli::parse_claude_args(std::env::args_os().skip(2))?;
            std::process::exit(cli::run_claude_wizard(claude_args)?);
        }
        None => {
            std::process::exit(cli::run_tool_selector()?);
        }
        _ => {}
    }

    let args = Args::parse();
    match args.command {
        Some(Command::Serve(serve)) => serve_gateway(serve).await,
        Some(Command::Logout) => cli::logout(),
        None => serve_gateway(args.serve).await,
    }
}

async fn serve_gateway(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let config = load_config(&args.config)?;

    let mut providers = ProviderRegistry::new();
    providers::register_all(&mut providers);

    let model_router = Router::from_config(&config, &providers)?;

    let http = AppState::build_http_client()?;
    let model_cost_map = model_prices::load(&http).await;

    let state = Arc::new(AppState::new(
        config.clone(),
        model_router,
        http,
        model_cost_map,
    ));

    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let app: AxumRouter = router(state).layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(addr).await?;

    println!("\nLiteLLM: Proxy initialized with Config, Set models:");
    for entry in &config.model_list {
        println!("  {}", entry.model_name);
    }
    if let Some(key) = &config.general_settings.master_key {
        let hint = if key.len() > 8 { &key[..8] } else { key };
        println!("LiteLLM: Set Master Key: {}****", hint);
    }
    println!("INFO:     Application startup complete.");
    println!(
        "INFO:     Uvicorn running on http://{} (Press CTRL+C to quit)",
        addr
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    println!("\nINFO:     Shutting down LiteLLM Proxy Server");
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
