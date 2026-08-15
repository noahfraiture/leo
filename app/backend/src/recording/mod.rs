//! Synology recording catalogue and media download access.

mod error;
mod synology;

pub(crate) use error::Error;
pub(crate) use synology::SynologyClient;
