use std::io;

pub type Result<T> = std::result::Result<T, HostableError>;

#[derive(Debug, thiserror::Error)]
pub enum HostableError {
    #[error("unsupported adapter operation: {0}")]
    Unsupported(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}
