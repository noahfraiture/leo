use uuid::Uuid;

/// Configuration errors describe storage or editing failures, never candidate performance.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("exercise storage I/O failed")]
    Io(#[from] std::io::Error),
    #[error("exercise data is not valid JSON")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Model(#[from] crate::model::Error),
    #[error("exercise data is invalid: {0}")]
    Invalid(String),
    #[error("exercise {0} does not exist")]
    NotFound(Uuid),
    #[error("the exercise was changed elsewhere; reload before saving")]
    StaleDraft,
    #[error("the checklist changed while the proposal was being prepared")]
    StaleProposal,
    #[error("reference imports support only decoded JPEG and PNG images")]
    UnsupportedImage,
    #[error("reference image exceeds the file-size or decoded-pixel limit")]
    ImageTooLarge,
    #[error("reference image could not be decoded")]
    DecodeImage(#[from] image::ImageError),
    #[error("reference image {0} is missing, changed, or has inconsistent metadata")]
    ChangedAsset(Uuid),
    #[error("stored content must be a direct regular file or directory")]
    InvalidStorageEntry,
    #[error("exercise revision counter exhausted")]
    RevisionOverflow,
}

pub type Result<T> = std::result::Result<T, Error>;
