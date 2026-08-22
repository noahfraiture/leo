use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use backend::recording::RecorderSettings;
use serde::Deserialize;
use url::Url;

const DEFAULT_CAMERA_CONFIG: &str = "./cameras.json";
const DEFAULT_DATA_DIR: &str = "./data";
const DEFAULT_RECORDER_TIMEOUT_SECS: &str = "10";

/// One configured camera shared by preview, recording, and analysis state.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraConfig {
    pub id: u32,
    pub name: String,
    /// Credential-bearing source URL. Do not include it in logs or errors.
    pub rtsp_url: String,
    pub enabled: bool,
    /// Analysis sampling cadence in whole milliseconds.
    pub sample_every_ms: u64,
}

/// Validated process configuration and its runtime storage roots.
#[derive(Debug, Clone, PartialEq)]
pub struct StartupConfig {
    pub cameras: Vec<CameraConfig>,
    pub data_root: PathBuf,
    pub sessions_root: PathBuf,
    pub logs_root: PathBuf,
    pub recorder_settings: RecorderSettings,
}

/// Startup configuration loading and validation failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read camera configuration at {path}")]
    ReadCameraConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse camera configuration at {path}")]
    ParseCameraConfig {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("camera configuration must contain exactly two rows, found {actual}")]
    CameraCount { actual: usize },
    #[error("camera IDs must be non-zero")]
    ZeroCameraId,
    #[error("camera ID {camera_id} is duplicated")]
    DuplicateCameraId { camera_id: u32 },
    #[error("camera {camera_id} has a blank name")]
    BlankCameraName { camera_id: u32 },
    #[error("camera {camera_id} has a blank RTSP URL")]
    BlankCameraUrl { camera_id: u32 },
    #[error("camera {camera_id} has an invalid RTSP URL")]
    InvalidCameraUrl { camera_id: u32 },
    #[error("camera {camera_id} sampling cadence must be positive whole seconds")]
    InvalidSamplingCadence { camera_id: u32 },
    #[error("{variable} must contain valid Unicode")]
    InvalidEnvironment { variable: &'static str },
    #[error("recorder timeout must be a positive integer representable as microseconds")]
    InvalidRecorderTimeout,
    #[error("runtime path is not a direct directory: {path}")]
    InvalidDirectory { path: PathBuf },
    #[error("failed to inspect runtime directory {path}")]
    InspectDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create runtime directory {path}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Loads process configuration from `LEO_*` overrides or their local defaults.
pub fn load_startup_config() -> Result<StartupConfig, Error> {
    let camera_path = env::var_os("LEO_CAMERA_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CAMERA_CONFIG));
    let data_root = env::var_os("LEO_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIR));
    let timeout = match env::var("LEO_RECORDER_TIMEOUT_SECS") {
        Ok(timeout) => timeout,
        Err(env::VarError::NotPresent) => DEFAULT_RECORDER_TIMEOUT_SECS.into(),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(Error::InvalidEnvironment {
                variable: "LEO_RECORDER_TIMEOUT_SECS",
            });
        }
    };

    load(&camera_path, &data_root, &timeout)
}

fn load(camera_path: &Path, data_root: &Path, timeout_text: &str) -> Result<StartupConfig, Error> {
    let contents = fs::read(camera_path).map_err(|source| Error::ReadCameraConfig {
        path: camera_path.to_owned(),
        source,
    })?;
    let cameras: Vec<CameraConfig> =
        serde_json::from_slice(&contents).map_err(|source| Error::ParseCameraConfig {
            path: camera_path.to_owned(),
            source,
        })?;
    validate_cameras(&cameras)?;

    let io_timeout = timeout_text
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .filter(|timeout| {
            i64::try_from(timeout.as_micros()).is_ok()
                && Instant::now().checked_add(*timeout).is_some()
        })
        .ok_or(Error::InvalidRecorderTimeout)?;

    ensure_directory(data_root)?;
    let sessions_root = data_root.join("sessions");
    let logs_root = data_root.join("logs");
    ensure_directory(&sessions_root)?;
    ensure_directory(&logs_root)?;

    Ok(StartupConfig {
        cameras,
        data_root: data_root.to_owned(),
        sessions_root,
        logs_root,
        recorder_settings: RecorderSettings {
            io_timeout,
            retry_delay: Duration::from_secs(1),
            stop_timeout: Duration::from_secs(5),
        },
    })
}

fn validate_cameras(cameras: &[CameraConfig]) -> Result<(), Error> {
    if cameras.len() != 2 {
        return Err(Error::CameraCount {
            actual: cameras.len(),
        });
    }

    let mut ids = HashSet::with_capacity(cameras.len());
    for camera in cameras {
        if camera.id == 0 {
            return Err(Error::ZeroCameraId);
        }
        if !ids.insert(camera.id) {
            return Err(Error::DuplicateCameraId {
                camera_id: camera.id,
            });
        }
        if camera.name.trim().is_empty() {
            return Err(Error::BlankCameraName {
                camera_id: camera.id,
            });
        }
        if camera.rtsp_url.trim().is_empty() {
            return Err(Error::BlankCameraUrl {
                camera_id: camera.id,
            });
        }
        if !Url::parse(&camera.rtsp_url).is_ok_and(|url| url.scheme() == "rtsp") {
            return Err(Error::InvalidCameraUrl {
                camera_id: camera.id,
            });
        }
        if camera.sample_every_ms == 0 || camera.sample_every_ms % 1_000 != 0 {
            return Err(Error::InvalidSamplingCadence {
                camera_id: camera.id,
            });
        }
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => return Ok(()),
        Ok(_) => {
            return Err(Error::InvalidDirectory {
                path: path.to_owned(),
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
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
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir()) {
        return Err(Error::InvalidDirectory {
            path: path.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::Duration,
    };

    use backend::recording::RecorderSettings;
    use serde_json::{Value, json};

    use super::{
        CameraConfig, DEFAULT_CAMERA_CONFIG, DEFAULT_DATA_DIR, DEFAULT_RECORDER_TIMEOUT_SECS,
        Error, load,
    };

    fn cameras() -> Value {
        json!([
            {
                "id": 26,
                "name": "Salon 1",
                "rtspUrl": "rtsp://camera-one.example/live",
                "enabled": true,
                "sampleEveryMs": 1000
            },
            {
                "id": 41,
                "name": "Salon 2",
                "rtspUrl": "rtsp://camera-two.example/live",
                "enabled": false,
                "sampleEveryMs": 2000
            }
        ])
    }

    fn write_cameras(directory: &Path, cameras: &Value) -> PathBuf {
        let path = directory.join("cameras.json");
        fs::write(&path, serde_json::to_vec(cameras).unwrap()).unwrap();
        path
    }

    fn load_value(directory: &Path, cameras: &Value) -> Result<super::StartupConfig, Error> {
        let camera_path = write_cameras(directory, cameras);
        load(
            &camera_path,
            &directory.join("data"),
            DEFAULT_RECORDER_TIMEOUT_SECS,
        )
    }

    #[test]
    fn loads_exact_two_camera_configuration_and_default_values() {
        let directory = tempfile::tempdir().unwrap();
        let camera_path = write_cameras(directory.path(), &cameras());
        let data_root = directory.path().join("data");

        let config = load(&camera_path, &data_root, DEFAULT_RECORDER_TIMEOUT_SECS).unwrap();

        assert_eq!(DEFAULT_CAMERA_CONFIG, "./cameras.json");
        assert_eq!(DEFAULT_DATA_DIR, "./data");
        assert_eq!(DEFAULT_RECORDER_TIMEOUT_SECS, "10");
        assert_eq!(
            config.cameras,
            vec![
                CameraConfig {
                    id: 26,
                    name: "Salon 1".into(),
                    rtsp_url: "rtsp://camera-one.example/live".into(),
                    enabled: true,
                    sample_every_ms: 1000,
                },
                CameraConfig {
                    id: 41,
                    name: "Salon 2".into(),
                    rtsp_url: "rtsp://camera-two.example/live".into(),
                    enabled: false,
                    sample_every_ms: 2000,
                },
            ]
        );
        assert_eq!(config.data_root, data_root);
        assert_eq!(config.sessions_root, data_root.join("sessions"));
        assert_eq!(config.logs_root, data_root.join("logs"));
        assert_eq!(
            config.recorder_settings,
            RecorderSettings {
                io_timeout: Duration::from_secs(10),
                retry_delay: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(5),
            }
        );
        for path in [&config.data_root, &config.sessions_root, &config.logs_root] {
            assert!(fs::symlink_metadata(path).unwrap().file_type().is_dir());
        }
    }

    #[test]
    fn loads_explicit_path_and_timeout_overrides() {
        let directory = tempfile::tempdir().unwrap();
        let camera_path = write_cameras(directory.path(), &cameras());
        let data_root = directory.path().join("external-data");

        let config = load(&camera_path, &data_root, "23").unwrap();

        assert_eq!(config.data_root, data_root);
        assert_eq!(config.recorder_settings.io_timeout, Duration::from_secs(23));
    }

    #[test]
    fn rejects_camera_count_other_than_two() {
        let directory = tempfile::tempdir().unwrap();
        let mut one = cameras();
        one.as_array_mut().unwrap().pop();
        let mut three = cameras();
        let third = three[0].clone();
        three.as_array_mut().unwrap().push(third);

        for (cameras, actual) in [(&one, 1), (&three, 3)] {
            assert!(matches!(
                load_value(directory.path(), cameras),
                Err(Error::CameraCount { actual: count }) if count == actual
            ));
        }
    }

    #[test]
    fn rejects_unknown_camera_fields() {
        let directory = tempfile::tempdir().unwrap();
        let mut value = cameras();
        value[0]["password"] = json!("secret");

        assert!(matches!(
            load_value(directory.path(), &value),
            Err(Error::ParseCameraConfig { .. })
        ));
    }

    #[test]
    fn rejects_zero_camera_id() {
        let directory = tempfile::tempdir().unwrap();
        let mut value = cameras();
        value[0]["id"] = json!(0);

        assert!(matches!(
            load_value(directory.path(), &value),
            Err(Error::ZeroCameraId)
        ));
    }

    #[test]
    fn rejects_duplicate_camera_ids() {
        let directory = tempfile::tempdir().unwrap();
        let mut value = cameras();
        value[1]["id"] = value[0]["id"].clone();

        assert!(matches!(
            load_value(directory.path(), &value),
            Err(Error::DuplicateCameraId { camera_id: 26 })
        ));
    }

    #[test]
    fn rejects_blank_camera_names() {
        for name in ["", " \t\n"] {
            let directory = tempfile::tempdir().unwrap();
            let mut value = cameras();
            value[0]["name"] = json!(name);

            assert!(matches!(
                load_value(directory.path(), &value),
                Err(Error::BlankCameraName { camera_id: 26 })
            ));
        }
    }

    #[test]
    fn rejects_blank_camera_urls() {
        for rtsp_url in ["", " \t\n"] {
            let directory = tempfile::tempdir().unwrap();
            let mut value = cameras();
            value[0]["rtspUrl"] = json!(rtsp_url);

            assert!(matches!(
                load_value(directory.path(), &value),
                Err(Error::BlankCameraUrl { camera_id: 26 })
            ));
        }
    }

    #[test]
    fn rejects_non_rtsp_camera_urls() {
        for rtsp_url in ["https://camera.example/live", "not a URL"] {
            let directory = tempfile::tempdir().unwrap();
            let mut value = cameras();
            value[0]["rtspUrl"] = json!(rtsp_url);

            assert!(matches!(
                load_value(directory.path(), &value),
                Err(Error::InvalidCameraUrl { camera_id: 26 })
            ));
        }
    }

    #[test]
    fn rejects_zero_sampling_cadence() {
        let directory = tempfile::tempdir().unwrap();
        let mut value = cameras();
        value[0]["sampleEveryMs"] = json!(0);

        assert!(matches!(
            load_value(directory.path(), &value),
            Err(Error::InvalidSamplingCadence { camera_id: 26 })
        ));
    }

    #[test]
    fn rejects_non_whole_second_sampling_cadence() {
        let directory = tempfile::tempdir().unwrap();
        let mut value = cameras();
        value[0]["sampleEveryMs"] = json!(1500);

        assert!(matches!(
            load_value(directory.path(), &value),
            Err(Error::InvalidSamplingCadence { camera_id: 26 })
        ));
    }

    #[test]
    fn reports_a_missing_camera_file() {
        let directory = tempfile::tempdir().unwrap();

        assert!(matches!(
            load(
                &directory.path().join("missing.json"),
                &directory.path().join("data"),
                "10"
            ),
            Err(Error::ReadCameraConfig { .. })
        ));
    }

    #[test]
    fn reports_a_malformed_camera_file() {
        let directory = tempfile::tempdir().unwrap();
        let camera_path = directory.path().join("cameras.json");
        fs::write(&camera_path, b"not JSON").unwrap();

        assert!(matches!(
            load(&camera_path, &directory.path().join("data"), "10"),
            Err(Error::ParseCameraConfig { .. })
        ));
    }

    #[test]
    fn rejects_a_non_directory_data_root() {
        let directory = tempfile::tempdir().unwrap();
        let camera_path = write_cameras(directory.path(), &cameras());
        let data_root = directory.path().join("data");
        fs::write(&data_root, b"not a directory").unwrap();

        assert!(matches!(
            load(&camera_path, &data_root, "10"),
            Err(Error::InvalidDirectory { path }) if path == data_root
        ));
    }

    #[test]
    fn rejects_non_directory_runtime_children() {
        for child in ["sessions", "logs"] {
            let directory = tempfile::tempdir().unwrap();
            let camera_path = write_cameras(directory.path(), &cameras());
            let data_root = directory.path().join("data");
            fs::create_dir(&data_root).unwrap();
            fs::write(data_root.join(child), b"not a directory").unwrap();

            assert!(matches!(
                load(&camera_path, &data_root, "10"),
                Err(Error::InvalidDirectory { path }) if path == data_root.join(child)
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_data_and_runtime_directories() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let camera_path = write_cameras(directory.path(), &cameras());
        let target = directory.path().join("target");
        fs::create_dir(&target).unwrap();
        let data_root = directory.path().join("data-link");
        symlink(&target, &data_root).unwrap();
        assert!(matches!(
            load(&camera_path, &data_root, "10"),
            Err(Error::InvalidDirectory { path }) if path == data_root
        ));

        for child in ["sessions", "logs"] {
            let data_root = directory.path().join(format!("data-{child}"));
            fs::create_dir(&data_root).unwrap();
            symlink(&target, data_root.join(child)).unwrap();

            assert!(matches!(
                load(&camera_path, &data_root, "10"),
                Err(Error::InvalidDirectory { path }) if path == data_root.join(child)
            ));
        }
    }

    #[test]
    fn rejects_zero_and_malformed_recorder_timeouts() {
        let directory = tempfile::tempdir().unwrap();
        let camera_path = write_cameras(directory.path(), &cameras());

        for timeout in ["0", "ten", "-1", "1.5"] {
            assert!(matches!(
                load(&camera_path, &directory.path().join("data"), timeout),
                Err(Error::InvalidRecorderTimeout)
            ));
        }
    }

    #[test]
    fn rejects_recorder_timeout_overflow() {
        let directory = tempfile::tempdir().unwrap();
        let camera_path = write_cameras(directory.path(), &cameras());

        assert!(matches!(
            load(
                &camera_path,
                &directory.path().join("data"),
                &u64::MAX.to_string()
            ),
            Err(Error::InvalidRecorderTimeout)
        ));
    }
}
