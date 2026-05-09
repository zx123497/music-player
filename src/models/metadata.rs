use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, FromRow, Serialize)]
pub struct Artist {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, FromRow, Serialize)]
pub struct Album {
    pub id: i64,
    pub artist_id: i64,
    pub title: String,
}

#[derive(Debug, FromRow, Serialize)]
pub struct Track {
    pub id: i64,
    pub album_id: i64,
    pub artist_id: i64,
    pub title: String,
    pub duration_seconds: i32,
    pub file_path: String,
}

#[derive(Debug, FromRow, Serialize)]
pub struct TrackFullMetadata {
    pub title: String,
    pub artist_name: String,
    pub album_name: String,
    pub duration: i32,
}

#[derive(Debug, Deserialize)]
pub struct CreateArtistRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateUploadPresignedUrlRequest {
    pub file_name: String,
}
