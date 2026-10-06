mod database;
mod ports;

use crate::{ports::index::IndexRepository, Result};
use std::path::Path;

/// Open the durable repository and recover abandoned leases.
pub fn open(path: &Path) -> Result<Box<dyn IndexRepository>> {
    Ok(Box::new(database::Database::open(path)?))
}

pub fn restore(backup: &Path, destination: &Path) -> Result<()> {
    database::restore(backup, destination)
}
