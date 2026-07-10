//! Supported AI provider identifiers and parsing.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// AI provider selected for a video analysis job.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnalysisProvider {
    Gemini,
    Gemma,
    Mistral,
    OpenAi,
    Qwen,
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
            Self::Gemma => "gemma",
            Self::Mistral => "mistral",
            Self::OpenAi => "openai",
            Self::Qwen => "qwen",
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
            "gemma" => Ok(Self::Gemma),
            "mistral" => Ok(Self::Mistral),
            "openai" | "open-ai" => Ok(Self::OpenAi),
            "qwen" => Ok(Self::Qwen),
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
        assert_eq!(
            AnalysisProvider::from_str("Gemma").expect("provider should parse"),
            AnalysisProvider::Gemma
        );
        assert_eq!(
            AnalysisProvider::from_str("Qwen").expect("provider should parse"),
            AnalysisProvider::Qwen
        );
    }

    #[test]
    fn provider_parses_and_displays_mistral_case_insensitively() {
        let provider =
            AnalysisProvider::from_str("MiStRaL").expect("Mistral provider should parse");

        assert_eq!(provider, AnalysisProvider::Mistral);
        assert_eq!(provider.as_str(), "mistral");
        assert_eq!(provider.to_string(), "mistral");
    }

    #[test]
    fn provider_rejects_unknown_values() {
        assert!(AnalysisProvider::from_str("anthropic").is_err());
    }
}
