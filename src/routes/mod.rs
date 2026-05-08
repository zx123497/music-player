use crate::state::AppState;
use axum::Router;
use std::sync::Arc;

mod metadata;

pub fn create_router() -> Router<Arc<AppState>> {
    Router::new().nest("/metadata", metadata::create_router())
}
