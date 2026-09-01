use std::path::PathBuf;

/// Platform resolution, validation, or durable settings-storage failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("standard {category} directory is unavailable")]
    StandardDirectoryUnavailable { category: &'static str },
    #[error("{category} path must be absolute: {path}")]
    NonAbsolutePath {
        category: &'static str,
        path: PathBuf,
    },
    #[error("settings are invalid")]
    InvalidSettings(#[source] ValidationErrors),
    #[error("settings path has no parent directory: {path}")]
    SettingsPathWithoutParent { path: PathBuf },
    #[error("failed to inspect settings file at {path}")]
    InspectSettingsFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("settings file is not a direct regular file: {path}")]
    InvalidSettingsFile { path: PathBuf },
    #[error("failed to read settings file at {path}")]
    ReadSettings {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse settings file at {path}")]
    ParseSettings { path: PathBuf },
    #[error("managed path is not a direct directory: {path}")]
    InvalidDirectory { path: PathBuf },
    #[error("failed to inspect managed directory {path}")]
    InspectDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create managed directory {path}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to probe managed directory {path}")]
    ProbeDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create a temporary settings file beside {path}")]
    CreateTemporarySettings {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[cfg(unix)]
    #[error("failed to make the temporary settings file private at {path}")]
    SetTemporaryPermissions {
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
    #[error("failed to write temporary settings beside {path}")]
    WriteTemporarySettings {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to replace settings file at {path}")]
    ReplaceSettings {
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
    #[error("camera IDs are exhausted")]
    CameraIdExhausted,
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
    #[error("camera {camera_id} sampling cadence must be positive whole seconds")]
    InvalidSamplingCadence { camera_id: u32 },
    #[error("recorder timeout must be positive and fit runtime limits")]
    InvalidRecorderTimeout,
    #[error("custom data root must be absolute: {path}")]
    DataRootNotAbsolute { path: PathBuf },
    #[error("OpenAI base URL must be an absolute HTTP or HTTPS URL")]
    InvalidOpenAiBaseUrl,
}

/// All validation failures found in one pass over application settings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("settings validation failed")]
pub struct ValidationErrors(pub Vec<ValidationError>);
