use crate::models::metadata::{Album, Artist, CreateArtistRequest, Track};
use crate::repositories::metadata as metadata_repo;
use crate::services::transcode_services as transcode_service;
use crate::state;

pub async fn new_artist(
    create_artist: &CreateArtistRequest,
    pool: &sqlx::PgPool,
) -> Result<Artist, sqlx::Error> {
    let artist = metadata_repo::create_artist(pool, create_artist).await?;
    Ok(artist)
}

pub async fn get_all_artists(pool: &sqlx::PgPool) -> Result<Vec<Artist>, sqlx::Error> {
    let artists = metadata_repo::get_all_artists(pool).await?;
    Ok(artists)
}

pub async fn new_album(
    artist_id: i64,
    title: &str,
    pool: &sqlx::PgPool,
) -> Result<(), sqlx::Error> {
    metadata_repo::create_album(pool, artist_id, title).await
}

pub async fn get_albums_by_artist(
    artist_id: i64,
    pool: &sqlx::PgPool,
) -> Result<Vec<Album>, sqlx::Error> {
    let albums = metadata_repo::get_albums_by_artist(pool, artist_id).await?;
    Ok(albums)
}

pub async fn create_track(
    album_id: i64,
    artist_id: i64,
    title: &str,
    file_name: &str,
    upload_id: uuid::Uuid,
    state: &state::AppState,
) -> Result<Track, Box<dyn std::error::Error>> {
    let object_key = format!("uploads/{}/{}", upload_id, file_name);
    // check if upload_id exists in S3 and is valid before creating track metadata
    let head_object_output = state
        .s3_client
        .head_object()
        .bucket("soundzone")
        .key(&object_key)
        .send()
        .await;
    if head_object_output.is_err() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Uploaded file not found in S3",
        )));
    }

    let duration = transcode_service::get_mp3_duration(&state, &object_key).await?;

    let track = metadata_repo::create_track(
        &state.pg_pool,
        artist_id,
        album_id,
        title,
        "",
        duration,
        upload_id,
    )
    .await?;
    // need to trigger transcode job after creating track metadata
    let job = crate::services::transcode::queue::TranscodeJob {
        track_id: track.id,
        upload_id,
        file_name: file_name.to_string(),
    };
    state
        .transcode_thread_pool
        .sender
        .send(job)
        .await
        .expect("Failed to send transcode job to thread pool");
    Ok(track)
}
