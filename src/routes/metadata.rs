use axum::{Router, extract::Path, response::Response, routing::get};

pub fn create_router() -> Router {
    Router::new().route("/{music_id}", get(get_metadata))
}

pub struct Artist {
    pub id: i64,
    pub name: String,
}

pub struct Album {
    pub id: i64,
    pub artist_id: i64,
    pub title: String,
}

pub struct Track {
    pub id: i64,
    pub album_id: i64,
    pub title: String,
    pub duration_seconds: i32,
}

pub struct TrackFullMetadata {
    pub title: String,
    pub artist_name: String,
    pub album_name: String,
    pub duration: i32,
}

async fn get_metadata(Path(music_id): Path<String>) -> Response {
    Response::builder()
        .status(200)
        .body(format!("Metadata for music ID: {}", music_id).into())
        .unwrap()
}
