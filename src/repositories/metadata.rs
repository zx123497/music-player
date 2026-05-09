use crate::models::metadata::{Album, Artist, CreateArtistRequest, Track};
use sqlx::PgPool;

pub async fn create_artist(
    pool: &PgPool,
    create_artist: &CreateArtistRequest,
) -> Result<Artist, sqlx::Error> {
    let artist = sqlx::query_as::<_, Artist>(
        r#"
        INSERT INTO metadata.artists (name)
        VALUES ($1)
        RETURNING id, name
        "#,
    )
    .bind(create_artist.name.clone())
    .fetch_one(pool)
    .await?;
    Ok(artist)
}

pub async fn get_all_artists(pool: &PgPool) -> Result<Vec<Artist>, sqlx::Error> {
    let artists = sqlx::query_as::<_, Artist>(
        r#"
        SELECT id, name FROM metadata.artists
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(artists)
}

pub async fn create_album(pool: &PgPool, artist_id: i64, title: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO metadata.albums (artist_id, title)
        VALUES ($1, $2)
        "#,
    )
    .bind(artist_id)
    .bind(title)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_albums_by_artist(
    pool: &PgPool,
    artist_id: i64,
) -> Result<Vec<Album>, sqlx::Error> {
    let albums = sqlx::query_as::<_, Album>(
        r#"
        SELECT id, artist_id, title FROM metadata.albums
        WHERE artist_id = $1
        "#,
    )
    .bind(artist_id)
    .fetch_all(pool)
    .await?;
    Ok(albums)
}

pub async fn create_track(
    pool: &PgPool,
    album_id: i64,
    artist_id: i64,
    title: &str,
    duration_seconds: i32,
    file_path: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO metadata.tracks (album_id, artist_id, title, duration_seconds, file_path)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(album_id)
    .bind(artist_id)
    .bind(title)
    .bind(duration_seconds)
    .bind(file_path)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_tracks_by_album(pool: &PgPool, album_id: i64) -> Result<Vec<Track>, sqlx::Error> {
    let tracks = sqlx::query_as::<_, Track>(
        r#"
        SELECT id, album_id, artist_id, title, duration_seconds, file_path FROM metadata.tracks
        WHERE album_id = $1
        "#,
    )
    .bind(album_id)
    .fetch_all(pool)
    .await?;
    Ok(tracks)
}
