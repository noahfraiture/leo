mod analyses;
mod analyze;
mod delete;
mod home;
mod upload;

// Pages
pub use analyses::AnalysesPage;
pub use analyze::{AnalysisStatusRoute, AnalyzeRoute};
pub use delete::DeleteVideoRoute;
pub use home::{HomePage, healthz};
pub use upload::UploadVideoRoute;
