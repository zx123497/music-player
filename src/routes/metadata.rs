use axum::{Router, extract::Path, response::Response, routing::get};

pub fn create_router() -> Router {
    Router::new().route("/{music_id}", get(get_metadata))
}

async fn get_metadata(Path(music_id): Path<String>) -> Response {
    Response::builder()
        .status(200)
        .body(format!("Metadata for music ID: {}", music_id).into())
        .unwrap()
}
