//! Named model connections and bounded execution planning.
mod configuration;
mod error;
mod schedule;
pub use configuration::{ModelProfile, ModelSet, ModelSnapshot, ProviderConnection, ProviderSnapshot, ResolvedModelSet, Transport};
pub use error::{Error, Result};
pub use schedule::{ExecutionPolicy, ExecutionWave, ResidencyKey, ResidentModel, ScheduledJob, plan_execution};
