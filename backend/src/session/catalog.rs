//! Discovers analyzable completed sessions and creates their durable completion markers.

use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

use super::{
    error::{Error, Result},
    event_log::Session,
};

/// A completed session and the directory containing its durable files.
#[derive(Debug)]
pub struct StoredSession {
    /// Direct child directory discovered under the catalogue root.
    pub directory: PathBuf,
    /// Strictly replayed, completed session event log.
    pub session: Session,
}

/// Lists marked, completed sessions directly beneath `root`, newest first.
pub fn list_sessions(root: &Path) -> Result<Vec<StoredSession>> {
    let root_metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if !root_metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "session catalogue root is not a direct directory",
        )
        .into());
    }

    let mut sessions = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let directory = entry.path();
        let child_metadata = match fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            Err(_) => {
                tracing::warn!(path = %directory.display(), "skipping invalid session catalogue entry");
                continue;
            }
        };
        if !child_metadata.file_type().is_dir() {
            if child_metadata.file_type().is_symlink() {
                tracing::warn!(path = %directory.display(), "skipping invalid session catalogue entry");
            } else {
                tracing::debug!(path = %directory.display(), "skipping unrelated session catalogue entry");
            }
            continue;
        }

        let events_path = directory.join("events.jsonl");
        let marker_path = directory.join("recording-complete");
        let valid_files = fs::symlink_metadata(&events_path)
            .is_ok_and(|metadata| metadata.file_type().is_file())
            && fs::symlink_metadata(&marker_path)
                .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() == 0);
        if !valid_files {
            tracing::warn!(path = %directory.display(), "skipping invalid or active session directory");
            continue;
        }

        match Session::load(&events_path) {
            Ok(session) => sessions.push(StoredSession { directory, session }),
            Err(_) => {
                tracing::warn!(path = %directory.display(), "skipping invalid or active session directory");
            }
        }
    }

    sessions.sort_by(|left, right| {
        right
            .session
            .start_utc_ms
            .cmp(&left.session.start_utc_ms)
            .then_with(|| right.session.id.cmp(&left.session.id))
    });
    Ok(sessions)
}

/// Durably creates the zero-byte completion marker without replacing an existing entry.
pub fn mark_recording_complete(directory: &Path) -> Result<()> {
    if !fs::symlink_metadata(directory)?.file_type().is_dir() {
        return Err(Error::InvalidSessionDirectory);
    }

    let path = directory.join("recording-complete");
    let mut temporary = tempfile::Builder::new()
        .prefix(".recording-complete-")
        .tempfile_in(directory)?;
    temporary.write_all(&[0])?;
    temporary.as_file().sync_all()?;
    let file = temporary
        .persist_noclobber(&path)
        .map_err(|error| error.error)?;
    let completed = (|| {
        File::open(directory)?.sync_all()?;
        file.set_len(0)?;
        file.sync_all()
    })();
    if let Err(first_error) = completed {
        let _ = file.set_len(1).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = fs::remove_file(&path) {
            let _ = File::open(directory).and_then(|directory| directory.sync_all());
            return Err(error.into());
        }
        let _ = File::open(directory).and_then(|directory| directory.sync_all());
        return Err(first_error.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/catalog.rs"]
mod tests;
