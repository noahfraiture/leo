use std::path::Path;

use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};

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
pub fn init(directory: &Path) -> Result<LogGuard, Error> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let (subscriber, guard) = build_subscriber(directory, filter)?;
    subscriber.try_init()?;

    Ok(guard)
}

fn build_subscriber(
    directory: &Path,
    filter: EnvFilter,
) -> Result<(impl tracing::Subscriber + Send + Sync, LogGuard), Error> {
    let filter = filter.add_directive(
        "rig::completions=off"
            .parse()
            .expect("provider payload filter should be valid"),
    );
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("leo.jsonl")
        .build(directory)?;
    let (writer, worker) = tracing_appender::non_blocking(appender);
    let subscriber = tracing_subscriber::registry()
        .with(filter)
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

    use serde_json::Value;
    use tracing_subscriber::EnvFilter;

    use super::build_subscriber;

    #[test]
    fn writes_structured_events_to_daily_json_log() {
        let directory = tempfile::tempdir().unwrap();
        let (subscriber, guard) =
            build_subscriber(directory.path(), EnvFilter::new("info")).unwrap();

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
    fn never_writes_provider_payloads_when_trace_is_enabled() {
        let directory = tempfile::tempdir().unwrap();
        let (subscriber, guard) =
            build_subscriber(directory.path(), EnvFilter::new("trace")).unwrap();

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
