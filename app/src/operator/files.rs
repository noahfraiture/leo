//! Folder access through the platform launcher, without waiting inside a UI callback.

use std::{io, path::PathBuf, process::Stdio};

use dioxus::prelude::*;
use tokio::process::Command;

use super::{Error, OperatorState};

/// Open an operator-selected local folder; the launch task survives route changes.
pub fn spawn_open_folder(mut operator: Signal<OperatorState>, directory: PathBuf) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut command = Command::new(program);
    command.arg(&directory);
    dioxus::dioxus_core::spawn_forever(async move {
        if let Err(error) = open_folder(directory, command).await {
            operator.write().report_transient_error(error.to_string());
        }
    });
}

async fn open_folder(directory: PathBuf, command: Command) -> Result<(), Error> {
    let metadata = tokio::fs::symlink_metadata(directory)
        .await
        .map_err(Error::OpenFolder)?;
    if !metadata.file_type().is_dir() {
        return Err(Error::OpenFolder(io::Error::new(
            io::ErrorKind::NotADirectory,
            "Folder path is not a direct directory",
        )));
    }
    run_folder_opener(command).await
}

async fn run_folder_opener(mut command: Command) -> Result<(), Error> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Leave the user's file browser open when Leo closes; only the launcher wait belongs here.
    let status = command.status().await.map_err(Error::OpenFolder)?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::OpenFolder(io::Error::other(
            "Folder opener exited unsuccessfully",
        )))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread")]
    async fn waiting_for_a_folder_opener_keeps_other_ui_work_responsive() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 0.25"]);
        let waiting = run_folder_opener(command);
        tokio::pin!(waiting);

        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            _ = &mut waiting => panic!("The folder opener blocked the UI timer"),
        }
        waiting.await.unwrap();
    }

    #[tokio::test]
    async fn unsuccessful_folder_openers_report_a_failure() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);
        let temporary = tempfile::tempdir().unwrap();
        for command in [
            command,
            Command::new(temporary.path().join("missing-opener")),
        ] {
            assert!(matches!(
                run_folder_opener(command).await,
                Err(Error::OpenFolder(_))
            ));
        }
    }

    #[tokio::test]
    async fn a_non_directory_is_rejected_before_the_platform_launcher_runs() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("retained-file");
        let marker = temporary.path().join("opener-ran");
        std::fs::write(&source, b"not a directory").unwrap();
        let mut command = Command::new("sh");
        command
            .args(["-c", "printf launched > \"$1\"", "opener-test"])
            .arg(&marker);

        assert!(matches!(
            open_folder(source, command).await,
            Err(Error::OpenFolder(_))
        ));
        assert!(!marker.exists());
    }
}
