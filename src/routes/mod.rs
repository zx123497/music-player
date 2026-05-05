use axum::Router;
mod metadata;

pub fn create_router() -> Router {
    Router::new().nest("/metadata", metadata::create_router())
}
