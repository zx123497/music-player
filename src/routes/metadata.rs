use crate::models::metadata::CreateArtistRequest;
use crate::services::metadata as metadata_service;
use crate::state::AppState;
use axum::{
    Router,
    extract::{Json, State},
    http::StatusCode,
    response::Response,
    routing::{get, post},
};
use std::sync::Arc;

pub fn create_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/artist", post(create_artist))
        .route("/artists", get(get_artists))
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
