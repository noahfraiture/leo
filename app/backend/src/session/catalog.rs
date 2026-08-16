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
    mark_recording_complete_with_sync(
        directory,
        File::sync_all,
        |directory| File::open(directory)?.sync_all(),
        |file| file.set_len(1).and_then(|()| file.sync_all()),
        |path| fs::remove_file(path),
    )
}

fn mark_recording_complete_with_sync(
    directory: &Path,
    mut sync_marker: impl FnMut(&File) -> io::Result<()>,
    mut sync_directory: impl FnMut(&Path) -> io::Result<()>,
    invalidate_marker: impl FnOnce(&File) -> io::Result<()>,
    mut remove_marker: impl FnMut(&Path) -> io::Result<()>,
) -> Result<()> {
    if !fs::symlink_metadata(directory)?.file_type().is_dir() {
        return Err(Error::InvalidSessionDirectory);
    }

    let path = directory.join("recording-complete");
    let mut temporary = tempfile::Builder::new()
        .prefix(".recording-complete-")
        .tempfile_in(directory)?;
    temporary.write_all(&[0])?;
    sync_marker(temporary.as_file())?;
    let file = temporary
        .persist_noclobber(&path)
        .map_err(|error| error.error)?;
    let completed = (|| {
        sync_directory(directory)?;
        file.set_len(0)?;
        sync_marker(&file)
    })();
    if let Err(first_error) = completed {
        let _ = invalidate_marker(&file);
        drop(file);
        if let Err(error) = remove_marker(&path) {
            let _ = sync_directory(directory);
            return Err(error.into());
        }
        let _ = sync_directory(directory);
        return Err(first_error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs, io,
        path::{Path, PathBuf},
    };

    use serde_json::json;
    use uuid::Uuid;

    use super::mark_recording_complete_with_sync;
    use crate::session::{Error, StoredSession, list_sessions, mark_recording_complete};

    const VALID_ID: &str = "5a660250-36fc-4c2b-93fa-b04247bdad20";

    fn events(session_id: &str, start_utc_ms: i64, ended: bool) -> String {
        let mut events = vec![json!({
            "schema_version": 1,
            "sequence": 0,
            "session_id": session_id,
            "utc_ms": start_utc_ms,
            "session_offset_ms": 0,
            "action": {
                "type": "session_started",
                "cameras": [{
                    "camera_id": 1,
                    "name": "Front",
                    "enabled": true,
                    "sample_every_ms": 1_000
                }]
            }
        })];
        if ended {
            events.push(json!({
                "schema_version": 1,
                "sequence": 1,
                "session_id": session_id,
                "utc_ms": start_utc_ms + 1_000,
                "session_offset_ms": 1_000,
                "action": { "type": "session_ended" }
            }));
        }

        let mut contents = events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        contents.push('\n');
        contents
    }

    fn write_log(directory: &Path, session_id: &str, start_utc_ms: i64, ended: bool) {
        fs::create_dir_all(directory).expect("session directory should be created");
        fs::write(
            directory.join("events.jsonl"),
            events(session_id, start_utc_ms, ended),
        )
        .expect("events should be written");
    }

    fn write_complete_session(
        root: &Path,
        name: &str,
        session_id: &str,
        start_utc_ms: i64,
    ) -> PathBuf {
        let directory = root.join(name);
        write_log(&directory, session_id, start_utc_ms, true);
        fs::write(directory.join("recording-complete"), b"")
            .expect("completion marker should be written");
        directory
    }

    fn listed(root: &Path) -> Vec<StoredSession> {
        list_sessions(root).expect("session catalogue should load")
    }

    #[test]
    fn missing_root_returns_no_sessions() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");

        assert!(listed(&directory.path().join("missing")).is_empty());
    }

    #[test]
    fn catalogue_requires_ended_log_and_completion_marker() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let valid = write_complete_session(root.path(), "valid", VALID_ID, 1_000);

        write_log(
            &root.path().join("ended-without-marker"),
            "00000000-0000-0000-0000-000000000001",
            2_000,
            true,
        );
        let active = root.path().join("active-with-marker");
        write_log(
            &active,
            "00000000-0000-0000-0000-000000000002",
            3_000,
            false,
        );
        fs::write(active.join("recording-complete"), b"")
            .expect("completion marker should be written");
        let nonempty_marker = root.path().join("nonempty-marker");
        write_log(
            &nonempty_marker,
            "00000000-0000-0000-0000-000000000003",
            4_000,
            true,
        );
        fs::write(nonempty_marker.join("recording-complete"), b"x")
            .expect("invalid marker should be written");

        let sessions = listed(root.path());

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].directory, valid);
        assert_eq!(sessions[0].session.id, Uuid::parse_str(VALID_ID).unwrap());
    }

    #[test]
    fn catalogue_skips_active_malformed_nested_and_unrelated_entries() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let valid = write_complete_session(root.path(), "valid", VALID_ID, 1_000);

        let active = root.path().join("active");
        write_log(
            &active,
            "00000000-0000-0000-0000-000000000001",
            2_000,
            false,
        );
        fs::write(active.join("recording-complete"), b"")
            .expect("completion marker should be written");

        let malformed = root.path().join("malformed");
        fs::create_dir(&malformed).expect("malformed directory should be created");
        fs::write(malformed.join("events.jsonl"), "secret event contents\n")
            .expect("malformed events should be written");
        fs::write(malformed.join("recording-complete"), b"")
            .expect("completion marker should be written");

        let nested_id = "00000000-0000-0000-0000-000000000009";
        write_complete_session(&root.path().join("container"), "nested", nested_id, 9_000);
        fs::write(root.path().join("unrelated.txt"), b"unrelated")
            .expect("unrelated file should be written");
        fs::create_dir(root.path().join("unrelated-directory"))
            .expect("unrelated directory should be created");

        let sessions = listed(root.path());

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].directory, valid);
        assert_ne!(sessions[0].session.id, Uuid::parse_str(nested_id).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn catalogue_rejects_symlinked_directories_events_and_markers() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary directory should be created");
        let targets = tempfile::tempdir().expect("target directory should be created");
        let valid = write_complete_session(root.path(), "valid", VALID_ID, 1_000);

        let target_directory = write_complete_session(
            targets.path(),
            "directory-target",
            "00000000-0000-0000-0000-000000000001",
            2_000,
        );
        symlink(&target_directory, root.path().join("symlinked-directory"))
            .expect("directory symlink should be created");

        let symlinked_events = root.path().join("symlinked-events");
        fs::create_dir(&symlinked_events).expect("session directory should be created");
        let events_target = targets.path().join("events-target.jsonl");
        fs::write(
            &events_target,
            events("00000000-0000-0000-0000-000000000002", 3_000, true),
        )
        .expect("event target should be written");
        symlink(&events_target, symlinked_events.join("events.jsonl"))
            .expect("event symlink should be created");
        fs::write(symlinked_events.join("recording-complete"), b"")
            .expect("completion marker should be written");

        let symlinked_marker = root.path().join("symlinked-marker");
        write_log(
            &symlinked_marker,
            "00000000-0000-0000-0000-000000000003",
            4_000,
            true,
        );
        let marker_target = targets.path().join("marker-target");
        fs::write(&marker_target, b"").expect("marker target should be written");
        symlink(&marker_target, symlinked_marker.join("recording-complete"))
            .expect("marker symlink should be created");

        let sessions = listed(root.path());

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].directory, valid);
    }

    #[cfg(unix)]
    #[test]
    fn catalogue_rejects_a_symlinked_root() {
        use std::os::unix::fs::symlink;

        let target = tempfile::tempdir().expect("target directory should be created");
        write_complete_session(target.path(), "valid", VALID_ID, 1_000);
        let parent = tempfile::tempdir().expect("temporary directory should be created");
        let root = parent.path().join("sessions");
        symlink(target.path(), &root).expect("root symlink should be created");

        let Error::Io(error) = list_sessions(&root).expect_err("symlinked root should be rejected")
        else {
            panic!("symlinked root should return an I/O error");
        };
        assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory);
    }

    #[test]
    fn catalogue_sorts_newest_first_by_start_and_uuid() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let old = "00000000-0000-0000-0000-000000000004";
        let tied_low = "00000000-0000-0000-0000-000000000001";
        let tied_high = "00000000-0000-0000-0000-000000000002";
        let newest = "00000000-0000-0000-0000-000000000003";
        write_complete_session(root.path(), "old", old, 1_000);
        write_complete_session(root.path(), "tied-low", tied_low, 2_000);
        write_complete_session(root.path(), "tied-high", tied_high, 2_000);
        write_complete_session(root.path(), "newest", newest, 3_000);

        let ids = listed(root.path())
            .into_iter()
            .map(|stored| stored.session.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            [newest, tied_high, tied_low, old].map(|id| Uuid::parse_str(id).unwrap())
        );
    }

    #[test]
    fn mark_recording_complete_uses_create_new_and_zero_bytes() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        fs::create_dir(&directory).expect("session directory should be created");

        mark_recording_complete(&directory).expect("completion should be marked");

        let marker = directory.join("recording-complete");
        let metadata = fs::symlink_metadata(marker).expect("marker metadata should be available");
        assert!(metadata.file_type().is_file());
        assert_eq!(metadata.len(), 0);

        let Error::Io(error) = mark_recording_complete(&directory)
            .expect_err("an existing marker must not be replaced")
        else {
            panic!("existing marker should return an I/O error");
        };
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn temporary_marker_sync_failure_never_publishes_marker() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        fs::create_dir(&directory).expect("session directory should be created");
        let directory_syncs = Cell::new(0);

        let Error::Io(error) = mark_recording_complete_with_sync(
            &directory,
            |_| Err(io::Error::other("injected marker sync failure")),
            |_| {
                directory_syncs.set(directory_syncs.get() + 1);
                Ok(())
            },
            |file| file.set_len(1).and_then(|()| file.sync_all()),
            |path| fs::remove_file(path),
        )
        .expect_err("marker sync should fail") else {
            panic!("sync failure should return an I/O error");
        };

        assert_eq!(error.to_string(), "injected marker sync failure");
        assert_eq!(directory_syncs.get(), 0);
        assert!(!directory.join("recording-complete").exists());
    }

    #[test]
    fn directory_sync_failure_preserves_first_error_after_cleanup_attempts() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        fs::create_dir(&directory).expect("session directory should be created");
        let directory_syncs = Cell::new(0);

        let Error::Io(error) = mark_recording_complete_with_sync(
            &directory,
            |_| Ok(()),
            |_| {
                let call = directory_syncs.get();
                directory_syncs.set(call + 1);
                Err(io::Error::other(if call == 0 {
                    "injected initial directory sync failure"
                } else {
                    "injected cleanup directory sync failure"
                }))
            },
            |file| file.set_len(1).and_then(|()| file.sync_all()),
            |path| fs::remove_file(path),
        )
        .expect_err("directory sync should fail") else {
            panic!("sync failure should return an I/O error");
        };

        assert_eq!(error.to_string(), "injected initial directory sync failure");
        assert_eq!(directory_syncs.get(), 2);
        assert!(!directory.join("recording-complete").exists());
    }

    #[test]
    fn marker_removal_failure_leaves_an_invalid_marker() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        fs::create_dir(&directory).expect("session directory should be created");

        let Error::Io(error) = mark_recording_complete_with_sync(
            &directory,
            |_| Ok(()),
            |_| Err(io::Error::other("injected directory sync failure")),
            |file| file.set_len(1).and_then(|()| file.sync_all()),
            |_| Err(io::Error::other("injected marker removal failure")),
        )
        .expect_err("marker rollback should fail") else {
            panic!("rollback failure should return an I/O error");
        };

        assert_eq!(error.to_string(), "injected marker removal failure");
        assert_eq!(
            fs::metadata(directory.join("recording-complete"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn marker_invalidation_failure_still_attempts_removal() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        fs::create_dir(&directory).expect("session directory should be created");
        let removal_attempted = Cell::new(false);

        let Error::Io(error) = mark_recording_complete_with_sync(
            &directory,
            |_| Ok(()),
            |_| Err(io::Error::other("injected directory sync failure")),
            |_| Err(io::Error::other("injected marker invalidation failure")),
            |path| {
                removal_attempted.set(true);
                fs::remove_file(path)
            },
        )
        .expect_err("marker sync should fail") else {
            panic!("sync failure should return an I/O error");
        };

        assert_eq!(error.to_string(), "injected directory sync failure");
        assert!(removal_attempted.get());
        assert!(!directory.join("recording-complete").exists());
    }

    #[test]
    fn marker_stays_unpublished_until_the_sentinel_is_synced() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let directory = root.path().join("session");
        fs::create_dir(&directory).expect("session directory should be created");
        let marker = directory.join("recording-complete");

        let Error::Io(error) = mark_recording_complete_with_sync(
            &directory,
            |_| {
                assert!(!marker.exists(), "zero-byte marker was published too early");
                Err(io::Error::other("injected temporary marker sync failure"))
            },
            |_| Ok(()),
            |file| file.set_len(1).and_then(|()| file.sync_all()),
            |path| fs::remove_file(path),
        )
        .expect_err("temporary marker sync should fail") else {
            panic!("sync failure should return an I/O error");
        };

        assert_eq!(error.to_string(), "injected temporary marker sync failure");
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[test]
    fn mark_recording_complete_rejects_a_symlinked_path_before_marker_creation() {
        use std::os::unix::fs::symlink;

        let target = tempfile::tempdir().expect("target directory should be created");
        let parent = tempfile::tempdir().expect("temporary directory should be created");
        let directory = parent.path().join("session");
        symlink(target.path(), &directory).expect("directory symlink should be created");

        assert!(matches!(
            mark_recording_complete(&directory),
            Err(Error::InvalidSessionDirectory)
        ));
        assert!(!target.path().join("recording-complete").exists());
    }

    #[test]
    fn mark_recording_complete_rejects_a_regular_file_path_before_marker_creation() {
        let parent = tempfile::tempdir().expect("temporary directory should be created");
        let directory = parent.path().join("session");
        fs::write(&directory, b"not a directory").expect("session file should be written");

        assert!(matches!(
            mark_recording_complete(&directory),
            Err(Error::InvalidSessionDirectory)
        ));
        assert!(!directory.join("recording-complete").exists());
    }

    #[test]
    fn regular_file_root_returns_io_error() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let root = directory.path().join("sessions");
        fs::write(&root, b"not a directory").expect("root file should be written");

        assert!(matches!(list_sessions(&root), Err(Error::Io(_))));
    }
}
