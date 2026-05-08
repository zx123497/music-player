use crate::models::metadata::{Artist, CreateArtistRequest};
use crate::repositories::metadata as metadata_repo;

pub async fn new_artist(
    create_artist: &CreateArtistRequest,
    pool: &sqlx::PgPool,
) -> Result<Artist, sqlx::Error> {
    let artist = metadata_repo::create_artist(pool, create_artist).await?;
    Ok(artist)
}
