use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
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
    pub monitoring_error: Option<String>,
    pub analysis_error: Option<String>,
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
        settings
            .validate_recording()
            .map_err(Error::InvalidSettings)?;
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
        let monitoring_error = settings
            .validate_monitoring()
            .err()
            .map(|error| error.to_string());
        let analysis_error = settings
            .validate_analysis()
            .err()
            .map(|error| error.to_string());
        Ok(ResolvedSettings {
            settings,
            sessions_root,
            logs_root,
            recorder_settings,
            monitoring_error,
            analysis_error,
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
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| Error::ParseSettings {
                path: self.settings_path.clone(),
            })?;
        if let Some(version) = value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            .and_then(|version| u32::try_from(version).ok())
            && version != super::model::SETTINGS_SCHEMA_VERSION
        {
            return Err(Error::InvalidSettings(super::ValidationErrors(vec![
                super::ValidationError::UnsupportedSchemaVersion {
                    expected: super::model::SETTINGS_SCHEMA_VERSION,
                    actual: version,
                },
            ])));
        }
        let mut monitoring_parse_error = None;
        let mut analysis_parse_error = None;
        if let Some(object) = value.as_object_mut() {
            // Optional sections decode independently. Empty replacements disable the affected feature.
            for (key, replacement, valid) in [
                (
                    "monitoringProfiles",
                    serde_json::json!([]),
                    object.get("monitoringProfiles").is_some_and(|v| {
                        serde_json::from_value::<Vec<backend::profiles::MonitoringProfile>>(
                            v.clone(),
                        )
                        .is_ok()
                    }),
                ),
                (
                    "analysisProfiles",
                    serde_json::json!([]),
                    object.get("analysisProfiles").is_some_and(|v| {
                        serde_json::from_value::<Vec<backend::profiles::AnalysisProfile>>(v.clone())
                            .is_ok()
                    }),
                ),
                (
                    "openai",
                    serde_json::json!({"apiKey":"", "baseUrl":null}),
                    object.get("openai").is_some_and(|v| {
                        serde_json::from_value::<super::OpenAiSettings>(v.clone()).is_ok()
                    }),
                ),
            ] {
                if !valid {
                    let message = Some(format!(
                        "Invalid {key} section in Settings; correct it before using this feature."
                    ));
                    if key == "monitoringProfiles" {
                        monitoring_parse_error = message;
                    } else {
                        analysis_parse_error = message;
                    }
                    object.insert(key.into(), replacement);
                }
            }
            if let Some(cameras) = object
                .get_mut("cameras")
                .and_then(serde_json::Value::as_array_mut)
            {
                for camera in cameras {
                    if let Some(camera) = camera.as_object_mut()
                        && !camera
                            .get("initiallyIncludedInAnalysis")
                            .is_some_and(serde_json::Value::is_boolean)
                    {
                        camera.insert(
                            "initiallyIncludedInAnalysis".into(),
                            serde_json::json!(false),
                        );
                        monitoring_parse_error = Some("Invalid initiallyIncludedInAnalysis in Settings; correct it before enabling monitoring metadata.".into());
                    }
                }
            }
            for key in [
                "nextMonitoringProfileId",
                "nextAnalysisProfileId",
                "defaultAnalysisProfileId",
            ] {
                if !object
                    .get(key)
                    .is_some_and(|v| v.as_u64().is_some_and(|id| u32::try_from(id).is_ok()))
                {
                    object.insert(key.into(), serde_json::json!(0));
                }
            }
        }
        let settings = serde_json::from_value(value).map_err(|_| Error::ParseSettings {
            path: self.settings_path.clone(),
        })?;
        let mut resolved = self.resolve(settings)?;
        resolved.monitoring_error = monitoring_parse_error.or(resolved.monitoring_error);
        resolved.analysis_error = analysis_parse_error.or(resolved.analysis_error);
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
    for directory in [settings_parent, &resolved.sessions_root] {
        fs::create_dir_all(directory).map_err(|source| Error::CreateDirectory {
            path: directory.to_owned(),
            source,
        })?;
    }
    if let Err(error) = fs::create_dir_all(&resolved.logs_root) {
        tracing::warn!(error = %error, "diagnostic file logging unavailable; recording storage is ready");
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
#[path = "tests/store.rs"]
mod tests;
