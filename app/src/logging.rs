use std::path::Path;

use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::settings::LogLevel;

/// Keeps the nonblocking JSON writer alive until process shutdown.
pub struct LogGuard {
    _worker: WorkerGuard,
}

/// Structured logging initialization failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to initialize the daily JSON log")]
    Appender(#[from] tracing_appender::rolling::InitError),
    #[error("failed to install the process logging subscriber")]
    Subscriber(#[from] tracing_subscriber::util::TryInitError),
}

/// Installs compact console output and daily JSON logging in `directory`.
pub fn init(directory: &Path, level: LogLevel) -> Result<LogGuard, Error> {
    let (subscriber, guard) = build_subscriber(directory, level)?;
    subscriber.try_init()?;

    Ok(guard)
}

/// Installs compact stderr logging without a file appender.
pub fn init_stderr(level: LogLevel) -> Result<(), Error> {
    tracing_subscriber::registry()
        .with(filter(level))
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_writer(std::io::stderr),
        )
        .try_init()?;
    Ok(())
}

fn filter(level: LogLevel) -> EnvFilter {
    EnvFilter::new(level.as_str()).add_directive(
        "rig::completions=off"
            .parse()
            .expect("provider payload filter should be valid"),
    )
}

fn build_subscriber(
    directory: &Path,
    level: LogLevel,
) -> Result<(impl tracing::Subscriber + Send + Sync, LogGuard), Error> {
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("leo.jsonl")
        .build(directory)?;
    let (writer, worker) = tracing_appender::non_blocking(appender);
    let subscriber = tracing_subscriber::registry()
        .with(filter(level))
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_writer(std::io::stderr),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_ansi(false)
                .with_writer(writer),
        );

    Ok((subscriber, LogGuard { _worker: worker }))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::build_subscriber;
    use crate::settings::LogLevel;
    use serde_json::Value;

    #[test]
    fn writes_structured_events_to_daily_json_log() {
        let directory = tempfile::tempdir().unwrap();
        let (subscriber, guard) = build_subscriber(directory.path(), LogLevel::Info).unwrap();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(camera_id = 41, camera_count = 2, "preview ready");
        });
        drop(guard);

        let entries = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("leo.jsonl.")
        );
        let contents = fs::read_to_string(&entries[0]).unwrap();
        let event: Value = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .find(|event: &Value| event["fields"]["message"] == "preview ready")
            .expect("structured preview event should be written");

        assert_eq!(event["level"], "INFO");
        assert_eq!(event["fields"]["camera_id"], 41);
        assert_eq!(event["fields"]["camera_count"], 2);
    }

    #[test]
    fn warn_level_omits_info_and_retains_warning() {
        let directory = tempfile::tempdir().unwrap();
        let (subscriber, guard) = build_subscriber(directory.path(), LogLevel::Warn).unwrap();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("filtered info event");
            tracing::warn!("retained warning event");
        });
        drop(guard);

        let path = fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let contents = fs::read_to_string(path).unwrap();

        assert!(!contents.contains("filtered info event"));
        assert!(contents.contains("retained warning event"));
    }

    #[test]
    fn never_writes_provider_payloads_when_trace_is_enabled() {
        let directory = tempfile::tempdir().unwrap();
        let (subscriber, guard) = build_subscriber(directory.path(), LogLevel::Trace).unwrap();

        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(
                target: "rig::completions",
                payload = "private checklist and image bytes",
                "provider request"
            );
        });
        drop(guard);

        let path = fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let contents = fs::read_to_string(path).unwrap();

        assert!(!contents.contains("private checklist and image bytes"));
    }
}
