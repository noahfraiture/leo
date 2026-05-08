mod analyze;
mod delete;
mod home;
mod upload;

// Pages
pub use analyze::AnalyzeRoute;
pub use delete::DeleteVideoRoute;
pub use home::{HomePage, healthz};
pub use upload::UploadVideoRoute;
