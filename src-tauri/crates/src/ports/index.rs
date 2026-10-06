//! Repository operations preserve application transactions, not storage-engine primitives.
use crate::{
    domain::{merkle::MerkleTree, model::*},
    Result,
};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};

pub trait Catalog {
    fn ensure_folder(&self, path: &str) -> Result<i64>;
    fn folder(&self, id: i64) -> Result<FolderRecord>;
    fn folders(&self) -> Result<Vec<FolderRecord>>;
    fn folder_progress(&self) -> Result<Vec<FolderProgress>>;
    fn snapshot(&self, id: i64) -> Result<FolderSnapshot>;
    fn file(&self, id: i64) -> Result<FileDetails>;
    fn file_by_path(&self, folder: i64, path: &str) -> Result<FileDetails>;
    fn find_files(&self, folder: i64, query: &str, offset: usize, fuzzy: bool) -> Result<FilePage>;
    fn search(&self, query: &str, folder: Option<i64>, limit: usize) -> Result<Vec<SearchHit>>;
    fn search_page(
        &self,
        query: &str,
        folder: Option<i64>,
        offset: usize,
        limit: usize,
        fuzzy: bool,
    ) -> Result<SearchPage>;
    fn merkle_checkpoint(
        &self,
        folder: i64,
        records: &[FileRecord],
        root: &str,
    ) -> Result<Option<MerkleTree>>;
    /// Atomically commit records, text/cache pointers, queue cleanup, and Merkle state.
    fn apply_scan(
        &mut self,
        folder: i64,
        changed: &[FileDetails],
        removed: &[String],
        root: &str,
        tree: Option<&MerkleTree>,
    ) -> Result<()>;
    fn cached_extraction(&self, image: &str, processor: &str) -> Result<Option<String>>;
}

pub trait ExtractionQueue {
    fn queue_file(&self, file: i64, image: &str, force: bool) -> Result<()>;
    fn clear_job(&self, file: i64) -> Result<()>;
    fn request_id(&self, file: i64) -> Result<Option<i64>>;
    fn extraction_jobs(&self, folder: i64) -> Result<Vec<ExtractionJob>>;
    fn ready_jobs(
        &self,
        folders: &[i64],
        recovery: &str,
        now: u64,
        active: &[i64],
        limit: usize,
    ) -> Result<Vec<ExtractionJob>>;
    fn next_due(&self, folders: &[i64], recovery: &str, active: &[i64]) -> Result<Option<u64>>;
    fn claim_job(&self, job: &ExtractionJob, owner: &str, now: u64, until: u64) -> Result<bool>;
    fn release_job(&self, job: &ExtractionJob) -> Result<()>;
    fn recover_leases(&self) -> Result<()>;
    fn defer_job(&self, job: &ExtractionJob, until: u64) -> Result<()>;
    fn saved_text(&self, job: &ExtractionJob, processor: &str) -> Result<Option<String>>;
    /// Sync a successful response before the application writes the image trailer.
    fn save_result(&self, job: &ExtractionJob, text: &str, processor: &str) -> Result<()>;
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct EndpointState {
    pub retry_at_ms: Option<u64>,
    pub next_request_ms: u64,
    pub blocked_error: Option<String>,
}

pub struct StoredFailure {
    pub file: FileRecord,
    pub request_id: Option<i64>,
    pub error: String,
    pub attempts: u32,
    pub retry_at_ms: Option<u64>,
}

pub trait ExtractionRecovery {
    fn recovery_state(&self, identity: &str) -> Result<(Vec<StoredFailure>, EndpointState)>;
    fn save_endpoint(&self, identity: &str, state: &EndpointState) -> Result<()>;
    /// Failure and endpoint backoff must commit together, respecting the job generation.
    fn save_failure(
        &self,
        identity: &str,
        job: &ExtractionJob,
        error: &str,
        attempts: u32,
        retry: Option<u64>,
        endpoint: &EndpointState,
    ) -> Result<bool>;
    fn clear_failures(&self, identity: &str) -> Result<()>;
}

pub trait IndexMaintenance {
    fn maintenance_status(&self) -> Result<MaintenanceStatus>;
    fn backup(&self, destination: &Path) -> Result<()>;
    fn cleanup_cache(&self) -> Result<usize>;
    fn rebuild_search(&self) -> Result<()>;
    fn persist(&self) -> Result<()>;
}

/// A single owner and serialization lock cover all repository capabilities.
/// Splitting storage instances would break scan and recovery atomicity.
pub trait IndexRepository:
    Catalog + ExtractionQueue + ExtractionRecovery + IndexMaintenance + Send
{
    fn change_notification(&self) -> Arc<tokio::sync::Notify>;
}
