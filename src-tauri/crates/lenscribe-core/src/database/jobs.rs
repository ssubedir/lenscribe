use rusqlite::{params, Connection, OptionalExtension};

use crate::{ExtractionJob, FileRecord, Result};

use super::rows::{file_row, FILE_COLUMNS};

pub(crate) struct ExtractionJobRepository<'a> {
    connection: &'a Connection,
}

impl<'a> ExtractionJobRepository<'a> {
    pub(super) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn queue(&self, file_id: i64, image_hash: &str, force: bool) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO extraction_jobs(file_id, image_hash, force) VALUES (?1, ?2, ?3)",
            params![file_id, image_hash, force],
        )?;
        self.connection.execute(
            "DELETE FROM extraction_failures WHERE file_id = ?1",
            [file_id],
        )?;
        Ok(())
    }

    pub fn clear(&self, file_id: i64) -> Result<()> {
        self.connection
            .execute("DELETE FROM extraction_jobs WHERE file_id = ?1", [file_id])?;
        self.connection.execute(
            "DELETE FROM extraction_failures WHERE file_id = ?1",
            [file_id],
        )?;
        Ok(())
    }

    pub fn request_id(&self, file_id: i64) -> Result<Option<i64>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM extraction_jobs WHERE file_id = ?1",
                [file_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn list(&self, folder_id: i64) -> Result<Vec<ExtractionJob>> {
        let mut query = self.connection.prepare(&format!(
            "SELECT {FILE_COLUMNS}, j.id, j.force FROM files f
             LEFT JOIN extraction_jobs j ON j.file_id = f.id AND j.image_hash = f.image_hash
             WHERE f.folder_id = ?1 AND (f.processor IS NULL OR j.id IS NOT NULL)
             ORDER BY j.id IS NULL, f.id",
        ))?;
        let jobs = query
            .query_map([folder_id], |row| {
                Ok(ExtractionJob {
                    file: file_row(row)?,
                    request_id: row.get(8)?,
                    force: row.get::<_, Option<bool>>(9)?.unwrap_or(false),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(jobs)
    }

    pub(super) fn discard_stale(&self, folder_id: i64, file: &FileRecord) -> Result<()> {
        self.connection.execute(
            "DELETE FROM extraction_jobs
             WHERE file_id = (SELECT id FROM files WHERE folder_id = ?1 AND relative_path = ?2)
                AND image_hash != ?3",
            params![folder_id, file.relative_path, file.image_hash],
        )?;
        Ok(())
    }
}
