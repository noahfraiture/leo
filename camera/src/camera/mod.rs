#[allow(clippy::module_inception)]
mod camera;
mod error;

pub(crate) use camera::Camera;
pub(crate) use error::Error;
