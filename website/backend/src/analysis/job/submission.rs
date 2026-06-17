//! Analysis submission validation and queued job creation.

use crate::{
    analysis::{
        self as ai_analysis,
        job::{
            AnalysisJobError, AnalysisSnapshot, AnalysisSubmission,
            events::{EventNumbers, record_analysis_event},
        },
    },
    db,
};

pub async fn queue_analysis(
    db: &db::Database,
    submission: AnalysisSubmission,
) -> Result<AnalysisSnapshot, AnalysisJobError> {
    let video_keys = validate_selected_videos(submission.video_keys)?;
    let prompt = validate_prompt(submission.prompt)?;
    let provider =
        ai_analysis::provider_from_value(submission.provider.as_deref().unwrap_or("gemini"))
            .map_err(|_| AnalysisJobError::BadRequest("unsupported analysis provider"))?;
    let frame_sample_rate_fps =
        validate_frame_sample_rate(submission.frame_sample_rate_fps.unwrap_or(0.2))?;
    let settings = ai_analysis::request::AnalysisSettings {
        frame_sample_rate_fps,
    };

    for key in &video_keys {
        if db::video::Video::find_by_file_key(db, key).await?.is_none() {
            return Err(AnalysisJobError::BadRequest("selected video was not found"));
        }
    }

    let analysis = db::analysis::Analysis::create_with_provider_and_settings(
        db, provider, settings, prompt, video_keys,
    )
    .await?;
    record_analysis_event(
        db,
        &analysis,
        "queued",
        "info",
        "analysis queued",
        EventNumbers::default(),
    )
    .await?;

    Ok(AnalysisSnapshot {
        events: db::analysis::AnalysisEvent::list_for_analysis(db, &analysis.key()).await?,
        analysis,
    })
}

pub async fn load_analysis_snapshot(
    db: &db::Database,
    analysis_id: &str,
) -> Result<Option<AnalysisSnapshot>, AnalysisJobError> {
    let Some(analysis) = db::analysis::Analysis::find(db, analysis_id).await? else {
        return Ok(None);
    };
    let events = db::analysis::AnalysisEvent::list_for_analysis(db, &analysis.key()).await?;

    Ok(Some(AnalysisSnapshot { analysis, events }))
}

fn validate_selected_videos(video_keys: Vec<String>) -> Result<Vec<String>, AnalysisJobError> {
    if video_keys.is_empty() {
        return Err(AnalysisJobError::BadRequest(
            "select at least one video to analyze",
        ));
    }

    if video_keys.len() > 10 {
        return Err(AnalysisJobError::BadRequest(
            "select no more than 10 videos to analyze",
        ));
    }

    Ok(video_keys)
}

fn validate_prompt(prompt: String) -> Result<String, AnalysisJobError> {
    let prompt = prompt.trim().to_owned();
    if prompt.is_empty() {
        return Err(AnalysisJobError::BadRequest(
            "analysis prompt cannot be empty",
        ));
    }

    Ok(prompt)
}

fn validate_frame_sample_rate(value: f64) -> Result<f64, AnalysisJobError> {
    if value.is_finite() && (0.1..=8.0).contains(&value) {
        Ok(value)
    } else {
        Err(AnalysisJobError::BadRequest(
            "unsupported frame sampling rate",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{AnalysisJobError, validate_frame_sample_rate, validate_prompt};

    #[test]
    fn prompt_validation_trims_user_prompt_before_persistence() {
        assert_eq!(
            validate_prompt("  Summarize this clip.  ".to_owned()).expect("prompt should validate"),
            "Summarize this clip."
        );
    }

    #[test]
    fn frame_sample_rate_validation_rejects_values_outside_supported_range() {
        assert!(matches!(
            validate_frame_sample_rate(20.0),
            Err(AnalysisJobError::BadRequest(
                "unsupported frame sampling rate"
            ))
        ));
    }
}
