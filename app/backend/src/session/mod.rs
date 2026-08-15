//! Durable JSONL session metadata and software-only sampling controls.

mod controller;
mod error;
mod session;

pub use controller::{OperatorAction, SessionController};
pub use error::Error;
pub use session::{Session, SessionCamera};
