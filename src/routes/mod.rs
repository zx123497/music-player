use crate::state::AppState;
use axum::Router;
use std::sync::Arc;

mod metadata;
mod streaming;

pub fn create_router() -> Router<Arc<AppState>> {
    Router::new()
        .nest("/metadata", metadata::create_router())
        .nest("/streaming", streaming::create_router())
}
