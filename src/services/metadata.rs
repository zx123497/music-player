use crate::models::metadata::{Album, Artist, CreateArtistRequest};
use crate::repositories::metadata as metadata_repo;

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
    duration_seconds: i32,
    file_path: &str,
    pool: &sqlx::PgPool,
) -> Result<(), sqlx::Error> {
    metadata_repo::create_track(
        pool,
        album_id,
        artist_id,
        title,
        duration_seconds,
        file_path,
    )
    .await
}
