use crate::state;
use aws_sdk_s3::presigning::PresigningConfig;
use mp3_duration;
use std::io::Write;
use tempfile::NamedTempFile;
use uuid::Uuid;

pub async fn get_upload_presigned_url(
    file_name: &str,
    uuid: Uuid,
    state: &crate::state::AppState,
) -> Result<String, Box<dyn std::error::Error>> {
    // In a real implementation, this would generate a presigned URL using AWS SDK or similar
    let presigning_config = PresigningConfig::builder()
        .expires_in(std::time::Duration::from_secs(3600))
        .build()?;

    let object_key = format!("uploads/{}/{}", uuid, file_name);

    let presigning_request = state
        .s3_client
        .put_object()
        .bucket("soundzone")
        .key(object_key)
        .presigned(presigning_config)
        .await?;
    Ok(presigning_request.uri().to_string())
}

pub async fn get_mp3_duration(
    state: &crate::state::AppState,
    object_key: &str,
) -> Result<i32, Box<dyn std::error::Error>> {
    // Download first 256KB to get metadata (usually enough for MP3 headers)
    let response = state
        .s3_client
        .get_object()
        .bucket(&state.config.s3.bucket)
        .key(object_key) // First 256KB
        .send()
        .await?;

    let bytes = response.body.collect().await?.into_bytes();

    // Write the full object to a temporary file for analysis
    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(&bytes)?;
    temp_file.flush()?;

    let duration = mp3_duration::from_path(temp_file.path())?;
    let duration_seconds = duration.as_secs() as i32;

    Ok(duration_seconds)
}
