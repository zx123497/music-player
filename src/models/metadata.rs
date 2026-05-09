use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

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
    pub upload_id: uuid::Uuid,
    pub status: String,
}

#[derive(Debug, FromRow, Serialize)]
pub struct CreateTrackMetadataRequest {
    pub upload_id: Uuid,
    pub album_id: i64,
    pub artist_id: i64,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateArtistRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateUploadPresignedUrlRequest {
    pub file_name: String,
}

#[derive(Debug, Serialize)]
pub struct PresignedUrlResponse {
    pub presigned_url: String,
    pub object_key: String,
    pub upload_id: Uuid,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Deserialize)]
pub struct CreateTrackRequest {
    pub upload_id: Uuid,
    pub artist_id: i64,
    pub album_id: i64,
    pub title: Option<String>,
    pub file_name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAlbumRequest {
    pub artist_id: i64,
    pub title: String,
}
