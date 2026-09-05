use std::path::PathBuf;

/// Validation or settings-storage failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidSettings(#[source] ValidationErrors),
    #[error("failed to read settings file at {path}")]
    ReadSettings {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse settings file at {path}")]
    ParseSettings { path: PathBuf },
    #[error("failed to create managed directory {path}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize settings at {path}")]
    SerializeSettings {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write settings at {path}")]
    WriteSettings {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// One invalid field or invariant in persisted application settings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("unsupported settings schema version {actual}; expected {expected}")]
    UnsupportedSchemaVersion { expected: u32, actual: u32 },
    #[error("next camera ID must be nonzero")]
    InvalidNextCameraId,
    #[error("camera at index {camera_index} has ID zero")]
    ZeroCameraId { camera_index: usize },
    #[error("camera ID {camera_id} is duplicated")]
    DuplicateCameraId { camera_id: u32 },
    #[error("camera ID {camera_id} must be below next camera ID {next_camera_id}")]
    CameraIdNotBelowNext { camera_id: u32, next_camera_id: u32 },
    #[error("camera {camera_id} has a blank name")]
    BlankCameraName { camera_id: u32 },
    #[error("camera {camera_id} has a blank RTSP URL")]
    BlankCameraUrl { camera_id: u32 },
    #[error("camera {camera_id} has an invalid RTSP URL")]
    InvalidCameraUrl { camera_id: u32 },
    #[error("recorder timeout must be positive and fit runtime limits")]
    InvalidRecorderTimeout,
    #[error(transparent)]
    Profile(#[from] backend::profiles::Error),
    #[error("next profile ID must be nonzero and greater than every allocated profile ID")]
    InvalidNextProfileId,
    #[error("enter an API key and a valid HTTP or HTTPS provider URL in Settings")]
    InvalidProvider,
    #[error("custom data root must be absolute: {path}")]
    DataRootNotAbsolute { path: PathBuf },
}

/// All validation failures found in one pass over application settings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("settings validation failed: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
pub struct ValidationErrors(pub Vec<ValidationError>);
