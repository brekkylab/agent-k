mod agent;
mod handlers;
mod models;
mod repository;
mod services;
mod state;

use actix_cors::Cors;
use actix_web::{App, HttpServer, web};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::state::AppState;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LOG_LEVEL")
                .or_else(|_| tracing_subscriber::EnvFilter::try_from_default_env())
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let app_state = web::Data::new(AppState::new().await?);

    tracing::info!("server listening on http://{bind_addr}");

    HttpServer::new(move || {
        // TODO: Replace allow_any_origin() with allowed_origin("https://your-domain.com") for production
        // Alternatively, deploying behind a reverse proxy (Nginx/Caddy) on the same domain eliminates the need for CORS
        let cors = Cors::default()
            .allow_any_origin()
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE"])
            .allowed_headers(vec!["Content-Type"])
            .max_age(3600);

        App::new()
            .wrap(cors)
            .app_data(app_state.clone())
            .app_data(web::PayloadConfig::default().limit(50 * 1024 * 1024))
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}")
                    .url("/api-docs/openapi.json", handlers::ApiDoc::openapi()),
            )
            .configure(handlers::configure)
    })
    .bind(&bind_addr)?
    .run()
    .await
}
