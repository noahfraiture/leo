//! Failure diagnostic mapping for persisted analysis jobs.

use serde_json::json;

use crate::{
    analysis::{
        error::AnalysisError as AiAnalysisError,
        gemini::GeminiError,
        job::{
            AnalysisJobError,
            events::{EventNumbers, record_analysis_event},
        },
        openai::OpenAiError,
        request::AnalysisTelemetry,
    },
    db,
};

pub async fn record_analysis_failure(
    db: &db::Database,
    analysis: &db::analysis::Analysis,
    error: &AnalysisJobError,
) -> Result<db::analysis::AnalysisFailureDiagnostic, AnalysisJobError> {
    let diagnostic = failure_diagnostic(&analysis.provider, error);
    eprintln!(
        "{}",
        AnalysisTelemetry::new(analysis.key(), analysis.provider.clone()).event_json(
            "error",
            "analysis_job",
            "failed",
            [
                ("stage", json!(&diagnostic.stage)),
                ("kind", json!(&diagnostic.kind)),
                ("message", json!(&diagnostic.message)),
            ],
        )
    );
    analysis
        .fail_with_diagnostic(db, diagnostic.clone())
        .await?;
    record_analysis_event(
        db,
        analysis,
        &diagnostic.stage,
        "error",
        &diagnostic.message,
        EventNumbers {
            attempt: diagnostic.attempt,
            attempts: diagnostic.attempts,
            payload_bytes: diagnostic.payload_bytes,
            offset_bytes: None,
            size_bytes: None,
            duration_ms: None,
        },
    )
    .await?;

    Ok(diagnostic)
}

fn failure_diagnostic(
    provider: &str,
    error: &AnalysisJobError,
) -> db::analysis::AnalysisFailureDiagnostic {
    match error {
        AnalysisJobError::AiAnalysis(AiAnalysisError::OpenAi(OpenAiError::Request {
            stage,
            attempt,
            attempts,
            payload_bytes,
            timeout,
            connect,
            body,
            request,
            ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: format!("openai.{stage}"),
            kind: request_failure_kind(*timeout, *connect, *body, *request).to_owned(),
            retryable: true,
            attempt: Some(*attempt as i64),
            attempts: Some(*attempts as i64),
            payload_bytes: Some(*payload_bytes as i64),
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::OpenAi(OpenAiError::Api {
            status, ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: "openai.api".to_owned(),
            kind: format!("http_{}", status.as_u16()),
            retryable: status.is_server_error(),
            attempt: None,
            attempts: None,
            payload_bytes: None,
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::Gemini(GeminiError::UploadRequest {
            offset,
            bytes,
            timeout,
            connect,
            body,
            ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: "gemini.upload".to_owned(),
            kind: request_failure_kind(*timeout, *connect, *body, false).to_owned(),
            retryable: true,
            attempt: None,
            attempts: None,
            payload_bytes: Some(*bytes as i64),
            message: format!("{error} offset={offset}"),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::Gemini(
            GeminiError::UploadFinalizationUnknown { attempts, .. },
        )) => db::analysis::AnalysisFailureDiagnostic {
            stage: "gemini.upload_finalize".to_owned(),
            kind: "lost_final_response".to_owned(),
            retryable: true,
            attempt: Some(*attempts as i64),
            attempts: Some(*attempts as i64),
            payload_bytes: None,
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::Gemini(GeminiError::Api {
            status, ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: "gemini.api".to_owned(),
            kind: format!("http_{}", status.as_u16()),
            retryable: status.is_server_error(),
            attempt: None,
            attempts: None,
            payload_bytes: None,
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::FrameExtraction(_)) => {
            db::analysis::AnalysisFailureDiagnostic {
                stage: "frame_extraction".to_owned(),
                kind: "ffmpeg".to_owned(),
                retryable: false,
                attempt: None,
                attempts: None,
                payload_bytes: None,
                message: error.to_string(),
            }
        }
        AnalysisJobError::BadRequest(_) => db::analysis::AnalysisFailureDiagnostic {
            stage: "input_validation".to_owned(),
            kind: "bad_request".to_owned(),
            retryable: false,
            attempt: None,
            attempts: None,
            payload_bytes: None,
            message: error.to_string(),
        },
        _ => db::analysis::AnalysisFailureDiagnostic {
            stage: provider.to_owned(),
            kind: "internal".to_owned(),
            retryable: false,
            attempt: None,
            attempts: None,
            payload_bytes: None,
            message: error.to_string(),
        },
    }
}

fn request_failure_kind(timeout: bool, connect: bool, body: bool, request: bool) -> &'static str {
    if timeout {
        "timeout"
    } else if connect {
        "connect"
    } else if body {
        "body"
    } else if request {
        "request"
    } else {
        "unknown"
    }
}
