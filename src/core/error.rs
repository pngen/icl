use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug, Clone)]
pub enum IclError {
    #[error("Asset {0} not found")]
    AssetNotFound(Uuid),

    #[error("Asset {0} already exists")]
    AssetAlreadyExists(Uuid),

    #[error("Invalid asset: {0}")]
    InvalidAsset(String),

    #[error("Invalid event: {0}")]
    InvalidEvent(String),

    #[error("Invalid entry: {0}")]
    InvalidEntry(String),

    #[error("Depreciation error: {0}")]
    DepreciationError(String),

    #[error("Integrity violation: {0}")]
    IntegrityViolation(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Integration error: {0}")]
    IntegrationError(String),

    #[error("Invalid date range: start {start} must be before end {end}")]
    InvalidDateRange { start: String, end: String },

    #[error("Overlapping depreciation period detected")]
    OverlappingDepreciation,

    #[error("Asset {0} is retired and cannot be modified")]
    AssetRetired(Uuid),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
}

pub type IclResult<T> = Result<T, IclError>;

impl From<serde_json::Error> for IclError {
    fn from(e: serde_json::Error) -> Self {
        IclError::SerializationError(e.to_string())
    }
}