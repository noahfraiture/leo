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
        mistral::MistralError,
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
        AnalysisJobError::AiAnalysis(AiAnalysisError::Mistral(MistralError::Request {
            stage,
            attempt,
            attempts,
            payload_bytes,
            timeout,
            connect,
            body,
            request,
            decode,
            ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: format!("mistral.{stage}"),
            kind: mistral_request_failure_kind(*timeout, *connect, *body, *request, *decode)
                .to_owned(),
            retryable: *timeout || *connect || *body || *request,
            attempt: Some(*attempt as i64),
            attempts: Some(*attempts as i64),
            payload_bytes: Some(*payload_bytes as i64),
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::Mistral(MistralError::Api {
            status,
            ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: "mistral.api".to_owned(),
            kind: format!("http_{}", status.as_u16()),
            retryable: matches!(
                *status,
                reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::TOO_MANY_REQUESTS
            ) || status.is_server_error(),
            attempt: None,
            attempts: None,
            payload_bytes: None,
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::Mistral(MistralError::FrameTooLarge {
            actual_bytes,
            ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: "mistral.frame_validation".to_owned(),
            kind: "image_too_large".to_owned(),
            retryable: false,
            attempt: None,
            attempts: None,
            payload_bytes: Some(*actual_bytes as i64),
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::Mistral(MistralError::FrameExtraction(
            _,
        ))) => db::analysis::AnalysisFailureDiagnostic {
            stage: "frame_extraction".to_owned(),
            kind: "ffmpeg".to_owned(),
            retryable: false,
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

fn mistral_request_failure_kind(
    timeout: bool,
    connect: bool,
    body: bool,
    request: bool,
    decode: bool,
) -> &'static str {
    if timeout || connect || body || request {
        request_failure_kind(timeout, connect, body, request)
    } else if decode {
        "decode"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use super::failure_diagnostic;
    use crate::{
        analysis::{
            error::AnalysisError as AiAnalysisError, job::AnalysisJobError, mistral::MistralError,
        },
        media::frames::FrameExtractionError,
    };

    fn request_error() -> reqwest::Error {
        reqwest::Client::new()
            .get("not a url")
            .build()
            .expect_err("relative URL should be rejected")
    }

    #[test]
    fn failure_diagnostic_maps_mistral_request_failures() {
        let diagnostic = failure_diagnostic(
            "mistral",
            &AnalysisJobError::AiAnalysis(AiAnalysisError::Mistral(MistralError::Request {
                stage: "chunk 1/2".to_owned(),
                attempt: 2,
                attempts: 3,
                payload_bytes: 456,
                timeout: false,
                connect: true,
                body: false,
                request: false,
                decode: false,
                chain: "synthetic transport failure".to_owned(),
                source: request_error(),
            })),
        );

        assert_eq!(diagnostic.stage, "mistral.chunk 1/2");
        assert_eq!(diagnostic.kind, "connect");
        assert!(diagnostic.retryable);
        assert_eq!(diagnostic.attempt, Some(2));
        assert_eq!(diagnostic.attempts, Some(3));
        assert_eq!(diagnostic.payload_bytes, Some(456));
        assert!(diagnostic.message.contains("Mistral request failed"));
    }

    #[test]
    fn failure_diagnostic_maps_mistral_decode_failures_as_non_retryable() {
        let diagnostic = failure_diagnostic(
            "mistral",
            &AnalysisJobError::AiAnalysis(AiAnalysisError::Mistral(MistralError::Request {
                stage: "summary".to_owned(),
                attempt: 1,
                attempts: 3,
                payload_bytes: 789,
                timeout: false,
                connect: false,
                body: false,
                request: false,
                decode: true,
                chain: "synthetic decode failure".to_owned(),
                source: request_error(),
            })),
        );

        assert_eq!(diagnostic.stage, "mistral.summary");
        assert_eq!(diagnostic.kind, "decode");
        assert!(!diagnostic.retryable);
        assert_eq!(diagnostic.attempt, Some(1));
        assert_eq!(diagnostic.attempts, Some(3));
        assert_eq!(diagnostic.payload_bytes, Some(789));
        assert!(diagnostic.message.contains("decode=true"));
    }

    #[test]
    fn failure_diagnostic_maps_mistral_api_failures_and_retryability() {
        for (status, retryable) in [
            (StatusCode::REQUEST_TIMEOUT, true),
            (StatusCode::TOO_MANY_REQUESTS, true),
            (StatusCode::BAD_GATEWAY, true),
            (StatusCode::BAD_REQUEST, false),
        ] {
            let diagnostic = failure_diagnostic(
                "mistral",
                &AnalysisJobError::AiAnalysis(AiAnalysisError::Mistral(MistralError::Api {
                    status,
                    body: "upstream failed".to_owned(),
                })),
            );

            assert_eq!(diagnostic.stage, "mistral.api");
            assert_eq!(diagnostic.kind, format!("http_{}", status.as_u16()));
            assert_eq!(diagnostic.retryable, retryable);
            assert_eq!(
                diagnostic.message,
                format!("Mistral API returned {status}: upstream failed")
            );
        }
    }

    #[test]
    fn failure_diagnostic_maps_mistral_oversized_frames() {
        let diagnostic = failure_diagnostic(
            "mistral",
            &AnalysisJobError::AiAnalysis(AiAnalysisError::Mistral(MistralError::FrameTooLarge {
                video_name: "sample.mp4".to_owned(),
                timestamp_secs: 1.25,
                actual_bytes: 12_000_000,
                limit_bytes: 10_000_000,
            })),
        );

        assert_eq!(diagnostic.stage, "mistral.frame_validation");
        assert_eq!(diagnostic.kind, "image_too_large");
        assert!(!diagnostic.retryable);
        assert_eq!(diagnostic.attempt, None);
        assert_eq!(diagnostic.attempts, None);
        assert_eq!(diagnostic.payload_bytes, Some(12_000_000));
        assert_eq!(
            diagnostic.message,
            "Mistral frame from sample.mp4 at 1.250s is 12000000 bytes; limit is 10000000 bytes"
        );
    }

    #[test]
    fn failure_diagnostic_maps_nested_mistral_frame_extraction_failures() {
        let diagnostic = failure_diagnostic(
            "mistral",
            &AnalysisJobError::AiAnalysis(AiAnalysisError::Mistral(MistralError::FrameExtraction(
                FrameExtractionError::CommandFailed {
                    name: "sample.mp4".to_owned(),
                    stderr: "decoder failed".to_owned(),
                },
            ))),
        );

        assert_eq!(diagnostic.stage, "frame_extraction");
        assert_eq!(diagnostic.kind, "ffmpeg");
        assert!(!diagnostic.retryable);
        assert_eq!(diagnostic.attempt, None);
        assert_eq!(diagnostic.attempts, None);
        assert_eq!(diagnostic.payload_bytes, None);
        assert_eq!(
            diagnostic.message,
            "ffmpeg failed for sample.mp4: decoder failed"
        );
    }
}
