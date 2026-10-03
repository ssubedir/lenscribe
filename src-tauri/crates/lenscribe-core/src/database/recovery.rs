use rusqlite::{params, Connection, OptionalExtension};

use crate::{ExtractionJob, FileRecord, Result};

use super::{
    files::FileRepository,
    jobs::ExtractionJobRepository,
    rows::{file_row, FILE_COLUMNS},
};

#[derive(Clone, Default)]
pub(crate) struct EndpointState {
    pub retry_at_ms: Option<u64>,
    pub next_request_ms: u64,
    pub blocked_error: Option<String>,
}

pub(crate) struct StoredFailure {
    pub file: FileRecord,
    pub request_id: Option<i64>,
    pub error: String,
    pub attempts: u32,
    pub retry_at_ms: Option<u64>,
}

pub(crate) struct RecoveryRepository<'a> {
    connection: &'a Connection,
}

impl<'a> RecoveryRepository<'a> {
    pub(super) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn load(&self, recovery_id: &str) -> Result<(Vec<StoredFailure>, EndpointState)> {
        let mut query = self.connection.prepare(&format!(
            "SELECT {FILE_COLUMNS}, r.request_id, r.error, r.attempts, r.retry_at_ms
             FROM extraction_failures r JOIN files f ON f.id = r.file_id
             LEFT JOIN extraction_jobs j ON j.file_id = f.id
             WHERE r.recovery_id = ?1 AND r.image_hash = f.image_hash
                AND r.request_id IS j.id AND (f.processor IS NULL OR j.id IS NOT NULL)",
        ))?;
        let failures = query
            .query_map([recovery_id], |row| {
                Ok(StoredFailure {
                    file: file_row(row)?,
                    request_id: row.get(8)?,
                    error: row.get(9)?,
                    attempts: row.get(10)?,
                    retry_at_ms: row.get::<_, Option<i64>>(11)?.map(|at| at.max(0) as u64),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let endpoint = self
            .connection
            .query_row(
                "SELECT retry_at_ms, next_request_ms, blocked_error
                 FROM extraction_endpoints WHERE recovery_id = ?1",
                [recovery_id],
                |row| {
                    Ok(EndpointState {
                        retry_at_ms: row.get::<_, Option<i64>>(0)?.map(|at| at.max(0) as u64),
                        next_request_ms: row.get::<_, i64>(1)?.max(0) as u64,
                        blocked_error: row.get(2)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default();
        Ok((failures, endpoint))
    }

    pub fn save_endpoint(&self, recovery_id: &str, state: &EndpointState) -> Result<()> {
        self.connection.execute(
            "INSERT INTO extraction_endpoints(recovery_id, retry_at_ms, next_request_ms, blocked_error)
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT(recovery_id) DO UPDATE SET
                retry_at_ms = excluded.retry_at_ms, next_request_ms = excluded.next_request_ms,
                blocked_error = excluded.blocked_error",
            params![
                recovery_id,
                state.retry_at_ms.map(sqlite_timestamp),
                sqlite_timestamp(state.next_request_ms),
                state.blocked_error
            ],
        )?;
        Ok(())
    }

    pub fn save_failure(
        &self,
        recovery_id: &str,
        job: &ExtractionJob,
        error: &str,
        attempts: u32,
        retry_at_ms: Option<u64>,
        endpoint: &EndpointState,
    ) -> Result<bool> {
        let Some(current) = FileRepository::new(self.connection).find_record(job.file.id)? else {
            return Ok(false);
        };
        if current.record_hash != job.file.record_hash
            || ExtractionJobRepository::new(self.connection).request_id(job.file.id)?
                != job.request_id
        {
            return Ok(false);
        }
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO extraction_failures(file_id, recovery_id, image_hash, request_id,
                error, attempts, retry_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(file_id, recovery_id) DO UPDATE SET
                image_hash = excluded.image_hash, request_id = excluded.request_id,
                error = excluded.error, attempts = excluded.attempts, retry_at_ms = excluded.retry_at_ms",
            params![
                job.file.id,
                recovery_id,
                job.file.image_hash,
                job.request_id,
                error,
                attempts,
                retry_at_ms.map(sqlite_timestamp)
            ],
        )?;
        RecoveryRepository::new(&transaction).save_endpoint(recovery_id, endpoint)?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn clear(&self, recovery_id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM extraction_failures WHERE recovery_id = ?1",
            [recovery_id],
        )?;
        // Keep request pacing on explicit retry, while clearing endpoint failures/backoff.
        self.connection.execute(
            "UPDATE extraction_endpoints SET retry_at_ms = NULL, blocked_error = NULL
             WHERE recovery_id = ?1",
            [recovery_id],
        )?;
        Ok(())
    }

    pub(super) fn discard_stale(&self, folder_id: i64, file: &FileRecord) -> Result<()> {
        self.connection.execute(
            "DELETE FROM extraction_failures
             WHERE file_id = (SELECT id FROM files WHERE folder_id = ?1 AND relative_path = ?2)
                AND (image_hash != ?3 OR ?4 IS NOT NULL)",
            params![
                folder_id,
                file.relative_path,
                file.image_hash,
                file.processor
            ],
        )?;
        Ok(())
    }
}

fn sqlite_timestamp(at: u64) -> i64 {
    at.min(i64::MAX as u64) as i64
}
