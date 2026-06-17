use std::path::PathBuf;

use crate::{
    analysis::job as analysis_job, db, http::metrics::AppMetrics, upload::ChunkedUploadStore,
};

/// Shared application services passed through axum state and reused by UI
/// route dispatch.
///
/// This must stay cheap to clone because axum state extraction and the custom
/// route adapter pass cloned `AppState` values through per-request async
/// boundaries rather than sharing a mutable singleton. Fields should therefore
/// be handles or internally shared types, not large owned payloads.
#[derive(Clone)]
pub struct AppState {
    db: db::Database,
    chunked_uploads: ChunkedUploadStore,
    metrics: AppMetrics,
    run_analysis_jobs: bool,
}

impl AppState {
    pub fn new(
        db: db::Database,
        upload_bucket_path: PathBuf,
        run_analysis_jobs: bool,
    ) -> Result<Self, std::io::Error> {
        Ok(Self {
            db,
            chunked_uploads: ChunkedUploadStore::new(upload_bucket_path.join(".partial"))?,
            metrics: AppMetrics::default(),
            run_analysis_jobs,
        })
    }

    pub fn db(&self) -> &db::Database {
        &self.db
    }

    pub fn chunked_uploads(&self) -> &ChunkedUploadStore {
        &self.chunked_uploads
    }

    pub fn metrics(&self) -> &AppMetrics {
        &self.metrics
    }

    pub fn runs_analysis_jobs(&self) -> bool {
        self.run_analysis_jobs
    }

    #[cfg(test)]
    pub async fn for_test() -> Self {
        let test_database = crate::test::database::init_with_bucket_path()
            .await
            .expect("test database should initialize");

        Self::new(test_database.db, test_database.upload_bucket_path, false)
            .expect("test app state should initialize")
    }
}

pub fn spawn_analysis_job(state: AppState, analysis: db::analysis::Analysis) {
    tokio::spawn(async move {
        let provider = analysis.provider.clone();

        match analysis_job::run_analysis_job(state.db(), &analysis).await {
            Ok(()) => {
                state.metrics().increment(
                    "leo_analysis_jobs_total",
                    &[("provider", &provider), ("result", "completed")],
                );
            }
            Err(error) => {
                state.metrics().increment(
                    "leo_analysis_jobs_total",
                    &[("provider", &provider), ("result", "failed")],
                );
                if let Err(update_error) =
                    analysis_job::record_analysis_failure(state.db(), &analysis, &error).await
                {
                    eprintln!("analysis failure update failed: {update_error}");
                }
            }
        }
    });
}
