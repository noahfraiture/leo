use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use backend::{analysis::OpenAiConfig, recording::RecorderSettings};

use super::{Error, LogLevel, Settings};

const DURABILITY_WARNING: &str =
    "Settings were saved, but crash durability could not be confirmed.";

/// Validated persisted settings and their runtime-only derived values.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedSettings {
    pub settings: Settings,
    pub settings_path: PathBuf,
    pub data_root: PathBuf,
    pub sessions_root: PathBuf,
    pub logs_root: PathBuf,
    pub recorder_settings: RecorderSettings,
    pub openai: Option<OpenAiConfig>,
    pub log_level: LogLevel,
}

/// Distinguishes first-run defaults from settings loaded from disk.
pub enum LoadOutcome {
    Missing(ResolvedSettings),
    Loaded(ResolvedSettings),
}

/// One visible save and any post-replacement durability warning.
pub struct SaveOutcome {
    pub resolved: ResolvedSettings,
    pub durability_warning: Option<String>,
}

/// Platform or explicitly located application settings storage.
#[derive(Clone, PartialEq, Eq)]
pub struct SettingsStore {
    pub settings_path: PathBuf,
    pub default_data_root: PathBuf,
}

impl SettingsStore {
    /// Resolves Leo's settings file and default data root for the current platform.
    pub fn platform() -> Result<Self, Error> {
        let config_dir = dirs::config_dir().ok_or(Error::StandardDirectoryUnavailable {
            category: "configuration",
        })?;
        let data_dir =
            dirs::data_dir().ok_or(Error::StandardDirectoryUnavailable { category: "data" })?;
        let application_directory = if cfg!(target_os = "linux") {
            "leo"
        } else {
            "Leo"
        };
        let default_data_root = if cfg!(target_os = "linux") {
            data_dir.join(application_directory)
        } else {
            data_dir.join(application_directory).join("data")
        };
        Self::new(
            config_dir.join(application_directory).join("settings.json"),
            default_data_root,
        )
    }

    /// Creates a store at explicit absolute paths, primarily for isolated launchers and tests.
    pub fn new(settings_path: PathBuf, default_data_root: PathBuf) -> Result<Self, Error> {
        if !settings_path.is_absolute() {
            return Err(Error::NonAbsolutePath {
                category: "settings file",
                path: settings_path,
            });
        }
        if !default_data_root.is_absolute() {
            return Err(Error::NonAbsolutePath {
                category: "default data root",
                path: default_data_root,
            });
        }
        Ok(Self {
            settings_path,
            default_data_root,
        })
    }

    /// Validates persisted settings and derives paths and runtime configuration without I/O.
    pub fn resolve(&self, settings: Settings) -> Result<ResolvedSettings, Error> {
        settings.validate().map_err(Error::InvalidSettings)?;
        let data_root = settings
            .data_root
            .clone()
            .unwrap_or_else(|| self.default_data_root.clone());
        let sessions_root = data_root.join("sessions");
        let logs_root = data_root.join("logs");
        let recorder_settings = RecorderSettings {
            io_timeout: Duration::from_secs(settings.recorder_timeout_secs),
            retry_delay: Duration::from_secs(1),
            stop_timeout: Duration::from_secs(5),
        };
        let openai = settings.openai_config();
        let log_level = settings.log_level;
        Ok(ResolvedSettings {
            settings,
            settings_path: self.settings_path.clone(),
            data_root,
            sessions_root,
            logs_root,
            recorder_settings,
            openai,
            log_level,
        })
    }

    /// Loads and prepares valid saved settings, or returns untouched first-run defaults.
    pub fn load(&self) -> Result<LoadOutcome, Error> {
        match fs::symlink_metadata(&self.settings_path) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => {
                return Err(Error::InvalidSettingsFile {
                    path: self.settings_path.clone(),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return self.resolve(Settings::default()).map(LoadOutcome::Missing);
            }
            Err(source) => {
                return Err(Error::InspectSettingsFile {
                    path: self.settings_path.clone(),
                    source,
                });
            }
        }

        let bytes = fs::read(&self.settings_path).map_err(|source| Error::ReadSettings {
            path: self.settings_path.clone(),
            source,
        })?;
        // Serde errors can echo invalid string values, so retain only the file category and path.
        let settings = serde_json::from_slice(&bytes).map_err(|_| Error::ParseSettings {
            path: self.settings_path.clone(),
        })?;
        let resolved = self.resolve(settings)?;
        prepare_directories(&resolved)?;
        Ok(LoadOutcome::Loaded(resolved))
    }

    /// Validates, prepares storage, and atomically replaces the settings file.
    pub fn save(&self, settings: &Settings) -> Result<SaveOutcome, Error> {
        let resolved = self.resolve(settings.clone())?;
        prepare_directories(&resolved)?;
        let durability_warning = atomic_save_with(
            settings,
            &self.settings_path,
            |temporary, path| {
                temporary
                    .persist(path)
                    .map(|_| ())
                    .map_err(|error| error.error)
            },
            |parent| File::open(parent)?.sync_all(),
        )?;
        Ok(SaveOutcome {
            resolved,
            durability_warning,
        })
    }
}

fn prepare_directories(resolved: &ResolvedSettings) -> Result<(), Error> {
    let settings_parent =
        resolved
            .settings_path
            .parent()
            .ok_or_else(|| Error::SettingsPathWithoutParent {
                path: resolved.settings_path.clone(),
            })?;
    for directory in [
        settings_parent,
        &resolved.data_root,
        &resolved.sessions_root,
        &resolved.logs_root,
    ] {
        prepare_directory(directory)?;
        probe_directory(directory)?;
    }
    Ok(())
}

fn prepare_directory(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => return Ok(()),
        Ok(_) => {
            return Err(Error::InvalidDirectory {
                path: path.to_owned(),
            });
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(Error::InspectDirectory {
                path: path.to_owned(),
                source,
            });
        }
    }

    fs::create_dir_all(path).map_err(|source| Error::CreateDirectory {
        path: path.to_owned(),
        source,
    })?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(Error::InvalidDirectory {
            path: path.to_owned(),
        }),
        Err(source) => Err(Error::InspectDirectory {
            path: path.to_owned(),
            source,
        }),
    }
}

fn probe_directory(directory: &Path) -> Result<(), Error> {
    let mut probe = tempfile::Builder::new()
        .prefix(".leo-write-probe-")
        .tempfile_in(directory)
        .map_err(|source| Error::ProbeDirectory {
            path: directory.to_owned(),
            source,
        })?;
    let operation = (|| -> io::Result<()> {
        probe.write_all(&[0])?;
        probe.flush()?;
        probe.as_file().sync_all()
    })();
    let cleanup = probe.close();
    operation
        .and(cleanup)
        .map_err(|source| Error::ProbeDirectory {
            path: directory.to_owned(),
            source,
        })
}

fn atomic_save_with(
    settings: &Settings,
    path: &Path,
    replace: impl FnOnce(tempfile::NamedTempFile, &Path) -> io::Result<()>,
    sync_parent: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<Option<String>, Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::SettingsPathWithoutParent {
            path: path.to_owned(),
        })?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".leo-settings-")
        .tempfile_in(parent)
        .map_err(|source| Error::CreateTemporarySettings {
            path: path.to_owned(),
            source,
        })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|source| Error::SetTemporaryPermissions {
                path: path.to_owned(),
                source,
            })?;
    }

    serde_json::to_writer_pretty(temporary.as_file_mut(), settings).map_err(|source| {
        Error::SerializeSettings {
            path: path.to_owned(),
            source,
        }
    })?;
    (|| -> io::Result<()> {
        temporary.write_all(b"\n")?;
        temporary.flush()?;
        temporary.as_file().sync_all()
    })()
    .map_err(|source| Error::WriteTemporarySettings {
        path: path.to_owned(),
        source,
    })?;

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => {
            return Err(Error::InvalidSettingsFile {
                path: path.to_owned(),
            });
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(Error::InspectSettingsFile {
                path: path.to_owned(),
                source,
            });
        }
    }
    replace(temporary, path).map_err(|source| Error::ReplaceSettings {
        path: path.to_owned(),
        source,
    })?;

    // Replacement is already visible; a fixed warning avoids leaking platform error details.
    Ok(sync_parent(parent)
        .err()
        .map(|_| DURABILITY_WARNING.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::{Path, PathBuf},
        time::Duration,
    };

    use super::{DURABILITY_WARNING, Error, LoadOutcome, SettingsStore, atomic_save_with};
    use crate::settings::{CameraSettings, LogLevel, SETTINGS_SCHEMA_VERSION, Settings};

    fn store(root: &Path) -> SettingsStore {
        SettingsStore::new(root.join("config/settings.json"), root.join("default-data")).unwrap()
    }

    fn populated_settings(data_root: PathBuf) -> Settings {
        let mut settings = Settings::default();
        settings.next_camera_id = 2;
        settings.cameras.push(CameraSettings {
            id: 1,
            name: "Room 1".into(),
            rtsp_url: "rtsp://operator:password@camera.example/stream".into(),
            initially_included_in_analysis: false,
            sample_every_ms: 2_000,
        });
        settings.data_root = Some(data_root);
        settings.recorder_timeout_secs = 23;
        settings.openai.api_key = "test-secret-key".into();
        settings.openai.model = "test-model".into();
        settings.openai.base_url = Some("https://provider.example/v1".into());
        settings.log_level = LogLevel::Trace;
        settings
    }

    fn replace(temporary: tempfile::NamedTempFile, path: &Path) -> io::Result<()> {
        temporary
            .persist(path)
            .map(|_| ())
            .map_err(|error| error.error)
    }

    #[test]
    fn missing_file_returns_default_without_creating_runtime_storage() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());

        let LoadOutcome::Missing(resolved) = store.load().unwrap() else {
            panic!("missing settings should remain distinguishable");
        };
        assert!(resolved.settings.cameras.is_empty());
        assert!(!store.settings_path.parent().unwrap().exists());
        assert!(!resolved.data_root.exists());
        assert!(!resolved.sessions_root.exists());
        assert!(!resolved.logs_root.exists());
    }

    #[test]
    fn save_creates_and_probes_every_effective_directory() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let outcome = store.save(&Settings::default()).unwrap();

        assert!(outcome.resolved.settings_path.is_file());
        assert!(outcome.resolved.data_root.is_dir());
        assert!(outcome.resolved.sessions_root.is_dir());
        assert!(outcome.resolved.logs_root.is_dir());
        assert!(outcome.durability_warning.is_none());
        for directory in [
            outcome.resolved.settings_path.parent().unwrap(),
            &outcome.resolved.data_root,
            &outcome.resolved.sessions_root,
            &outcome.resolved.logs_root,
        ] {
            assert!(fs::read_dir(directory).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".leo-write-probe-")
            }));
        }
    }

    #[test]
    fn strict_round_trip_resolves_runtime_values_and_one_trailing_newline() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let settings = populated_settings(root.path().join("selected-data"));

        let saved = store.save(&settings).unwrap();
        assert!(saved.resolved.settings == settings);
        let bytes = fs::read(&store.settings_path).unwrap();
        assert!(bytes.ends_with(b"}\n"));
        assert!(!bytes.ends_with(b"}\n\n"));

        let LoadOutcome::Loaded(resolved) = store.load().unwrap() else {
            panic!("saved settings should load");
        };
        assert!(resolved.settings == settings);
        assert_eq!(resolved.settings_path, store.settings_path);
        assert_eq!(resolved.data_root, root.path().join("selected-data"));
        assert_eq!(resolved.sessions_root, resolved.data_root.join("sessions"));
        assert_eq!(resolved.logs_root, resolved.data_root.join("logs"));
        assert_eq!(
            resolved.recorder_settings.io_timeout,
            Duration::from_secs(23)
        );
        assert_eq!(
            resolved.recorder_settings.retry_delay,
            Duration::from_secs(1)
        );
        assert_eq!(
            resolved.recorder_settings.stop_timeout,
            Duration::from_secs(5)
        );
        assert!(resolved.openai == settings.openai_config());
        assert_eq!(resolved.log_level, LogLevel::Trace);
    }

    #[test]
    fn malformed_unknown_and_invalid_files_are_preserved() {
        let malformed = b"{not-json\n".to_vec();
        let mut unknown = serde_json::to_value(Settings::default()).unwrap();
        unknown["unexpected"] = serde_json::json!("do-not-report-this-value");
        let unknown = serde_json::to_vec_pretty(&unknown).unwrap();
        let mut unsupported = Settings::default();
        unsupported.schema_version = SETTINGS_SCHEMA_VERSION + 1;
        unsupported.openai.api_key = "unsupported-secret".into();
        let unsupported = serde_json::to_vec_pretty(&unsupported).unwrap();
        let mut invalid = Settings::default();
        invalid.next_camera_id = 2;
        invalid.cameras.push(CameraSettings {
            id: 1,
            name: "Room 1".into(),
            rtsp_url: "https://operator:password@camera.example/stream".into(),
            initially_included_in_analysis: true,
            sample_every_ms: 1_000,
        });
        invalid.openai.api_key = "validation-secret".into();
        let invalid = serde_json::to_vec_pretty(&invalid).unwrap();

        for (bytes, parse_error) in [
            (malformed, true),
            (unknown, true),
            (unsupported, false),
            (invalid, false),
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = store(root.path());
            fs::create_dir_all(store.settings_path.parent().unwrap()).unwrap();
            fs::write(&store.settings_path, &bytes).unwrap();

            let error = match store.load() {
                Err(error) => error,
                Ok(_) => panic!("invalid settings should not load"),
            };
            if parse_error {
                assert!(matches!(&error, Error::ParseSettings { .. }));
            } else {
                assert!(matches!(&error, Error::InvalidSettings(_)));
            }
            let message = error.to_string();
            assert!(!message.contains("password"));
            assert!(!message.contains("secret"));
            assert_eq!(fs::read(&store.settings_path).unwrap(), bytes);
            assert!(!store.default_data_root.exists());
        }
    }

    #[test]
    fn invalid_save_preserves_target_and_creates_no_runtime_directories() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        fs::create_dir_all(store.settings_path.parent().unwrap()).unwrap();
        fs::write(&store.settings_path, b"old settings\n").unwrap();
        let mut settings = Settings::default();
        settings.recorder_timeout_secs = 0;
        settings.data_root = Some(root.path().join("selected-data"));

        assert!(matches!(
            store.save(&settings),
            Err(Error::InvalidSettings(_))
        ));
        assert_eq!(fs::read(&store.settings_path).unwrap(), b"old settings\n");
        assert!(!root.path().join("selected-data").exists());
    }

    #[test]
    fn non_absolute_constructor_paths_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        for (settings_path, data_root, category) in [
            (
                PathBuf::from("settings.json"),
                root.path().join("data"),
                "settings file",
            ),
            (
                root.path().join("settings.json"),
                PathBuf::from("data"),
                "default data root",
            ),
        ] {
            assert!(matches!(
                SettingsStore::new(settings_path, data_root),
                Err(Error::NonAbsolutePath {
                    category: actual,
                    ..
                }) if actual == category
            ));
        }
    }

    #[test]
    fn existing_non_directory_managed_paths_are_rejected() {
        for category in ["settings parent", "data root", "sessions root", "logs root"] {
            let root = tempfile::tempdir().unwrap();
            let store = store(root.path());
            let path = match category {
                "settings parent" => store.settings_path.parent().unwrap().to_owned(),
                "data root" => store.default_data_root.clone(),
                "sessions root" => store.default_data_root.join("sessions"),
                "logs root" => store.default_data_root.join("logs"),
                _ => unreachable!(),
            };
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"not a directory").unwrap();

            assert!(matches!(
                store.save(&Settings::default()),
                Err(Error::InvalidDirectory { path: actual }) if actual == path
            ));
        }
    }

    #[test]
    fn existing_non_regular_settings_target_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        fs::create_dir_all(&store.settings_path).unwrap();

        assert!(matches!(
            store.load(),
            Err(Error::InvalidSettingsFile { path }) if path == store.settings_path
        ));
    }

    #[test]
    fn injected_replacement_failure_preserves_previous_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"old\n").unwrap();
        let error = atomic_save_with(
            &Settings::default(),
            &path,
            |_, _| Err(io::Error::other("replace failed")),
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(matches!(error, Error::ReplaceSettings { .. }));
        assert_eq!(fs::read(path).unwrap(), b"old\n");
    }

    #[test]
    fn injected_parent_sync_failure_keeps_new_bytes_and_returns_fixed_warning() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"old\n").unwrap();

        let warning = atomic_save_with(&Settings::default(), &path, replace, |_| {
            Err(io::Error::other("sensitive sync detail"))
        })
        .unwrap();

        assert_eq!(warning.as_deref(), Some(DURABILITY_WARNING));
        assert!(!warning.unwrap().contains("sensitive sync detail"));
        let bytes = fs::read(path).unwrap();
        assert!(bytes.ends_with(b"}\n"));
        assert!(serde_json::from_slice::<Settings>(&bytes).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn saved_settings_mode_is_exactly_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        store.save(&Settings::default()).unwrap();

        let mode = fs::metadata(&store.settings_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_settings_file_and_managed_directories_are_rejected() {
        use std::os::unix::fs::symlink;

        for category in [
            "settings file",
            "settings parent",
            "data root",
            "sessions root",
            "logs root",
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = store(root.path());
            let target_directory = root.path().join("target-directory");
            fs::create_dir(&target_directory).unwrap();

            let expected_path = match category {
                "settings file" => {
                    fs::create_dir_all(store.settings_path.parent().unwrap()).unwrap();
                    let target = root.path().join("target-settings.json");
                    fs::write(&target, b"target bytes\n").unwrap();
                    symlink(&target, &store.settings_path).unwrap();
                    store.settings_path.clone()
                }
                "settings parent" => {
                    symlink(&target_directory, store.settings_path.parent().unwrap()).unwrap();
                    store.settings_path.parent().unwrap().to_owned()
                }
                "data root" => {
                    symlink(&target_directory, &store.default_data_root).unwrap();
                    store.default_data_root.clone()
                }
                "sessions root" => {
                    fs::create_dir(&store.default_data_root).unwrap();
                    symlink(&target_directory, store.default_data_root.join("sessions")).unwrap();
                    store.default_data_root.join("sessions")
                }
                "logs root" => {
                    fs::create_dir(&store.default_data_root).unwrap();
                    symlink(&target_directory, store.default_data_root.join("logs")).unwrap();
                    store.default_data_root.join("logs")
                }
                _ => unreachable!(),
            };

            let error = match store.save(&Settings::default()) {
                Err(error) => error,
                Ok(_) => panic!("symlinked managed path should be rejected"),
            };
            if category == "settings file" {
                assert!(matches!(
                    error,
                    Error::InvalidSettingsFile { path } if path == expected_path
                ));
                assert_eq!(
                    fs::read(root.path().join("target-settings.json")).unwrap(),
                    b"target bytes\n"
                );
            } else {
                assert!(matches!(
                    error,
                    Error::InvalidDirectory { path } if path == expected_path
                ));
            }
        }
    }
}
