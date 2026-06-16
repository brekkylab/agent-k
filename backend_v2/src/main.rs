mod auth;
mod event;
mod router;
mod state;

use std::{path::PathBuf, sync::Arc};

use aide::{
    axum::ApiRouter,
    openapi::{Info, OpenApi},
    scalar::Scalar,
};
use axum::{Extension, response::IntoResponse};
use tower_http::cors::{Any, CorsLayer};

use crate::{
    auth::{JwtConfig, bootstrap_admin_if_needed},
    state::AppState,
};

const DEFAULT_DB_PATH: &str = "sqlite://./data/agent-k.db";
const DEFAULT_JWT_EXPIRY_SECS: u64 = 60 * 60 * 24 * 7;

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

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_PATH.to_string());
    if db_url == DEFAULT_DB_PATH {
        std::fs::create_dir_all("./data")?;
    }

    let data_root = std::env::var("AGENT_K_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"));
    tokio::fs::create_dir_all(&data_root).await?;

    tracing::info!("data root: {}", data_root.display());

    let jwt_secret = std::env::var("AGENT_K_JWT_SECRET")
        .expect("AGENT_K_JWT_SECRET must be set");
    let jwt_expiry_secs = std::env::var("AGENT_K_JWT_EXPIRY_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_JWT_EXPIRY_SECS);
    let jwt = JwtConfig::new(&jwt_secret, jwt_expiry_secs);

    let app_state = Arc::new(
        AppState::new(&db_url, data_root, jwt)
            .await
            .expect("failed to initialise app state"),
    );

    bootstrap_admin_if_needed(&app_state.users).await;

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
}

async fn serve_openapi(Extension(openapi): Extension<Arc<OpenApi>>) -> impl IntoResponse {
    axum::Json(openapi.as_ref().clone())
}
