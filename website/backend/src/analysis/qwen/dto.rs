//! Qwen response DTOs and text extraction.

use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct QwenResponse {
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Vec<QwenOutputItem>,
}

impl QwenResponse {
    pub(super) fn text(self) -> Option<String> {
        let text = self
            .output_text
            .into_iter()
            .chain(
                self.output
                    .into_iter()
                    .flat_map(|item| item.content)
                    .filter(|content| content.is_output_text())
                    .filter_map(|content| content.text),
            )
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");

        if text.is_empty() { None } else { Some(text) }
    }
}

#[derive(Deserialize)]
struct QwenOutputItem {
    #[serde(default)]
    content: Vec<QwenContent>,
}

#[derive(Deserialize)]
struct QwenContent {
    #[serde(rename = "type")]
    content_type: Option<String>,
    text: Option<String>,
}

impl QwenContent {
    fn is_output_text(&self) -> bool {
        self.content_type.as_deref() == Some("output_text")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::QwenResponse;

    #[test]
    fn response_text_prefers_output_text() {
        let response: QwenResponse = serde_json::from_value(json!({
            "output_text": "summary"
        }))
        .expect("response should deserialize");

        assert_eq!(response.text().as_deref(), Some("summary"));
    }

    #[test]
    fn response_text_collects_output_content_text() {
        let response: QwenResponse = serde_json::from_value(json!({
            "output": [{
                "content": [
                    { "type": "output_text", "text": "first" },
                    { "type": "output_text", "text": "" },
                    { "type": "output_text", "text": "second" }
                ]
            }]
        }))
        .expect("response should deserialize");

        assert_eq!(response.text().as_deref(), Some("first\n\nsecond"));
    }

    #[test]
    fn response_text_ignores_reasoning_text() {
        let response: QwenResponse = serde_json::from_value(json!({
            "output": [
                {
                    "type": "reasoning",
                    "content": [
                        { "type": "reasoning_text", "text": "private reasoning" }
                    ]
                },
                {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "final answer" }
                    ]
                }
            ]
        }))
        .expect("response should deserialize");

        assert_eq!(response.text().as_deref(), Some("final answer"));
    }
}
