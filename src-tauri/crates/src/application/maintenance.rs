//! Explicit index/cache maintenance; image contents are never deleted by these operations.
use std::path::Path;

use crate::{Core, Error, MaintenanceReport, MaintenanceStatus, Result, ScanIssue};

impl Core {
    pub fn maintenance_status(&self) -> Result<MaintenanceStatus> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .maintenance_status()
    }

    /// Export a consistent, checksummed logical snapshot of the canonical records.
    /// Existing destinations are never overwritten.
    pub fn backup_database(&self, destination: impl AsRef<Path>) -> Result<()> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .backup(destination.as_ref())
    }

    pub fn cleanup_cache(&self) -> Result<usize> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .cleanup_cache()
    }

    /// Re-import current trailers using normal folder rules and rebuild search.
    /// Unavailable folders keep their existing records and are reported individually.
    pub fn rebuild_index(&self) -> Result<MaintenanceReport> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .rebuild_search()?;
        let folders = self.folders()?;
        let mut result = MaintenanceReport::default();
        for folder in folders {
            match self.scan_locked(Path::new(&folder.path)) {
                Ok(report) => {
                    result.scanned_folders += 1;
                    result.changed_files += report.changed;
                    result.removed_files += report.removed;
                    result
                        .issues
                        .extend(report.issues.into_iter().map(|issue| ScanIssue {
                            path: format!("{}/{}", folder.path, issue.path),
                            error: issue.error,
                        }));
                }
                Err(error) => result.issues.push(ScanIssue {
                    path: folder.path,
                    error: error.to_string(),
                }),
            }
        }
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .rebuild_search()?;
        Ok(result)
    }
}
