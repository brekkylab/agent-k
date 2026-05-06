use std::path::PathBuf;
use std::sync::Arc;

use agent_k_backend::{repository, router, state::AppState};
use aide::axum::ApiRouter;
use aide::openapi::{Info, OpenApi};
use aide::scalar::Scalar;
use ailoy::agent::default_provider_mut;
use ailoy::lang_model::LangModelProvider;
use axum::Extension;
use axum::response::IntoResponse;
use speedwagon::{Store, build_tools};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

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

    let repo = repository::create_repository_from_env()
        .await
        .expect("failed to initialise repository");

    let store_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".speedwagon");
    let store = Arc::new(RwLock::new(
        Store::new(&store_path).expect("speedwagon store init"),
    ));

    // Register API keys with the global provider (needed by Agent::try_with_tools)
    {
        let mut provider = default_provider_mut().await;

        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            provider
                .models
                .insert("openai/*".into(), LangModelProvider::openai(key));
        }
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            provider
                .models
                .insert("anthropic/*".into(), LangModelProvider::anthropic(key));
        }
        if let Ok(key) = std::env::var("GEMINI_API_KEY") {
            provider
                .models
                .insert("google/*".into(), LangModelProvider::gemini(key));
        }

        provider.tools = build_tools(store.clone());
    }

    let app_state = Arc::new(AppState::new(repo, store));
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
