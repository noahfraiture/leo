//! Durable JSONL session timelines, completion discovery, and software-only sampling controls.

mod catalog;
mod controller;
mod error;
mod event_log;

pub use catalog::{SessionCatalog, StoredSession, list_sessions, mark_recording_complete};
pub use controller::{OperatorAction, SessionController};
pub use error::Error;
pub use event_log::{Session, SessionCamera};
