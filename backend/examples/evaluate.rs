//! Isolated evaluations using the production planner, provider, and durable checkpoint flow.
//! Usage: evaluate <manifest.json> [--dry-run|--one-batch]. Credentials are process inputs only.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use backend::{
    analysis::{Agent, Analyzer, LocalChatAgent, OpenAiAgent, OpenAiConfig},
    profiles::AnalysisProfile,
    recording::list_segments,
    session::Session,
};
use serde::Deserialize;

/// A reproducible evaluation input. Source metadata/media must be isolated before running.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    directory: PathBuf,
    checklist: String,
    profile: AnalysisProfile,
    checkpoint_path: PathBuf,
    base_url: Option<String>,
    #[serde(default)]
    chat_completions: bool,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("usage: evaluate <manifest.json> [--dry-run|--one-batch]")]
    Arguments,
    #[error("evaluation input or output I/O failed")]
    Io(#[from] std::io::Error),
    #[error("evaluation manifest JSON is invalid")]
    Json(#[from] serde_json::Error),
    #[error("session metadata is invalid")]
    Session(#[from] backend::session::Error),
    #[error("recordings are invalid")]
    Recording(#[from] backend::recording::Error),
    #[error("analysis failed: {0}")]
    Analysis(#[from] backend::analysis::Error),
    #[error("provider failed: {0}")]
    Provider(#[from] backend::analysis::ProviderError),
    #[error("paid evaluation requires a key, explicit opt-in, and an accounting journal")]
    MissingAuthorization,
    #[error("session must be completed, and output must be a separate absolute path")]
    InvalidPaths,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty()
        || args.len() > 2
        || (args.len() == 2 && !matches!(args[1].as_str(), "--dry-run" | "--one-batch"))
    {
        return Err(Error::Arguments);
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(&args[0])?)?;
    if let Some(base) = &manifest.base_url {
        let url = url::Url::parse(base).map_err(|_| Error::MissingAuthorization)?;
        if !matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost")) {
            return Err(Error::MissingAuthorization);
        }
    }
    let dry_run = args.get(1).is_some_and(|arg| arg == "--dry-run");
    let one_batch = args.get(1).is_some_and(|arg| arg == "--one-batch");
    let marker = manifest.directory.join("recording-complete");
    if !manifest.directory.is_absolute()
        || !manifest.checkpoint_path.is_absolute()
        || !fs::symlink_metadata(&manifest.directory)?
            .file_type()
            .is_dir()
        || !fs::symlink_metadata(&marker)?.file_type().is_file()
        || fs::metadata(marker)?.len() != 0
        || manifest.checklist.trim().is_empty()
    {
        return Err(Error::InvalidPaths);
    }
    validate_output_path(&manifest.directory, &manifest.checkpoint_path)?;
    let session = Session::load(&manifest.directory.join("events.jsonl"))?;
    let camera_ids: Vec<u32> = session.cameras.iter().map(|camera| camera.id).collect();
    let segments = list_segments(&manifest.directory.join("recordings"), &camera_ids)?;
    let mut analyzer = Analyzer::resume(
        segments,
        session,
        manifest.checklist,
        manifest.profile.clone(),
        manifest.checkpoint_path,
    )
    .await?;
    println!("{}", serde_json::to_string(&analyzer.plan_summary())?);
    if dry_run {
        return Ok(());
    }
    let api_key = if manifest.base_url.is_none() {
        if std::env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref() != Ok("1")
            || std::env::var_os("LEO_EVALUATION_JOURNAL").is_none()
        {
            return Err(Error::MissingAuthorization);
        }
        std::env::var("OPENAI_API_KEY").map_err(|_| Error::MissingAuthorization)?
    } else {
        "local-evaluation".into()
    };
    let config = OpenAiConfig {
        api_key,
        base_url: manifest.base_url,
    };
    if manifest.chat_completions {
        let agent = LocalChatAgent::from_local_config(
            config,
            &manifest.profile,
            manifest.reasoning_effort,
        )?;
        run_batches(&mut analyzer, &agent, one_batch).await
    } else {
        let agent = OpenAiAgent::from_config(config, &manifest.profile)?;
        run_batches(&mut analyzer, &agent, one_batch).await
    }
}

// Resolve the existing output parent so aliases cannot put a checkpoint into source media.
fn validate_output_path(source: &Path, output: &Path) -> Result<(), Error> {
    let source = fs::canonicalize(source)?;
    let parent = output.parent().ok_or(Error::InvalidPaths)?;
    if fs::canonicalize(parent)?.starts_with(source) {
        return Err(Error::InvalidPaths);
    }
    match fs::symlink_metadata(output) {
        Ok(metadata) if !metadata.file_type().is_file() => Err(Error::InvalidPaths),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn run_batches<M: rig_core::completion::CompletionModel>(
    analyzer: &mut Analyzer,
    agent: &Agent<M>,
    one_batch: bool,
) -> Result<(), Error> {
    while analyzer.checkpoint().responses.len() < analyzer.checkpoint().total_batches {
        let started = Instant::now();
        analyzer.analyze_next(agent).await?;
        println!(
            "{}",
            serde_json::json!({
                "completed": analyzer.checkpoint().responses.len(),
                "total": analyzer.checkpoint().total_batches,
                "batch_elapsed_ms": started.elapsed().as_millis(),
            })
        );
        if one_batch {
            break;
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_output_cannot_alias_the_source_session() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let outputs = temp.path().join("outputs");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&outputs).unwrap();
        let alias = temp.path().join("source-alias");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        for path in [
            source.join("new.json"),
            outputs.join("../source/new.json"),
            alias.join("new.json"),
        ] {
            assert!(matches!(
                validate_output_path(&source, &path),
                Err(Error::InvalidPaths)
            ));
        }
        let target = source.join("events.jsonl");
        fs::write(&target, "source metadata").unwrap();
        let linked_output = outputs.join("linked.json");
        std::os::unix::fs::symlink(target, &linked_output).unwrap();
        assert!(validate_output_path(&source, &linked_output).is_err());
        assert!(validate_output_path(&source, &outputs.join("analysis.json")).is_ok());
    }
}
