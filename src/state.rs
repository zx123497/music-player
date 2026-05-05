use crate::config::Config;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::config::{Builder, Credentials, Region};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub s3_client: S3Client,
}

pub async fn create_app_state(config: Config) -> Arc<AppState> {
    let s3_client = create_s3_client(&config.s3).await;
    Arc::new(AppState { config, s3_client })
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
