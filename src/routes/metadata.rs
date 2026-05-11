use crate::models::metadata::{
    CreateAlbumRequest, CreateArtistRequest, CreateTrackRequest, CreateUploadPresignedUrlRequest,
    PresignedUrlResponse, Track,
};
use crate::services::{metadata as metadata_service, transcode_services as transcode_service};
use crate::state::AppState;
use axum::{
    Router,
    extract::{Json, State},
    http::StatusCode,
    response::Response,
    routing::{get, post},
};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/artist", post(create_artist))
        .route("/album", post(create_album))
        .route("/artists", get(get_artists))
        .route("/tracks/presigned-url", post(get_presigned_url))
        .route("/tracks", post(create_track))
}

async fn create_artist(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateArtistRequest>,
) -> Result<Response, StatusCode> {
    println!("Creating artist: {}", payload.name);
    println!("Database URL: {}", state.config.database.url);

    match metadata_service::new_artist(&payload, &state.pg_pool).await {
        Ok(artist) => {
            let response_body = serde_json::to_string(&artist).unwrap();
            return Ok(Response::builder()
                .status(201)
                .header("Content-Type", "application/json")
                .body(response_body.into())
                .unwrap());
        }
        Err(e) => {
            eprintln!("Failed to create artist: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
}

async fn get_artists(State(state): State<Arc<AppState>>) -> Response {
    println!("Database URL: {}", state.config.database.url);

    Response::builder()
        .status(200)
        .body("List of artists".into())
        .unwrap()
}

async fn create_album(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateAlbumRequest>,
) -> Result<Response, StatusCode> {
    println!("Creating album: {}", payload.title);
    println!("Database URL: {}", state.config.database.url);

    match metadata_service::new_album(payload.artist_id, &payload.title, &state.pg_pool).await {
        Ok(album) => {
            let response_body = serde_json::to_string(&album).unwrap();
            return Ok(Response::builder()
                .status(201)
                .header("Content-Type", "application/json")
                .body(response_body.into())
                .unwrap());
        }
        Err(e) => {
            eprintln!("Failed to create album: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
}

async fn get_presigned_url(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateUploadPresignedUrlRequest>,
) -> Result<Json<PresignedUrlResponse>, StatusCode> {
    println!("Database URL: {}", state.config.database.url);
    let upload_id = Uuid::new_v4();
    let object_key = format!("uploads/{}/{}", upload_id, payload.file_name);

    match transcode_service::get_upload_presigned_url(&payload.file_name, upload_id, &state).await {
        Ok(presigned_url) => {
            let response = PresignedUrlResponse {
                presigned_url,
                object_key,
                upload_id,
                expires_in_seconds: 3600,
            };
            Ok(Json(response))
        }
        Err(e) => {
            eprintln!("Failed to get presigned URL: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn create_track(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateTrackRequest>,
) -> Result<Json<Track>, StatusCode> {
    // Generate object key using upload_id and file_name
    let object_key = format!("uploads/{}/{}", payload.upload_id, payload.file_name);

    // Check if object exists
    state
        .s3_client
        .head_object()
        .bucket(&state.config.s3.bucket)
        .key(&object_key)
        .send()
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    // Create track record with extracted duration
    let track = metadata_service::create_track(
        payload.album_id,
        payload.artist_id,
        &payload
            .title
            .unwrap_or_else(|| "Untitled Track".to_string()),
        &payload.file_name,
        payload.upload_id,
        &state,
    )
    .await
    .map_err(|e| {
        eprintln!("Failed to create track: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(track))
}
