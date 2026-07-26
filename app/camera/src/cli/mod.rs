#[allow(clippy::module_inception)]
mod cli;

pub(crate) use cli::{Args, parse_args};
