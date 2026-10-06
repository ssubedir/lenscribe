use super::{
    store::{id_key, Change},
    Database,
};
use crate::{Error, ExtractionJob, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
type Priority = (bool, i64, i64);
/// Rebuildable scheduling projection. Time advances move due entries into an
/// ordered ready set; claiming a bounded batch never loads the whole queue.
#[derive(Default)]
pub(super) struct QueueIndex {
    future: BTreeSet<(u64, Priority)>,
    ready: BTreeSet<Priority>,
    entries: BTreeMap<i64, (u64, Priority)>,
    now: u64,
}
impl QueueIndex {
    fn new(state: &super::store::State, recovery: &str) -> Self {
        let mut queue = Self::default();
        for id in state.files.keys() {
            queue.update(state, recovery, *id);
        }
        queue
    }
    pub fn update(&mut self, state: &super::store::State, recovery: &str, id: i64) {
        if let Some((due, priority)) = self.entries.remove(&id) {
            self.future.remove(&(due, priority));
            self.ready.remove(&priority);
        }
        let Some(file) = state.files.get(&id) else {
            return;
        };
        let job = state
            .jobs
            .get(&id)
            .filter(|job| job.image_hash == file.image_hash);
        if file.processor.is_some() && job.is_none() {
            return;
        }
        let request = job.and_then(|job| job.id);
        let mut due = job.map_or(0, |job| job.ready_at_ms.max(job.lease_until_ms));
        if let Some(failure) = state
            .failures
            .get(&(recovery.into(), id))
            .filter(|failure| {
                failure.image_hash == file.image_hash && failure.request_id == request
            })
        {
            let Some(retry) = failure.retry_at_ms else {
                return;
            };
            due = due.max(retry);
        }
        let priority = (request.is_none(), request.unwrap_or(id), id);
        self.entries.insert(id, (due, priority));
        if due <= self.now {
            self.ready.insert(priority);
        } else {
            self.future.insert((due, priority));
        }
    }
    fn advance(&mut self, now: u64) {
        if now < self.now {
            // A wall-clock correction must not make deferred work run early.
            self.ready.retain(|priority| {
                let due = self.entries[&priority.2].0;
                if due > now {
                    self.future.insert((due, *priority));
                    false
                } else {
                    true
                }
            });
        }
        self.now = now;
        while self.future.first().is_some_and(|(due, _)| *due <= now) {
            let (_, priority) = self.future.pop_first().unwrap();
            self.ready.insert(priority);
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct JobState {
    pub id: Option<i64>,
    pub file_id: i64,
    pub image_hash: String,
    pub force: bool,
    pub ready_at_ms: u64,
    #[serde(default)]
    pub lease_owner: Option<String>,
    #[serde(default)]
    pub lease_until_ms: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SavedResult {
    pub file_id: i64,
    pub image_hash: String,
    pub record_hash: String,
    pub request_id: Option<i64>,
    pub processor: String,
    pub text_hash: String,
}
pub(crate) struct ExtractionJobRepository<'a> {
    database: &'a Database,
}
impl<'a> ExtractionJobRepository<'a> {
    pub(super) fn new(database: &'a Database) -> Self {
        Self { database }
    }
    pub fn queue(&self, file_id: i64, image: &str, force: bool) -> Result<()> {
        let file = self.database.files().get(file_id)?.file;
        if file.image_hash != image {
            return Err(Error::ImageChanged);
        }
        let id = self.next_id()?;
        let job = JobState {
            id: Some(id),
            file_id,
            image_hash: image.into(),
            force,
            ready_at_ms: 0,
            lease_owner: None,
            lease_until_ms: 0,
        };
        let mut changes = vec![
            Change::put(id_key("jobs", file_id), &job)?,
            Change::put("seq/job", &id)?,
            Change::remove(id_key("results", file_id)),
        ];
        changes.extend(
            self.database
                .recovery()
                .discard_changes(file_id, "", true)?,
        );
        self.database.commit(changes)
    }
    fn next_id(&self) -> Result<i64> {
        self.database
            .store
            .get::<i64>("seq/job")?
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| Error::Storage("job IDs exhausted".into()))
    }
    pub fn clear(&self, file: i64) -> Result<()> {
        let mut changes = vec![
            Change::remove(id_key("jobs", file)),
            Change::remove(id_key("results", file)),
        ];
        changes.extend(self.database.recovery().discard_changes(file, "", true)?);
        self.database.commit(changes)
    }
    pub fn request_id(&self, file: i64) -> Result<Option<i64>> {
        Ok(self
            .database
            .state
            .borrow()
            .jobs
            .get(&file)
            .and_then(|job| job.id))
    }
    pub fn list(&self, folder: i64) -> Result<Vec<ExtractionJob>> {
        let state = self.database.state.borrow();
        let mut jobs = state
            .files
            .values()
            .filter(|file| file.folder_id == folder)
            .filter_map(|file| {
                let job = state
                    .jobs
                    .get(&file.id)
                    .filter(|job| job.image_hash == file.image_hash);
                (file.processor.is_none() || job.is_some()).then(|| ExtractionJob {
                    file: file.clone(),
                    request_id: job.and_then(|job| job.id),
                    force: job.is_some_and(|job| job.force),
                })
            })
            .collect::<Vec<_>>();
        jobs.sort_by_key(|job| {
            (
                job.request_id.is_none(),
                job.request_id.unwrap_or(job.file.id),
                job.file.id,
            )
        });
        Ok(jobs)
    }
    pub fn ready(
        &self,
        folders: &[i64],
        recovery: &str,
        now: u64,
        active: &[i64],
        limit: usize,
    ) -> Result<Vec<ExtractionJob>> {
        if limit == 0 || folders.is_empty() {
            return Ok(vec![]);
        }
        let state = self.database.state.borrow();
        let mut queues = self.database.queues.borrow_mut();
        queues.retain(|id, _| id == recovery);
        let queue = queues
            .entry(recovery.into())
            .or_insert_with(|| QueueIndex::new(&state, recovery));
        queue.advance(now);
        Ok(queue
            .ready
            .iter()
            .filter_map(|(_, _, id)| {
                let file = state.files.get(id)?;
                if !folders.contains(&file.folder_id) || active.contains(id) {
                    return None;
                }
                let job = state.jobs.get(id);
                Some(ExtractionJob {
                    file: file.clone(),
                    request_id: job.and_then(|job| job.id),
                    force: job.is_some_and(|job| job.force),
                })
            })
            .take(limit.min(32))
            .collect())
    }
    pub fn next_due(&self, folders: &[i64], recovery: &str, active: &[i64]) -> Result<Option<u64>> {
        let state = self.database.state.borrow();
        let mut queues = self.database.queues.borrow_mut();
        queues.retain(|id, _| id == recovery);
        let queue = queues
            .entry(recovery.into())
            .or_insert_with(|| QueueIndex::new(&state, recovery));
        let eligible = |id: &i64| {
            state
                .files
                .get(id)
                .is_some_and(|file| folders.contains(&file.folder_id) && !active.contains(id))
        };
        if queue.ready.iter().any(|(_, _, id)| eligible(id)) {
            return Ok(Some(0));
        }
        Ok(queue
            .future
            .iter()
            .find(|(_, (_, _, id))| eligible(id))
            .map(|(due, _)| *due))
    }
    fn current(&self, job: &ExtractionJob) -> bool {
        let state = self.database.state.borrow();
        state
            .files
            .get(&job.file.id)
            .is_some_and(|file| file.record_hash == job.file.record_hash)
            && state.jobs.get(&job.file.id).and_then(|job| job.id) == job.request_id
    }
    fn entry(&self, job: &ExtractionJob) -> JobState {
        self.database
            .state
            .borrow()
            .jobs
            .get(&job.file.id)
            .cloned()
            .unwrap_or(JobState {
                id: job.request_id,
                file_id: job.file.id,
                image_hash: job.file.image_hash.clone(),
                force: job.force,
                ready_at_ms: 0,
                lease_owner: None,
                lease_until_ms: 0,
            })
    }
    pub fn claim(&self, job: &ExtractionJob, owner: &str, now: u64, until: u64) -> Result<bool> {
        if !self.current(job) {
            return Ok(false);
        }
        let mut entry = self.entry(job);
        if entry.ready_at_ms > now || entry.lease_until_ms > now {
            return Ok(false);
        }
        entry.lease_owner = Some(owner.into());
        entry.lease_until_ms = until;
        self.database
            .commit(vec![Change::put(id_key("jobs", job.file.id), &entry)?])?;
        Ok(true)
    }
    pub fn release(&self, job: &ExtractionJob) -> Result<()> {
        if !self.current(job) {
            return Ok(());
        }
        let mut entry = self.entry(job);
        entry.lease_owner = None;
        entry.lease_until_ms = 0;
        self.database
            .commit(vec![Change::put(id_key("jobs", job.file.id), &entry)?])
    }
    pub fn recover_leases(&self) -> Result<()> {
        let mut changes = Vec::new();
        for job in self.database.state.borrow().jobs.values() {
            if job.lease_owner.is_some() || job.lease_until_ms > 0 {
                let mut job = job.clone();
                job.lease_owner = None;
                job.lease_until_ms = 0;
                changes.push(Change::put(id_key("jobs", job.file_id), &job)?);
            }
        }
        self.database.commit(changes)
    }
    pub fn defer(&self, job: &ExtractionJob, until: u64) -> Result<()> {
        if !self.current(job) {
            return Ok(());
        }
        let mut entry = self.entry(job);
        entry.ready_at_ms = until;
        entry.lease_owner = None;
        entry.lease_until_ms = 0;
        let mut changes = Vec::new();
        if entry.id.is_none() {
            let id = self.next_id()?;
            entry.id = Some(id);
            changes.push(Change::put("seq/job", &id)?);
            for (key, mut failure) in self
                .database
                .store
                .scan::<super::recovery::FailureState>(&format!("failures/{:020}/", job.file.id))?
            {
                if failure.image_hash == job.file.image_hash && failure.request_id.is_none() {
                    failure.request_id = Some(id);
                    changes.push(Change::put(key, &failure)?);
                }
            }
            if let Some(mut result) = self
                .database
                .store
                .get::<SavedResult>(&id_key("results", job.file.id))?
            {
                if result.request_id == job.request_id && result.record_hash == job.file.record_hash
                {
                    result.request_id = Some(id);
                    changes.push(Change::put(id_key("results", job.file.id), &result)?);
                }
            }
        }
        changes.push(Change::put(id_key("jobs", job.file.id), &entry)?);
        self.database.commit(changes)
    }
    pub fn saved_text(&self, job: &ExtractionJob, processor: &str) -> Result<Option<String>> {
        let Some(saved) = self
            .database
            .store
            .get::<SavedResult>(&id_key("results", job.file.id))?
        else {
            return Ok(None);
        };
        if saved.image_hash != job.file.image_hash
            || saved.record_hash != job.file.record_hash
            || saved.request_id != job.request_id
            || saved.processor != processor
        {
            return Ok(None);
        }
        let text = self
            .database
            .store
            .get(&format!("texts/{}", saved.text_hash))?
            .ok_or_else(|| Error::Storage("missing saved response body".into()))?;
        Ok(Some(text))
    }
    pub fn save_result(&self, job: &ExtractionJob, text: &str, processor: &str) -> Result<()> {
        if !self.current(job) {
            return Err(Error::ImageChanged);
        }
        let hash = crate::trailer::hash_bytes(text.as_bytes());
        let result = SavedResult {
            file_id: job.file.id,
            image_hash: job.file.image_hash.clone(),
            record_hash: job.file.record_hash.clone(),
            request_id: job.request_id,
            processor: processor.into(),
            text_hash: hash.clone(),
        };
        let mut changes = super::cache::store_changes(&job.file.image_hash, processor, text)?;
        changes.push(Change::put(id_key("results", job.file.id), &result)?);
        self.database.commit(changes)
    }
}
