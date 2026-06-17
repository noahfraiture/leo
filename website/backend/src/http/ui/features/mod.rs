//! Page-oriented backend UI feature routes and views.

mod analyses;
mod analyze;
mod delete;
mod home;
mod upload;

// Pages
pub use analyses::{AnalysesPage, ClearAnalysesRoute, DeleteAnalysisRoute};
pub use analyze::{AnalysisStatusRoute, AnalyzeRoute};
pub use delete::DeleteVideoRoute;
pub use home::{HomePage, healthz};
pub use upload::{
    UploadVideoRoute, cancel_chunked_upload, complete_chunked_upload, start_chunked_upload,
    upload_chunk,
};
