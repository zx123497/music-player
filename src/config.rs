use figment::{
    Figment,
    providers::{Env, Format, Toml},
};

use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub s3: S3Config,
    pub jwt: JwtConfig,
    pub transcode: TranscodeConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RedisConfig {
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    pub use_path_style: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TranscodeConfig {
    pub worker_size: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct JwtConfig {
    pub secret: String,
    pub expiration_seconds: u64,
}

impl Config {
    pub fn load(path: &str) -> Self {
        Figment::new()
            .merge(Toml::file(path))
            .merge(Env::prefixed("APP_"))
            .extract()
            .expect("Failed to load configuration")
    }
}
