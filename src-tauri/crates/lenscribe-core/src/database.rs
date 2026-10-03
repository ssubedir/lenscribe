//! SQLite connection ownership and transactions spanning multiple repositories.

mod cache;
mod files;
mod folders;
mod jobs;
mod recovery;
mod rows;
mod search;

#[cfg(test)]
mod tests;

use std::{path::Path, time::Duration};

use rusqlite::Connection;

use crate::{FileDetails, Result};

use cache::ExtractionCacheRepository;
use files::FileRepository;
use folders::FolderRepository;
use jobs::ExtractionJobRepository;
pub(crate) use recovery::EndpointState;
use recovery::RecoveryRepository;
use search::SearchRepository;

pub(crate) struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        crate::migrations::apply(&connection)?;
        Ok(Self { connection })
    }

    pub fn folders(&self) -> FolderRepository<'_> {
        FolderRepository::new(&self.connection)
    }

    pub fn files(&self) -> FileRepository<'_> {
        FileRepository::new(&self.connection)
    }

    pub fn cache(&self) -> ExtractionCacheRepository<'_> {
        ExtractionCacheRepository::new(&self.connection)
    }

    pub fn jobs(&self) -> ExtractionJobRepository<'_> {
        ExtractionJobRepository::new(&self.connection)
    }

    pub fn recovery(&self) -> RecoveryRepository<'_> {
        RecoveryRepository::new(&self.connection)
    }

    pub fn search(&self) -> SearchRepository<'_> {
        SearchRepository::new(&self.connection)
    }

    /// Commit the index, cache, stale-job cleanup, and Merkle root together.
    pub fn apply_changes(
        &mut self,
        folder_id: i64,
        changed: &[FileDetails],
        removed: &[String],
        root_hash: &str,
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let files = FileRepository::new(&transaction);
        let jobs = ExtractionJobRepository::new(&transaction);
        let recovery = RecoveryRepository::new(&transaction);
        let cache = ExtractionCacheRepository::new(&transaction);

        for details in changed {
            let file = &details.file;
            files.upsert(folder_id, details)?;
            jobs.discard_stale(folder_id, file)?;
            recovery.discard_stale(folder_id, file)?;
            if let (Some(processor), Some(text)) = (&file.processor, &details.text) {
                cache.store(&file.image_hash, processor, text)?;
            }
        }
        for path in removed {
            files.remove(folder_id, path)?;
        }
        FolderRepository::new(&transaction).update_root(folder_id, root_hash)?;
        transaction.commit()?;
        Ok(())
    }
}
