use crate::models::streaming::StreamTrackPresignUrlResponse;
use crate::services::streaming as streaming_service;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Json, Router};
use std::sync::Arc;

pub fn create_router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/tracks/{track_id}/presigned-url",
        axum::routing::get(stream_track_presignurl),
    )
}

async fn stream_track_presignurl(
    state: State<Arc<AppState>>,
    Path(track_id): Path<i64>,
) -> Result<Json<StreamTrackPresignUrlResponse>, StatusCode> {
    println!("Generating presigned URL for track ID: {}", track_id);
    println!("S3 Bucket: {}", state.config.s3.bucket);

    match streaming_service::create_presigned_url(track_id, &state).await {
        Ok(presigned_url) => Ok(Json(StreamTrackPresignUrlResponse {
            presigned_url,
            expires_in_seconds: 3600,
        })),
        Err(e) => {
            eprintln!("Failed to generate presigned URL: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
