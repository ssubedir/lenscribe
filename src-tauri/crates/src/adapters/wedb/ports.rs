//! Map application repository contracts onto the existing atomic WeDB repositories.
use super::database::Database;
use crate::{
    domain::{merkle::MerkleTree, model::*},
    ports::index::*,
    Result,
};
use std::{path::Path, sync::Arc};

impl Catalog for Database {
    fn ensure_folder(&self, path: &str) -> Result<i64> {
        self.folders().ensure(path)
    }
    fn folder(&self, id: i64) -> Result<FolderRecord> {
        self.folders().get(id)
    }
    fn folders(&self) -> Result<Vec<FolderRecord>> {
        self.folders().list()
    }
    fn folder_progress(&self) -> Result<Vec<FolderProgress>> {
        self.folders().progress()
    }
    fn snapshot(&self, id: i64) -> Result<FolderSnapshot> {
        self.folders().snapshot(id)
    }
    fn file(&self, id: i64) -> Result<FileDetails> {
        self.files().get(id)
    }
    fn file_by_path(&self, folder: i64, path: &str) -> Result<FileDetails> {
        self.files().by_path(folder, path)
    }
    fn find_files(&self, folder: i64, query: &str, offset: usize, fuzzy: bool) -> Result<FilePage> {
        self.files().list(folder, query, offset, fuzzy)
    }
    fn search(&self, query: &str, folder: Option<i64>, limit: usize) -> Result<Vec<SearchHit>> {
        self.search().query(query, folder, limit)
    }
    fn search_page(
        &self,
        query: &str,
        folder: Option<i64>,
        offset: usize,
        limit: usize,
        fuzzy: bool,
    ) -> Result<SearchPage> {
        self.search().page(query, folder, offset, limit, fuzzy)
    }
    fn merkle_checkpoint(
        &self,
        folder: i64,
        records: &[FileRecord],
        root: &str,
    ) -> Result<Option<MerkleTree>> {
        Database::merkle_checkpoint(self, folder, records, root)
    }
    fn apply_scan(
        &mut self,
        folder: i64,
        changed: &[FileDetails],
        removed: &[String],
        root: &str,
        tree: Option<&MerkleTree>,
    ) -> Result<()> {
        Database::apply_scan(self, folder, changed, removed, root, tree)
    }
    fn cached_extraction(&self, image: &str, processor: &str) -> Result<Option<String>> {
        self.cache().get(image, processor)
    }
}

impl ExtractionQueue for Database {
    fn queue_file(&self, file: i64, image: &str, force: bool) -> Result<()> {
        self.jobs().queue(file, image, force)
    }
    fn clear_job(&self, file: i64) -> Result<()> {
        self.jobs().clear(file)
    }
    fn request_id(&self, file: i64) -> Result<Option<i64>> {
        self.jobs().request_id(file)
    }
    fn extraction_jobs(&self, folder: i64) -> Result<Vec<ExtractionJob>> {
        self.jobs().list(folder)
    }
    fn ready_jobs(
        &self,
        folders: &[i64],
        recovery: &str,
        now: u64,
        active: &[i64],
        limit: usize,
    ) -> Result<Vec<ExtractionJob>> {
        self.jobs().ready(folders, recovery, now, active, limit)
    }
    fn next_due(&self, folders: &[i64], recovery: &str, active: &[i64]) -> Result<Option<u64>> {
        self.jobs().next_due(folders, recovery, active)
    }
    fn claim_job(&self, job: &ExtractionJob, owner: &str, now: u64, until: u64) -> Result<bool> {
        self.jobs().claim(job, owner, now, until)
    }
    fn release_job(&self, job: &ExtractionJob) -> Result<()> {
        self.jobs().release(job)
    }
    fn recover_leases(&self) -> Result<()> {
        self.jobs().recover_leases()
    }
    fn defer_job(&self, job: &ExtractionJob, until: u64) -> Result<()> {
        self.jobs().defer(job, until)
    }
    fn saved_text(&self, job: &ExtractionJob, processor: &str) -> Result<Option<String>> {
        self.jobs().saved_text(job, processor)
    }
    fn save_result(&self, job: &ExtractionJob, text: &str, processor: &str) -> Result<()> {
        self.jobs().save_result(job, text, processor)
    }
}

impl ExtractionRecovery for Database {
    fn recovery_state(&self, identity: &str) -> Result<(Vec<StoredFailure>, EndpointState)> {
        self.recovery().load(identity)
    }
    fn save_endpoint(&self, identity: &str, state: &EndpointState) -> Result<()> {
        self.recovery().save_endpoint(identity, state)
    }
    fn save_failure(
        &self,
        identity: &str,
        job: &ExtractionJob,
        error: &str,
        attempts: u32,
        retry: Option<u64>,
        endpoint: &EndpointState,
    ) -> Result<bool> {
        self.recovery()
            .save_failure(identity, job, error, attempts, retry, endpoint)
    }
    fn clear_failures(&self, identity: &str) -> Result<()> {
        self.recovery().clear(identity)
    }
}

impl IndexMaintenance for Database {
    fn maintenance_status(&self) -> Result<MaintenanceStatus> {
        self.maintenance().status()
    }
    fn backup(&self, destination: &Path) -> Result<()> {
        self.maintenance().backup(destination)
    }
    fn cleanup_cache(&self) -> Result<usize> {
        self.maintenance().cleanup_cache()
    }
    fn rebuild_search(&self) -> Result<()> {
        self.search().rebuild()
    }
    fn persist(&self) -> Result<()> {
        Database::persist(self)
    }
}

impl IndexRepository for Database {
    fn change_notification(&self) -> Arc<tokio::sync::Notify> {
        self.wake.clone()
    }
}
