//! Mistral Chat Completions response DTOs and text extraction.

use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct MistralResponse {
    #[serde(default)]
    choices: Vec<MistralChoice>,
}

impl MistralResponse {
    pub(super) fn text(self) -> Option<String> {
        let text = self
            .choices
            .into_iter()
            .flat_map(|choice| choice.message.content.into_iter())
            .flat_map(MistralMessageContent::into_text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");

        if text.is_empty() { None } else { Some(text) }
    }
}

#[derive(Deserialize)]
struct MistralChoice {
    message: MistralMessage,
}

#[derive(Deserialize)]
struct MistralMessage {
    #[serde(default)]
    content: Option<MistralMessageContent>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MistralMessageContent {
    Text(String),
    Chunks(Vec<MistralContentChunk>),
}

impl MistralMessageContent {
    fn into_text(self) -> Vec<String> {
        match self {
            Self::Text(text) => vec![text],
            Self::Chunks(chunks) => chunks
                .into_iter()
                .filter_map(MistralContentChunk::into_text)
                .collect(),
        }
    }
}

#[derive(Deserialize)]
struct MistralContentChunk {
    #[serde(rename = "type")]
    content_type: Option<String>,
    text: Option<String>,
}

impl MistralContentChunk {
    fn into_text(self) -> Option<String> {
        (self.content_type.as_deref() == Some("text"))
            .then_some(self.text)
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::MistralResponse;

    #[test]
    fn response_text_accepts_string_content() {
        let response: MistralResponse = serde_json::from_value(json!({
            "choices": [{
                "message": {
                    "content": "summary"
                }
            }]
        }))
        .expect("response should deserialize");

        assert_eq!(response.text().as_deref(), Some("summary"));
    }

    #[test]
    fn response_text_joins_non_empty_text_chunks() {
        let response: MistralResponse = serde_json::from_value(json!({
            "choices": [{
                "message": {
                    "content": [
                        { "type": "text", "text": "first" },
                        { "type": "image_url", "image_url": "ignored" },
                        { "type": "text", "text": "  " },
                        { "type": "text", "text": "second" }
                    ]
                }
            }]
        }))
        .expect("response should deserialize");

        assert_eq!(response.text().as_deref(), Some("first\n\nsecond"));
    }

    #[test]
    fn response_text_returns_none_for_empty_content() {
        for value in [
            json!({ "choices": [] }),
            json!({ "choices": [{ "message": { "content": " " } }] }),
            json!({
                "choices": [{
                    "message": {
                        "content": [{ "type": "text", "text": "" }]
                    }
                }]
            }),
        ] {
            let response: MistralResponse =
                serde_json::from_value(value).expect("response should deserialize");

            assert_eq!(response.text(), None);
        }
    }
}
