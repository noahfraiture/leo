//! Persistent application configuration and its validated runtime representation.
//!
//! Settings are loaded against the current strict schema: a missing file starts first-run setup,
//! while malformed or invalid content stops runtime startup. Saving writes the validated draft;
//! operational changes take effect after restart.

mod error;
mod model;
mod store;

pub use error::{Error, ValidationError, ValidationErrors};
pub use model::{CameraSettings, LogLevel, OpenAiSettings, Settings};
pub use store::{ResolvedSettings, SettingsStore};
