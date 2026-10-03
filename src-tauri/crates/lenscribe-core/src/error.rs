use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("folder watcher: {0}")]
    Watch(#[from] notify::Error),
    #[error("invalid Lenscribe trailer: {0}")]
    InvalidTrailer(String),
    #[error("only PNG, JPEG, and WebP images are supported: {0}")]
    UnsupportedImage(PathBuf),
    #[error("the image changed before its extracted text could be saved")]
    ImageChanged,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal lock was poisoned")]
    Poisoned,
}

pub type Result<T> = std::result::Result<T, Error>;
