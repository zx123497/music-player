use crate::repositories::metadata as metadata_repo;
use crate::state::AppState;
use aws_sdk_s3::presigning::PresigningConfig;

pub async fn create_presigned_url(
    track_id: i64,
    state: &AppState,
) -> Result<String, Box<dyn std::error::Error>> {
    let track = metadata_repo::get_track_by_id(&state.pg_pool, track_id).await?;

    if track.file_path.is_empty() {
        return Err("Track has no file path yet".into());
    }

    let presigning_config = PresigningConfig::builder()
        .expires_in(std::time::Duration::from_secs(3600))
        .build()?;

    let presigned_request = state
        .s3_client
        .get_object()
        .bucket(&state.config.s3.bucket)
        .key(&track.file_path)
        .presigned(presigning_config)
        .await?;

    Ok(presigned_request.uri().to_string())
}
