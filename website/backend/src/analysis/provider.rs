//! Supported AI provider identifiers and parsing.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// AI provider selected for a video analysis job.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnalysisProvider {
    Gemini,
    OpenAi,
}

#[derive(Debug, Error)]
#[error("unsupported analysis provider: {value}")]
pub struct ProviderParseError {
    value: String,
}

impl AnalysisProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::OpenAi => "openai",
        }
    }
}

impl Default for AnalysisProvider {
    fn default() -> Self {
        Self::Gemini
    }
}

impl fmt::Display for AnalysisProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AnalysisProvider {
    type Err = ProviderParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "gemini" => Ok(Self::Gemini),
            "openai" | "open-ai" => Ok(Self::OpenAi),
            _ => Err(ProviderParseError {
                value: value.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::AnalysisProvider;

    #[test]
    fn provider_parses_form_values_case_insensitively() {
        assert_eq!(
            AnalysisProvider::from_str("gemini").expect("provider should parse"),
            AnalysisProvider::Gemini
        );
        assert_eq!(
            AnalysisProvider::from_str("OpenAI").expect("provider should parse"),
            AnalysisProvider::OpenAi
        );
    }

    #[test]
    fn provider_rejects_unknown_values() {
        assert!(AnalysisProvider::from_str("anthropic").is_err());
    }
}
