use axum::Router;
use core::time;
use music_backend::{Config, create_app_state, create_router as create_api_router};
use std::sync::Arc;
use tower_http::{
    cors::CorsLayer, limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer,
};

#[tokio::main]
async fn main() {
    let timeout = time::Duration::from_secs(30);
    let status_code = axum::http::StatusCode::REQUEST_TIMEOUT;
    let app = Router::new()
        .nest("/api/v1", create_api_router())
        .layer(CorsLayer::permissive())
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)) // 10 MB limit
        .layer(TimeoutLayer::with_status_code(status_code, timeout))
        .layer(TraceLayer::new_for_http());

    let addr = format!("{}:{}", "0.0.0.0", "3000");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
