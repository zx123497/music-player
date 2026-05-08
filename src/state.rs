use crate::config::Config;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::config::{Builder, Credentials, Region};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub s3_client: S3Client,
    pub pg_pool: PgPool,
}

pub async fn create_app_state(config: Config) -> Arc<AppState> {
    let s3_client = create_s3_client(&config.s3).await;

    let database_url = config.database.url.clone();
    let pg_pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("Failed to create PostgreSQL connection pool");

    Arc::new(AppState {
        config,
        s3_client,
        pg_pool,
    })
}

async fn create_s3_client(config: &crate::config::S3Config) -> S3Client {
    let credentials = Credentials::new(
        &config.access_key,
        &config.secret_key,
        None,
        None,
        "music-backend",
    );
    let builder = Builder::new()
        .endpoint_url(config.endpoint.clone())
        .credentials_provider(credentials)
        .region(Region::new(config.region.clone()))
        .force_path_style(config.use_path_style);

    S3Client::from_conf(builder.build())
}
