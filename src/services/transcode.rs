use crate::state;
use aws_sdk_s3::presigning::PresigningConfig;
use mp3_metadata;
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
        .key(object_key)
        .range("bytes=0-262143") // First 256KB
        .send()
        .await?;

    let bytes = response.body.collect().await?.into_bytes();

    // Parse MP3 metadata
    let metadata = mp3_metadata::read_from_slice(&bytes)
        .map_err(|e| format!("Failed to parse MP3 metadata: {}", e))?;

    let duration_ms = metadata.duration.as_millis() as i32;
    let duration_seconds = duration_ms / 1000;

    Ok(duration_seconds)
}
