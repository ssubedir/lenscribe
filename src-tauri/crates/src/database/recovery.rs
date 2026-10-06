use super::{store::Change, Database};
use crate::{ExtractionJob, FileRecord, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct EndpointState {
    pub retry_at_ms: Option<u64>,
    pub next_request_ms: u64,
    pub blocked_error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct FailureState {
    pub file_id: i64,
    pub recovery_id: String,
    pub image_hash: String,
    pub request_id: Option<i64>,
    pub error: String,
    pub attempts: u32,
    pub retry_at_ms: Option<u64>,
}
pub(crate) struct StoredFailure {
    pub file: FileRecord,
    pub request_id: Option<i64>,
    pub error: String,
    pub attempts: u32,
    pub retry_at_ms: Option<u64>,
}
pub(super) fn failure_key(recovery: &str, file: i64) -> String {
    format!("failures/{file:020}/{recovery}")
}
pub(crate) struct RecoveryRepository<'a> {
    database: &'a Database,
}
impl<'a> RecoveryRepository<'a> {
    pub(super) fn new(database: &'a Database) -> Self {
        Self { database }
    }
    pub fn load(&self, recovery: &str) -> Result<(Vec<StoredFailure>, EndpointState)> {
        let state = self.database.state.borrow();
        let mut failures = Vec::new();
        for failure in state.failures.values() {
            if failure.recovery_id != recovery {
                continue;
            }
            let Some(file) = state.files.get(&failure.file_id) else {
                continue;
            };
            let job = state.jobs.get(&file.id);
            if failure.image_hash != file.image_hash
                || failure.request_id != job.and_then(|job| job.id)
                || (file.processor.is_some() && job.is_none())
            {
                continue;
            }
            failures.push(StoredFailure {
                file: file.clone(),
                request_id: failure.request_id,
                error: failure.error.clone(),
                attempts: failure.attempts,
                retry_at_ms: failure.retry_at_ms,
            });
        }
        Ok((
            failures,
            self.database
                .store
                .get(&format!("endpoints/{recovery}"))?
                .unwrap_or_default(),
        ))
    }
    pub fn save_endpoint(&self, recovery: &str, state: &EndpointState) -> Result<()> {
        self.database
            .commit(vec![Change::put(format!("endpoints/{recovery}"), state)?])
    }
    pub fn save_failure(
        &self,
        recovery: &str,
        job: &ExtractionJob,
        error: &str,
        attempts: u32,
        retry: Option<u64>,
        endpoint: &EndpointState,
    ) -> Result<bool> {
        if self
            .database
            .state
            .borrow()
            .files
            .get(&job.file.id)
            .is_none_or(|file| file.record_hash != job.file.record_hash)
            || self.database.jobs().request_id(job.file.id)? != job.request_id
        {
            return Ok(false);
        }
        let failure = FailureState {
            file_id: job.file.id,
            recovery_id: recovery.into(),
            image_hash: job.file.image_hash.clone(),
            request_id: job.request_id,
            error: error.into(),
            attempts,
            retry_at_ms: retry,
        };
        self.database.commit(vec![
            Change::put(failure_key(recovery, job.file.id), &failure)?,
            Change::put(format!("endpoints/{recovery}"), endpoint)?,
        ])?;
        Ok(true)
    }
    pub fn clear(&self, recovery: &str) -> Result<()> {
        let mut changes = self
            .database
            .store
            .scan::<FailureState>("failures/")?
            .into_iter()
            .filter(|(_, f)| f.recovery_id == recovery)
            .map(|(key, _)| Change::remove(key))
            .collect::<Vec<_>>();
        let mut endpoint = self
            .database
            .store
            .get::<EndpointState>(&format!("endpoints/{recovery}"))?
            .unwrap_or_default();
        endpoint.retry_at_ms = None;
        endpoint.blocked_error = None;
        changes.push(Change::put(format!("endpoints/{recovery}"), &endpoint)?);
        self.database.commit(changes)
    }
    pub(super) fn discard_changes(
        &self,
        file: i64,
        image: &str,
        processed: bool,
    ) -> Result<Vec<Change>> {
        Ok(self
            .database
            .store
            .scan::<FailureState>(&format!("failures/{file:020}/"))?
            .into_iter()
            .filter(|(_, failure)| processed || failure.image_hash != image)
            .map(|(key, _)| Change::remove(key))
            .collect())
    }
}
