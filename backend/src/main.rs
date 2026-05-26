mod cli;

use std::{path::PathBuf, sync::Arc};

use agent_k_backend::{auth, ingest_worker, repository, router, state::AppState, worker};
use aide::{
    axum::ApiRouter,
    openapi::{Info, OpenApi},
    scalar::Scalar,
};
use axum::{Extension, response::IntoResponse};
use clap::Parser;
use tower_http::cors::{Any, CorsLayer};

use crate::cli::{Cli, Command, ServeArgs, ServeMode};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LOG_LEVEL")
                .or_else(|_| tracing_subscriber::EnvFilter::try_from_default_env())
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Command::CreateAdmin {
            username,
            password,
            display_name,
        }) => {
            cli::run_create_admin(username, password, display_name).await;
            return Ok(());
        }
        Some(Command::Serve(args)) => {
            run_server(args.mode).await?;
        }
        None => {
            run_server(ServeArgs::default().mode).await?;
        }
    }

    Ok(())
}

async fn run_server(mode: ServeMode) -> std::io::Result<()> {
    let jwt_secret = std::env::var("AGENT_K_JWT_SECRET").unwrap_or_else(|_| {
        tracing::warn!("AGENT_K_JWT_SECRET not set — using insecure fallback secret");
        "jwtsecret".to_string()
    });
    let jwt_expiry = std::env::var("JWT_EXPIRY_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(604_800); // 7 days
    let jwt = auth::JwtConfig::new(&jwt_secret, jwt_expiry);

    let repo = repository::create_repository_from_env()
        .await
        .expect("failed to initialise repository");

    // Admin bootstrap only meaningful when the API is exposed in this process.
    if mode.runs_api() {
        auth::bootstrap_admin_if_needed(&repo).await;
    }

    // Per-project Speedwagon stores live under data_root/projects/{id}/.speedwagon
    // and are created lazily by AppState::get_store. Tools are no longer
    // registered against the process-wide default provider — build_agent
    // constructs a per-call provider so each session sees only its project's
    // corpus. ailoy's default ToolProvider still supplies the built-in
    // web_search/python/bash factories.

    let data_root = std::env::var("AGENT_K_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"));

    tracing::info!("data root: {}", data_root.display());
    tracing::info!("serve mode: {:?}", mode);

    let app_state = Arc::new(AppState::new(repo, jwt, data_root));

    if mode.runs_worker() {
        let worker_count = std::env::var("AGENT_K_WORKER_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2usize);
        worker::spawn_workers(app_state.clone(), worker_count);
        worker::spawn_housekeeper(app_state.clone());
        worker::spawn_cron_ticker(app_state.clone());

        let ingest_worker_count = std::env::var("AGENT_K_INGEST_WORKER_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1usize);
        ingest_worker::spawn_ingest_workers(app_state.clone(), ingest_worker_count);
        ingest_worker::spawn_ingest_housekeeper(app_state.clone());
    }

    if mode.runs_api() {
        let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

        aide::generate::on_error(|error| {
            tracing::warn!("aide schema error: {error}");
        });
        aide::generate::extract_schemas(true);

        let mut openapi = OpenApi {
            info: Info {
                title: "Agent-K API".to_string(),
                version: "0.1.0".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        let app = router::get_router(app_state)
            .finish_api(&mut openapi)
            .merge(
                ApiRouter::new()
                    .route("/api-docs/openapi.json", axum::routing::get(serve_openapi))
                    .route(
                        "/docs",
                        axum::routing::get(Scalar::new("/api-docs/openapi.json").axum_handler()),
                    ),
            )
            .layer(Extension(Arc::new(openapi)))
            .layer(cors);

        tracing::info!("server listening on http://{bind_addr}");
        tracing::info!("API docs: http://{bind_addr}/docs");

        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
        axum::serve(listener, app).await
    } else {
        // Worker-only: nothing to bind, so block on SIGINT/SIGTERM equivalent.
        // Background workers keep running on tokio runtime tasks; this just
        // keeps the runtime alive until the operator stops the process.
        tracing::info!("worker-only mode: awaiting Ctrl+C to shut down");
        tokio::signal::ctrl_c().await?;
        tracing::info!("shutdown signal received");
        Ok(())
    }
}

async fn serve_openapi(Extension(openapi): Extension<Arc<OpenApi>>) -> impl IntoResponse {
    axum::Json(openapi.as_ref().clone())
}
