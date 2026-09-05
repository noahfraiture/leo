/// Invalid profile definitions or references, reported before the affected operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("profile {id}: {reason}")]
    Invalid { id: u32, reason: &'static str },
    #[error("monitoring profile {id} does not exist")]
    UnknownMonitoring { id: u32 },
    #[error("analysis profile {id} does not exist")]
    UnknownAnalysis { id: u32 },
}
