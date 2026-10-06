use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("database: {0}")]
    #[cfg(feature = "legacy-sqlite")]
    Database(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("folder watcher: {0}")]
    Watch(String),
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

impl Error {
    pub(crate) fn is_transient_read(&self) -> bool {
        match self {
            Self::ImageChanged => true,
            Self::Io(error) => {
                matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::UnexpectedEof
                ) || (cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)))
            }
            _ => false,
        }
    }
}
