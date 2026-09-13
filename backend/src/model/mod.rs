//! Provider configuration, structured model requests, and bounded execution planning.

mod client;
mod configuration;
mod error;
mod request;
mod schedule;

pub use client::ModelClient;
pub use configuration::{
    ModelProfile, ModelSet, ModelSnapshot, ProviderConnection, ProviderSnapshot, ResolvedModelSet,
    Transport,
};
pub use error::{Error, Result};
pub use request::{
    Completion, ImageFormat, ImageRole, JobRole, StructuredImage, StructuredJob, TokenUsage,
};
pub use schedule::{
    ExecutionPolicy, ExecutionWave, ResidencyKey, ResidentModel, ScheduledJob, plan_execution,
};
