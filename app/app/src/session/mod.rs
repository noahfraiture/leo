//! Durable JSONL session metadata and software-only sampling controls.

mod controller;
mod error;
mod session;

pub(crate) use controller::OperatorAction;
pub(crate) use session::{Session, SessionCamera};
