use crate::domain::{model::PreparedImage, settings::ExtractionSettings};
use std::{future::Future, pin::Pin, sync::Arc};

pub type ExtractionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<String, ExtractionError>> + Send + 'a>>;

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct ExtractionError {
    pub message: String,
    pub retryable: bool,
    pub blocks_queue: bool,
    pub retry_after_seconds: Option<u64>,
}

impl ExtractionError {
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
            blocks_queue: false,
            retry_after_seconds: None,
        }
    }
    pub(crate) fn temporary(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
            blocks_queue: false,
            retry_after_seconds: None,
        }
    }
}

pub trait VisionProvider: Send + Sync {
    fn processor(&self) -> &str;
    fn extract<'a>(&'a self, image: &'a PreparedImage) -> ExtractionFuture<'a>;
}

pub trait VisionFactory: Send + Sync {
    fn create(
        &self,
        settings: ExtractionSettings,
    ) -> Result<Arc<dyn VisionProvider>, ExtractionError>;
}
