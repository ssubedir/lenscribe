//! Desktop-independent domain and application services, composed with replaceable adapters.

pub mod adapters;
pub mod application;
pub mod composition;
pub mod domain;
mod error;
pub mod ports;
pub mod runtime;

#[cfg(feature = "bindings")]
pub mod bindings;

// Preserve the public API consumed by Tauri, the headless binary, and integrations.
pub use adapters::{filesystem::trailer, http, llm};
pub use application::{core::Core, extraction};
pub use domain::model::*;
pub use domain::watch::{WatchEvent, WatchFailure, WatchStatus};
pub use domain::{merkle, model, settings};
pub use error::{Error, Result};
pub use runtime::daemon;
