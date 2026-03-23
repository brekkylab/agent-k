mod agent;
mod handlers;
mod models;
mod repository;
mod state;

use actix_web::{App, HttpServer, web};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::state::AppState;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let app_state = web::Data::new(AppState::new().await?);

    println!("server listening on http://{bind_addr}");

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
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
