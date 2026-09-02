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
    EnvFilter::new(level.as_str())
        .add_directive(
            "rig::completions=off"
                .parse()
                .expect("provider payload filter should be valid"),
        )
        .add_directive(
            "dioxus_core::diff::node=off"
                .parse()
                .expect("Dioxus VNode filter should be valid"),
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
#[path = "tests/logging.rs"]
mod tests;
