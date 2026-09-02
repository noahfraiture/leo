use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    num::NonZeroUsize,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

use backend::recording::RecorderSettings;

use super::{Error, Settings};

/// Validated persisted settings and their runtime-only derived values.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedSettings {
    pub settings: Settings,
    pub sessions_root: PathBuf,
    pub logs_root: PathBuf,
    pub recorder_settings: RecorderSettings,
    pub analysis_frame_sets_per_prompt: NonZeroUsize,
    pub analysis_overlap_frame_sets: usize,
}

/// Platform or explicitly located application settings storage.
#[derive(Clone, PartialEq, Eq)]
pub struct SettingsStore {
    pub settings_path: PathBuf,
    pub default_data_root: PathBuf,
}

impl SettingsStore {
    /// Resolves Leo's settings file and default data root for the current platform.
    pub fn platform() -> Self {
        let config_dir = dirs::config_dir().expect("platform configuration directory is required");
        let data_dir = dirs::data_dir().expect("platform data directory is required");
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
    pub fn new(settings_path: PathBuf, default_data_root: PathBuf) -> Self {
        assert!(
            settings_path.is_absolute(),
            "settings path must be absolute"
        );
        assert!(
            settings_path.parent().is_some(),
            "settings path must have a parent"
        );
        assert!(
            default_data_root.is_absolute(),
            "default data root must be absolute"
        );
        Self {
            settings_path,
            default_data_root,
        }
    }

    /// Validates settings and derives paths and runtime configuration without I/O.
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
        let analysis_frame_sets_per_prompt = NonZeroUsize::new(
            usize::try_from(settings.analysis_frame_sets_per_prompt)
                .expect("validated analysis frame-set count should fit usize"),
        )
        .expect("validated analysis frame-set count should be nonzero");
        let analysis_overlap_frame_sets = usize::try_from(settings.analysis_overlap_frame_sets)
            .expect("validated analysis overlap should fit usize");
        Ok(ResolvedSettings {
            settings,
            sessions_root,
            logs_root,
            recorder_settings,
            analysis_frame_sets_per_prompt,
            analysis_overlap_frame_sets,
        })
    }

    /// Loads and prepares saved settings, returning `None` on first launch.
    pub fn load(&self) -> Result<Option<ResolvedSettings>, Error> {
        let bytes = match fs::read(&self.settings_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::ReadSettings {
                    path: self.settings_path.clone(),
                    source,
                });
            }
        };
        // Serde errors can echo field values, so expose only the file category and path.
        let settings = serde_json::from_slice(&bytes).map_err(|_| Error::ParseSettings {
            path: self.settings_path.clone(),
        })?;
        let resolved = self.resolve(settings)?;
        prepare_directories(&resolved, &self.settings_path)?;
        Ok(Some(resolved))
    }

    /// Validates settings, creates their directories, and writes the settings file.
    pub fn save(&self, settings: &Settings) -> Result<(), Error> {
        let resolved = self.resolve(settings.clone())?;
        prepare_directories(&resolved, &self.settings_path)?;
        write_settings(settings, &self.settings_path)
    }
}

fn prepare_directories(resolved: &ResolvedSettings, settings_path: &Path) -> Result<(), Error> {
    let settings_parent = settings_path
        .parent()
        .expect("settings store paths always have a parent");
    for directory in [
        settings_parent,
        &resolved.sessions_root,
        &resolved.logs_root,
    ] {
        fs::create_dir_all(directory).map_err(|source| Error::CreateDirectory {
            path: directory.to_owned(),
            source,
        })?;
    }
    Ok(())
}

fn write_settings(settings: &Settings, path: &Path) -> Result<(), Error> {
    let mut bytes =
        serde_json::to_vec_pretty(settings).map_err(|source| Error::SerializeSettings {
            path: path.to_owned(),
            source,
        })?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|source| Error::WriteSettings {
            path: path.to_owned(),
            source,
        })?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .and_then(|()| file.write_all(&bytes))
        .map_err(|source| Error::WriteSettings {
            path: path.to_owned(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        time::Duration,
    };

    use super::{Error, SettingsStore};
    use crate::settings::{CameraSettings, LogLevel, OpenAiSettings, Settings};

    fn store(root: &Path) -> SettingsStore {
        SettingsStore::new(root.join("config/settings.json"), root.join("default-data"))
    }

    fn populated_settings(data_root: PathBuf) -> Settings {
        Settings {
            next_camera_id: 2,
            cameras: vec![CameraSettings {
                id: 1,
                name: "Room 1".into(),
                rtsp_url: "rtsp://operator:password@camera.example/stream".into(),
                initially_included_in_analysis: false,
                sample_every_ms: 2_000,
            }],
            data_root: Some(data_root),
            recorder_timeout_secs: 23,
            analysis_frame_sets_per_prompt: 7,
            analysis_overlap_frame_sets: 2,
            openai: OpenAiSettings {
                api_key: "test-secret-key".into(),
                model: "test-model".into(),
                base_url: Some("https://provider.example/v1".into()),
            },
            log_level: LogLevel::Trace,
            ..Settings::default()
        }
    }

    #[test]
    fn missing_file_is_first_launch_without_side_effects() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());

        assert!(store.load().unwrap().is_none());
        assert!(!store.settings_path.parent().unwrap().exists());
        assert!(!store.default_data_root.exists());
    }

    #[test]
    fn settings_round_trip_and_resolve_runtime_values() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let settings = populated_settings(root.path().join("selected-data"));

        store.save(&settings).unwrap();
        let resolved = store.load().unwrap().expect("saved settings should load");

        assert!(resolved.settings == settings);
        let data_root = root.path().join("selected-data");
        assert_eq!(resolved.sessions_root, data_root.join("sessions"));
        assert_eq!(resolved.logs_root, data_root.join("logs"));
        assert_eq!(
            resolved.recorder_settings.io_timeout,
            Duration::from_secs(23)
        );
        assert_eq!(resolved.analysis_frame_sets_per_prompt.get(), 7);
        assert_eq!(resolved.analysis_overlap_frame_sets, 2);
        let bytes = fs::read(&store.settings_path).unwrap();
        assert!(bytes.ends_with(b"}\n"));
    }

    #[test]
    fn invalid_file_fails_without_creating_runtime_directories() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        fs::create_dir_all(store.settings_path.parent().unwrap()).unwrap();
        fs::write(&store.settings_path, b"{not-json\n").unwrap();

        assert!(matches!(store.load(), Err(Error::ParseSettings { .. })));
        assert!(!store.default_data_root.exists());
    }

    #[test]
    fn invalid_save_does_not_overwrite_the_existing_file() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        fs::create_dir_all(store.settings_path.parent().unwrap()).unwrap();
        fs::write(&store.settings_path, b"old settings\n").unwrap();
        let settings = Settings {
            recorder_timeout_secs: 0,
            ..Settings::default()
        };

        assert!(matches!(
            store.save(&settings),
            Err(Error::InvalidSettings(_))
        ));
        assert_eq!(fs::read(&store.settings_path).unwrap(), b"old settings\n");
    }

    #[test]
    fn saved_settings_are_owner_only() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        store.save(&Settings::default()).unwrap();

        let mode = fs::metadata(&store.settings_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
