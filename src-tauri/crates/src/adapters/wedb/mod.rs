mod database;
#[cfg(all(test, feature = "legacy-sqlite"))]
pub(crate) mod migrations;
mod ports;

use crate::{ports::index::IndexRepository, Result};
use std::path::Path;

/// Open the durable repository, including legacy import and abandoned-lease recovery.
pub fn open(path: &Path) -> Result<Box<dyn IndexRepository>> {
    Ok(Box::new(database::Database::open(path)?))
}

pub fn restore(backup: &Path, destination: &Path) -> Result<()> {
    database::restore(backup, destination)
}

#[cfg(feature = "legacy-sqlite")]
impl From<rusqlite::Error> for crate::Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}
