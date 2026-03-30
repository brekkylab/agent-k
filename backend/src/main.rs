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
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let app_state = web::Data::new(AppState::new().await?);

    println!("server listening on http://{bind_addr}");

    HttpServer::new(move || {
        // TODO: 프로덕션 배포 시 allow_any_origin() → allowed_origin("https://your-domain.com")으로 변경 필요
        // 또는 리버스 프록시(Nginx/Caddy)로 같은 도메인에 배포하면 CORS 설정 자체가 불필요
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
