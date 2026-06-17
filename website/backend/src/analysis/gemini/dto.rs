//! Gemini API response shapes and file-state parsing.

use serde::{Deserialize, Deserializer};

#[derive(Debug, PartialEq)]
pub(super) struct UploadedFile {
    pub uri: String,
    pub mime_type: String,
}

#[derive(Deserialize)]
pub(super) struct UploadResponse {
    pub file: UploadedFileResponse,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum GetFileResponse {
    Wrapped { file: UploadedFileResponse },
    Direct(UploadedFileResponse),
}

impl GetFileResponse {
    pub(super) fn into_file(self) -> UploadedFileResponse {
        match self {
            Self::Wrapped { file } | Self::Direct(file) => file,
        }
    }
}

#[derive(Deserialize)]
pub(super) struct UploadedFileResponse {
    pub name: String,
    pub uri: String,
    #[serde(default, deserialize_with = "deserialize_file_state")]
    pub state: FileState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FileState {
    Unspecified,
    Processing,
    Active,
    Failed,
}

impl Default for FileState {
    fn default() -> Self {
        Self::Unspecified
    }
}

impl FileState {
    pub(super) fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    pub(super) fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    #[cfg(test)]
    pub(super) fn is_waitable(self) -> bool {
        matches!(self, Self::Unspecified | Self::Processing)
    }
}

fn deserialize_file_state<'de, D>(deserializer: D) -> Result<FileState, D::Error>
where
    D: Deserializer<'de>,
{
    let state = Option::<String>::deserialize(deserializer)?;

    Ok(match state.as_deref() {
        Some("ACTIVE") => FileState::Active,
        Some("FAILED") => FileState::Failed,
        Some("PROCESSING") => FileState::Processing,
        _ => FileState::Unspecified,
    })
}

#[derive(Deserialize)]
pub(super) struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

impl GenerateContentResponse {
    pub(super) fn text(self) -> Option<String> {
        let text = self
            .candidates
            .into_iter()
            .flat_map(|candidate| candidate.content.parts)
            .filter_map(|part| part.text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");

        if text.is_empty() { None } else { Some(text) }
    }
}

#[derive(Deserialize)]
struct Candidate {
    content: Content,
}

#[derive(Deserialize)]
struct Content {
    #[serde(default)]
    parts: Vec<ResponsePart>,
}

#[derive(Deserialize)]
struct ResponsePart {
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FileState, GetFileResponse, UploadResponse};

    #[test]
    fn upload_response_keeps_file_name_uri_and_state() {
        let response: UploadResponse = serde_json::from_value(json!({
            "file": {
                "name": "files/46pyf29h2xti",
                "uri": "https://generativelanguage.googleapis.com/v1beta/files/46pyf29h2xti",
                "state": "PROCESSING"
            }
        }))
        .expect("upload response should deserialize");

        assert_eq!(response.file.name, "files/46pyf29h2xti");
        assert_eq!(
            response.file.uri,
            "https://generativelanguage.googleapis.com/v1beta/files/46pyf29h2xti"
        );
        assert_eq!(response.file.state, FileState::Processing);
    }

    #[test]
    fn get_file_response_accepts_direct_file_shape() {
        let response: GetFileResponse = serde_json::from_value(json!({
            "name": "files/46pyf29h2xti",
            "uri": "https://generativelanguage.googleapis.com/v1beta/files/46pyf29h2xti",
            "state": "ACTIVE"
        }))
        .expect("get file response should deserialize");
        let file = response.into_file();

        assert_eq!(file.name, "files/46pyf29h2xti");
        assert_eq!(file.state, FileState::Active);
    }

    #[test]
    fn file_state_detects_ready_and_failed_states() {
        assert!(FileState::Active.is_active());
        assert!(!FileState::Processing.is_active());
        assert!(FileState::Failed.is_failed());
        assert!(FileState::Unspecified.is_waitable());
        assert!(FileState::Processing.is_waitable());
        assert!(!FileState::Failed.is_waitable());
    }
}
