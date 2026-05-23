use std::{error::Error, path::PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use tokio::fs;

use crate::analysis::{
    gemini::GeminiClient,
    openai::OpenAiClient,
    provider::AnalysisProvider,
    request::{AnalysisRequest, AnalysisSettings, AnalysisTelemetry, AnalysisVideo},
};

#[derive(Parser)]
#[command(name = "video-analysis")]
#[command(about = "Run the video analysis web app or analyze local videos from the CLI.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    Serve,
    Analyze(AnalyzeArgs),
}

#[derive(Parser)]
pub struct AnalyzeArgs {
    #[arg(long, default_value = "gemini")]
    provider: ProviderArg,
    #[arg(long)]
    model: Option<String>,
    #[arg(short, long)]
    prompt: String,
    #[arg(long, default_value_t = 0.2)]
    frame_sample_rate_fps: f64,
    #[arg(required = true)]
    videos: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ProviderArg {
    Gemini,
    Openai,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}

impl From<ProviderArg> for AnalysisProvider {
    fn from(provider: ProviderArg) -> Self {
        match provider {
            ProviderArg::Gemini => Self::Gemini,
            ProviderArg::Openai => Self::OpenAi,
        }
    }
}

pub async fn analyze(args: AnalyzeArgs) -> Result<(), Box<dyn Error>> {
    let provider = AnalysisProvider::from(args.provider);
    let videos = read_videos(&args.videos).await?;
    let prompt = validate_prompt(args.prompt)?;
    let request = AnalysisRequest {
        videos,
        prompt,
        settings: analysis_settings(args.frame_sample_rate_fps)?,
        telemetry: AnalysisTelemetry {
            provider: Some(provider.to_string()),
            ..AnalysisTelemetry::default()
        },
    };

    let response = match provider {
        AnalysisProvider::Gemini => {
            GeminiClient::from_env_with_model(args.model)?
                .analyze(request)
                .await?
        }
        AnalysisProvider::OpenAi => {
            OpenAiClient::from_env_with_model(args.model)?
                .analyze(request)
                .await?
        }
    };

    println!("{response}");
    Ok(())
}

fn validate_prompt(prompt: String) -> Result<String, Box<dyn Error>> {
    let prompt = prompt.trim().to_owned();
    if prompt.is_empty() {
        return Err("analysis prompt cannot be empty".into());
    }

    Ok(prompt)
}

fn analysis_settings(frame_sample_rate_fps: f64) -> Result<AnalysisSettings, Box<dyn Error>> {
    if frame_sample_rate_fps.is_finite() && (0.1..=8.0).contains(&frame_sample_rate_fps) {
        Ok(AnalysisSettings {
            frame_sample_rate_fps,
        })
    } else {
        Err("unsupported frame sampling rate".into())
    }
}

async fn read_videos(paths: &[PathBuf]) -> Result<Vec<AnalysisVideo>, Box<dyn Error>> {
    let mut videos = Vec::with_capacity(paths.len());

    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("video path has no valid file name: {}", path.display()))?
            .to_owned();
        let bytes = fs::read(path).await?;
        if bytes.is_empty() {
            return Err(format!("video cannot be empty: {}", path.display()).into());
        }

        videos.push(AnalysisVideo { name, bytes });
    }

    Ok(videos)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command, ProviderArg, analysis_settings, read_videos, validate_prompt};

    #[test]
    fn analyze_command_accepts_provider_model_prompt_and_multiple_videos() {
        let cli = Cli::try_parse_from([
            "video-analysis",
            "analyze",
            "--provider",
            "openai",
            "--model",
            "gpt-test",
            "--prompt",
            "Summarize the videos",
            "first.mp4",
            "second.mp4",
        ])
        .expect("cli should parse");

        let Some(Command::Analyze(args)) = cli.command else {
            panic!("expected analyze command");
        };
        assert_eq!(args.provider, ProviderArg::Openai);
        assert_eq!(args.model.as_deref(), Some("gpt-test"));
        assert_eq!(args.prompt, "Summarize the videos");
        assert_eq!(args.videos.len(), 2);
    }

    #[test]
    fn no_command_defaults_to_server_in_main() {
        let cli = Cli::try_parse_from(["video-analysis"]).expect("cli should parse");

        assert!(cli.command.is_none());
    }

    #[test]
    fn serve_command_parses_explicitly() {
        let cli = Cli::try_parse_from(["video-analysis", "serve"]).expect("cli should parse");

        assert!(matches!(cli.command, Some(Command::Serve)));
    }

    #[test]
    fn cli_validation_trims_prompt_and_rejects_invalid_sample_rates() {
        assert_eq!(
            validate_prompt("  What happens?  ".to_owned()).expect("prompt should validate"),
            "What happens?"
        );
        assert!(validate_prompt("   ".to_owned()).is_err());
        assert!(analysis_settings(2.0).is_ok());
        assert!(analysis_settings(20.0).is_err());
    }

    #[tokio::test]
    async fn read_videos_uses_local_file_names_and_bytes() {
        let dir = tempfile::tempdir().expect("tempdir should create");
        let path = dir.path().join("clip.mp4");
        std::fs::write(&path, b"video bytes").expect("video file should write");

        let videos = read_videos(&[path]).await.expect("video should read");

        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].name, "clip.mp4");
        assert_eq!(videos[0].bytes, b"video bytes");
    }
}
