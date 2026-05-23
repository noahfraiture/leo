use crate::{analysis::job as analysis_job, db, http::router::AppState};

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
