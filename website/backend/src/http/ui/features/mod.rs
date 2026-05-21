mod analyses;
mod analyze;
mod delete;
mod home;
mod upload;

// Pages
pub use analyses::AnalysesPage;
pub use analyze::{AnalysisStatusRoute, AnalyzeRoute, spawn_analysis_job};
pub use delete::DeleteVideoRoute;
pub use home::{HomePage, healthz};
pub use upload::{
    ChunkedUploadStore, UploadVideoRoute, cancel_chunked_upload, complete_chunked_upload,
    start_chunked_upload, upload_chunk,
};
