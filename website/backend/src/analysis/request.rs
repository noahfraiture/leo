use serde_json::{Map, Value, json};

pub const DEFAULT_FRAME_SAMPLE_RATE_FPS: f64 = 0.2;

/// Provider-agnostic analysis input built by the background job.
///
/// Providers decide whether to upload the original video bytes directly or to
/// convert them into sampled frames before calling their model API.
pub struct AnalysisRequest {
    pub videos: Vec<AnalysisVideo>,
    pub prompt: String,
    pub settings: AnalysisSettings,
    pub telemetry: AnalysisTelemetry,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisVideo {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnalysisSettings {
    pub frame_sample_rate_fps: f64,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            frame_sample_rate_fps: DEFAULT_FRAME_SAMPLE_RATE_FPS,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnalysisTelemetry {
    pub analysis_id: Option<String>,
    pub provider: Option<String>,
    pub is_canary: bool,
}

impl AnalysisTelemetry {
    pub fn new(analysis_id: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            analysis_id: Some(analysis_id.into()),
            provider: Some(provider.into()),
            is_canary: false,
        }
    }

    pub fn with_canary(mut self, is_canary: bool) -> Self {
        self.is_canary = is_canary;
        self
    }

    pub fn event_json(
        &self,
        level: &str,
        component: &str,
        event: &str,
        fields: impl IntoIterator<Item = (&'static str, Value)>,
    ) -> String {
        let mut payload = Map::new();
        payload.insert("level".to_owned(), json!(level));
        payload.insert("component".to_owned(), json!(component));
        payload.insert("event".to_owned(), json!(event));

        if let Some(analysis_id) = &self.analysis_id {
            payload.insert("analysis_id".to_owned(), json!(analysis_id));
        }

        if let Some(provider) = &self.provider {
            payload.insert("provider".to_owned(), json!(provider));
        }

        payload.insert("is_canary".to_owned(), json!(self.is_canary));

        for (key, value) in fields {
            payload.insert(key.to_owned(), value);
        }

        Value::Object(payload).to_string()
    }

    pub fn log(
        &self,
        level: &str,
        component: &str,
        event: &str,
        fields: impl IntoIterator<Item = (&'static str, Value)>,
    ) {
        eprintln!("{}", self.event_json(level, component, event, fields));
    }
}

/// A sampled video frame ready to send to a vision model.
///
/// Frames keep their source video and timestamp so chunk prompts can preserve
/// temporal context even when multiple videos are analyzed together.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoFrame {
    pub video_name: String,
    pub timestamp_secs: f64,
    pub mime_type: &'static str,
    pub bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::analysis::request::AnalysisTelemetry;

    #[test]
    fn telemetry_events_render_as_structured_json_with_correlation_fields() {
        let telemetry = AnalysisTelemetry::new("analysis-123", "openai").with_canary(true);

        let line = telemetry.event_json(
            "info",
            "openai",
            "request_retry",
            [
                ("stage", json!("chunk 1/1")),
                ("attempt", json!(2)),
                ("payload_bytes", json!(4096)),
            ],
        );
        let value: serde_json::Value =
            serde_json::from_str(&line).expect("log line should be json");

        assert_eq!(value["analysis_id"], "analysis-123");
        assert_eq!(value["provider"], "openai");
        assert_eq!(value["is_canary"], true);
        assert_eq!(value["component"], "openai");
        assert_eq!(value["event"], "request_retry");
        assert_eq!(value["stage"], "chunk 1/1");
        assert_eq!(value["attempt"], 2);
        assert_eq!(value["payload_bytes"], 4096);
    }
}
