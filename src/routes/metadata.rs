use crate::models::metadata::{CreateArtistRequest, CreateUploadPresignedUrlRequest};
use crate::services::{metadata as metadata_service, transcode as transcode_service};
use crate::state::AppState;
use axum::{
    Router,
    extract::Multipart,
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
        .route("/artists", get(get_artists))
        .route("/tracks/upload", post(upload_track))
        .route("/tracks/presigned-url", post(get_presigned_url))
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

async fn get_presigned_url(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateUploadPresignedUrlRequest>,
) -> Response {
    println!("Database URL: {}", state.config.database.url);
    let uuid = Uuid::new_v4();
    match transcode_service::get_upload_presigned_url(&payload.file_name, uuid, &state).await {
        Ok(url) => {
            let response_body = serde_json::to_string(&url).unwrap();
            return Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(response_body.into())
                .unwrap();
        }
        Err(e) => {
            eprintln!("Failed to get presigned URL: {}", e);
            return Response::builder()
                .status(500)
                .body("Failed to get presigned URL".into())
                .unwrap();
        }
    }
}

async fn upload_track(State(state): State<Arc<AppState>>, mut multipart: Multipart) -> Response {
    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap_or("").to_string();
        let file_name = field.file_name().unwrap_or("").to_string();
        let content_type = field.content_type().unwrap_or("").to_string();
        let data = field.bytes().await.unwrap();

        println!(
            "Received field: name={}, file_name={}, content_type={}, size={}",
            name,
            file_name,
            content_type,
            data.len()
        );
    }

    Response::builder()
        .status(200)
        .body("Track uploaded".into())
        .unwrap()
}
