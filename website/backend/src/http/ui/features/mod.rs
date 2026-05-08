mod analyze;
mod home;
mod upload;

// Pages
pub use analyze::AnalyzeRoute;
pub use home::{HomePage, healthz};
pub use upload::UploadVideoRoute;
