use crate::models::metadata::{Artist, CreateArtistRequest};
use sqlx::PgPool;

pub async fn create_artist(
    pool: &PgPool,
    create_artist: &CreateArtistRequest,
) -> Result<Artist, sqlx::Error> {
    let artist = sqlx::query_as::<_, Artist>(
        r#"
        INSERT INTO artists (name)
        VALUES ($1)
        RETURNING id, name
        "#,
    )
    .bind(create_artist.name.clone())
    .fetch_one(pool)
    .await?;
    Ok(artist)
}
