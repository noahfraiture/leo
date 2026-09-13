//! Operator-facing recording and completed-session state and orchestration.

mod error;
mod recording;
mod state;

pub use error::Error;
pub use recording::{
    handle_recorder_event, handle_recorder_event_channel_closed, set_monitoring_profile,
    set_participation, spawn_fault_cleanup, spawn_start_session, spawn_stop_session,
};
pub use state::{
    FaultSessionRequest, OperatorState, SessionRunState, StartSessionRequest, StopSessionRequest,
};
