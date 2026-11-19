use crate::model::FileKind;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported file kind: {0:?}")]
    Unsupported(FileKind),

    #[error("input limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
