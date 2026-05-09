use crate::state;
use aws_sdk_s3::presigning::PresigningConfig;
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
