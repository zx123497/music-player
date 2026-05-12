use aws_sdk_s3::Client as S3Client;
use sqlx::postgres::PgPool;
use std::sync::Arc;
use tokio::sync::{
    Semaphore,
    mpsc::{Sender, channel},
};

pub struct TranscodeJob {
    pub track_id: i64,
    pub upload_id: uuid::Uuid,
    pub file_name: String,
}

#[derive(Clone)]
pub struct ThreadPool {
    pub sender: Sender<TranscodeJob>,
}

impl ThreadPool {
    pub fn new(
        worker_size: usize,
        s3_client: S3Client,
        pg_pool: PgPool,
        bucket: String,
    ) -> ThreadPool {
        let (sender, mut receiver) = channel::<TranscodeJob>(worker_size.saturating_mul(8).max(16));

        let semaphore = Arc::new(Semaphore::new(worker_size));

        tokio::spawn(async move {
            while let Some(job) = receiver.recv().await {
                let permit = semaphore.clone().acquire_owned().await.unwrap();
                let s3_client = s3_client.clone();
                let pg_pool = pg_pool.clone();
                let bucket = bucket.clone();

                tokio::spawn(async move {
                    let _permit = permit;
                    process_transcode_job(job, s3_client, pg_pool, bucket).await;
                });
            }
        });

        ThreadPool { sender }
    }
}

async fn process_transcode_job(
    job: TranscodeJob,
    s3_client: S3Client,
    pg_pool: PgPool,
    bucket: String,
) {
    sqlx::query(
        r#"
        UPDATE metadata.tracks
        SET status = 'transcoding'
        WHERE id = $1
        "#,
    )
    .bind(job.track_id)
    .execute(&pg_pool)
    .await
    .expect("Failed to update track status");

    let client = &s3_client;
    let source_key = format!("uploads/{}/{}", job.upload_id, job.file_name);
    let dest_key = format!("music/{}/{}", job.upload_id, job.file_name);

    client
        .copy_object()
        .bucket(&bucket)
        .copy_source(format!("{}/{}", bucket, source_key))
        .key(&dest_key)
        .send()
        .await
        .expect("Failed to copy object in S3");

    client
        .delete_object()
        .bucket(&bucket)
        .key(&source_key)
        .send()
        .await
        .expect("Failed to delete original object in S3");

    let new_file_path = format!("music/{}/{}", job.upload_id, job.file_name);
    sqlx::query(
        r#"
        UPDATE metadata.tracks
        SET file_path = $1, status = 'transcoded'
        WHERE id = $2
        "#,
    )
    .bind(new_file_path)
    .bind(job.track_id)
    .execute(&pg_pool)
    .await
    .expect("Failed to update track record in database");
}
