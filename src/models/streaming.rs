use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamTrackPresignUrlResponse {
    pub presigned_url: String,
    pub expires_in_seconds: u64,
}
