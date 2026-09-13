use std::{collections::HashSet, fmt};

use base64::{Engine, engine::general_purpose::STANDARD};
use rig_core::{
    OneOrMany,
    completion::Message,
    message::{ImageDetail, ImageMediaType, UserContent},
};
use serde::{Deserialize, Serialize};

use crate::profiles::ImageDetailPolicy;

use super::{Error, Result};

/// Purpose of one stateless request, recorded separately from provider/model identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobRole {
    ChecklistFormatter,
    VisualReader,
    VisualReviewer,
}

/// Distinguishes examples supplied by the operator from the candidate's recorded performance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageRole {
    Reference,
    Candidate,
}

/// Encoded formats accepted by the shared completion transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
}

/// One already prepared image. Its caller owns source provenance and evidence validation.
#[derive(Clone)]
pub struct StructuredImage {
    /// Stable manifest ID, used in the prompt beside the actual image bytes.
    pub id: String,
    pub role: ImageRole,
    /// Source description, timestamp, or operator-provided reference label.
    pub description: String,
    pub bytes: Vec<u8>,
    pub format: ImageFormat,
    pub detail: ImageDetailPolicy,
}

impl fmt::Debug for StructuredImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StructuredImage")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("format", &self.format)
            .field("byte_count", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// Complete instructions and inputs for one request. Previous requests are never added implicitly.
#[derive(Clone)]
pub struct StructuredJob {
    pub role: JobRole,
    pub instructions: String,
    pub text: String,
    pub images: Vec<StructuredImage>,
    pub max_output_tokens: Option<u64>,
}

impl StructuredJob {
    /// Builds provider-neutral content, labeling every reference as ineligible candidate evidence.
    pub fn message(&self) -> Result<Message> {
        if self.instructions.trim().is_empty()
            || self.text.trim().is_empty()
            || self.max_output_tokens == Some(0)
        {
            return Err(Error::InvalidRequest);
        }
        let mut ids = HashSet::new();
        let mut content = OneOrMany::one(UserContent::text(&self.text));
        for image in &self.images {
            if image.id.trim().is_empty() || image.bytes.is_empty() || !ids.insert(&image.id) {
                return Err(Error::InvalidImage);
            }
            let role = match image.role {
                ImageRole::Reference => "REFERENCE EXAMPLE: never evidence of candidate behavior",
                ImageRole::Candidate => {
                    "CANDIDATE RECORDING: cite this image ID for visible evidence"
                }
            };
            content.push(UserContent::text(format!(
                "{role}\nImage ID: {}\n{}",
                image.id, image.description
            )));
            content.push(UserContent::image_base64(
                STANDARD.encode(&image.bytes),
                Some(match image.format {
                    ImageFormat::Jpeg => ImageMediaType::JPEG,
                    ImageFormat::Png => ImageMediaType::PNG,
                }),
                match image.detail {
                    ImageDetailPolicy::ProviderDefault => None,
                    ImageDetailPolicy::Low => Some(ImageDetail::Low),
                    ImageDetailPolicy::High => Some(ImageDetail::High),
                },
            ));
        }
        Ok(Message::User { content })
    }
}

/// Provider-reported token usage. Missing provider metrics remain `None` on `Completion`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    pub reasoning_tokens: u64,
}

/// Parsed role-specific output with non-sensitive request measurements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Completion<T> {
    pub value: T,
    pub usage: Option<TokenUsage>,
    pub elapsed_ms: u64,
}
