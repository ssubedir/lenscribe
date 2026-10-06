//! WeDB owns canonical records; Core serializes read/modify/write operations.
mod cache;
mod files;
mod folders;
mod jobs;
#[cfg(feature = "legacy-sqlite")]
mod legacy;
mod maintenance;
mod recovery;
mod search;
mod store;
#[cfg(test)]
mod tests;

use crate::{Error, FileDetails, Result};
pub(crate) use maintenance::restore;
pub(crate) use recovery::EndpointState;
use std::{
    cell::{Cell, RefCell},
    path::Path,
};
use store::{Change, State, Store};

pub(crate) struct Database {
    pub(crate) wake: std::sync::Arc<tokio::sync::Notify>,
    store: Store,
    state: RefCell<State>,
    index: RefCell<search::SearchIndex>,
    queues: RefCell<std::collections::BTreeMap<String, jobs::QueueIndex>>,
    poisoned: Cell<bool>,
    #[cfg(test)]
    fail_write: Cell<bool>,
}

impl Database {
    pub fn persist(&self) -> Result<()> {
        if self.poisoned.get() {
            return Err(Error::Storage(
                "storage needs reopening after a failed commit".into(),
            ));
        }
        let result = self.store.db.persist().map_err(store::storage_error);
        if result.is_err() {
            self.poisoned.set(true);
        }
        result
    }
    pub fn open(path: &Path) -> Result<Self> {
        let store = Store::open(path)?;
        if store.get::<u32>("schema")?.is_none() {
            let changes = vec![Change::put("schema", &store::SCHEMA)?];
            let old = store.path.with_extension("sqlite");
            #[cfg(feature = "legacy-sqlite")]
            let changes = {
                let mut changes = changes;
                if old.is_file() {
                    changes.extend(legacy::import(&old)?);
                }
                changes
            };
            #[cfg(not(feature = "legacy-sqlite"))]
            if old.is_file() {
                return Err(Error::InvalidInput(
                    "existing SQLite data requires the legacy-sqlite migration feature".into(),
                ));
            }
            store.commit(&changes)?;
        }
        if store.get::<u32>("schema")? != Some(store::SCHEMA) {
            return Err(Error::InvalidInput(
                "database belongs to an unsupported Lenscribe version".into(),
            ));
        }
        let state = State::load(&store)?;
        let index = search::SearchIndex::load(&store, &state)?;
        let database = Self {
            wake: std::sync::Arc::new(tokio::sync::Notify::new()),
            store,
            state: RefCell::new(state),
            index: RefCell::new(index),
            queues: RefCell::new(std::collections::BTreeMap::new()),
            poisoned: Cell::new(false),
            #[cfg(test)]
            fail_write: Cell::new(false),
        };
        database.jobs().recover_leases()?;
        Ok(database)
    }

    fn commit(&self, changes: Vec<Change>) -> Result<()> {
        if self.poisoned.get() {
            return Err(Error::Storage(
                "storage needs reopening after a failed commit".into(),
            ));
        }
        #[cfg(test)]
        if self.fail_write.replace(false) {
            return Err(Error::Storage("injected write failure".into()));
        }
        // Publish projections only after the atomic batch AND its disk sync succeed.
        if let Err(error) = self.store.commit(&changes) {
            self.poisoned.set(true);
            return Err(error);
        }
        if let Err(error) = self.publish(&changes) {
            self.poisoned.set(true);
            return Err(error);
        }
        self.wake.notify_one();
        Ok(())
    }

    fn publish(&self, changes: &[Change]) -> Result<()> {
        let mut state = self.state.borrow_mut();
        for change in changes {
            state.apply(change)?;
        }
        let touched = changes
            .iter()
            .filter_map(|change| {
                let (table, rest) = change.key.split_once('/')?;
                matches!(table, "files" | "jobs" | "failures")
                    .then(|| rest.split('/').next()?.parse::<i64>().ok())
                    .flatten()
            })
            .collect::<std::collections::BTreeSet<_>>();
        for (recovery, queue) in self.queues.borrow_mut().iter_mut() {
            for id in &touched {
                queue.update(&state, recovery, *id);
            }
        }
        let mut index = self.index.borrow_mut();
        for change in changes {
            if let Some(id) = change.key.strip_prefix("files/") {
                let id: i64 = id
                    .parse()
                    .map_err(|_| Error::Storage("invalid file key".into()))?;
                if let Some(file) = state.files.get(&id) {
                    index.upsert(&self.store, file)?;
                } else {
                    index.remove(id);
                }
            }
        }
        Ok(())
    }

    pub fn folders(&self) -> folders::FolderRepository<'_> {
        folders::FolderRepository::new(self)
    }
    pub fn files(&self) -> files::FileRepository<'_> {
        files::FileRepository::new(self)
    }
    pub fn cache(&self) -> cache::ExtractionCacheRepository<'_> {
        cache::ExtractionCacheRepository::new(self)
    }
    pub fn jobs(&self) -> jobs::ExtractionJobRepository<'_> {
        jobs::ExtractionJobRepository::new(self)
    }
    pub fn recovery(&self) -> recovery::RecoveryRepository<'_> {
        recovery::RecoveryRepository::new(self)
    }
    pub fn search(&self) -> search::SearchRepository<'_> {
        search::SearchRepository::new(self)
    }
    pub fn maintenance(&self) -> maintenance::MaintenanceRepository<'_> {
        maintenance::MaintenanceRepository::new(self)
    }

    #[cfg(test)]
    pub fn apply_changes(
        &mut self,
        folder_id: i64,
        changed: &[FileDetails],
        removed: &[String],
        root_hash: &str,
    ) -> Result<()> {
        self.apply_scan(folder_id, changed, removed, root_hash, None)
    }

    pub fn merkle_checkpoint(
        &self,
        folder: i64,
        records: &[crate::FileRecord],
        root: &str,
    ) -> Result<Option<crate::merkle::MerkleTree>> {
        let prefix = format!("merkle/{folder:020}/");
        let entries = self
            .store
            .scan::<std::collections::BTreeMap<String, crate::merkle::Child>>(&prefix)?;
        if entries.is_empty() {
            return Ok(None);
        }
        let directories = entries
            .into_iter()
            .map(|(key, value)| (key[prefix.len()..].to_owned(), value))
            .collect();
        // Checkpoints are disposable: fall back to rebuilding from canonical records.
        Ok(crate::merkle::MerkleTree::from_checkpoints(directories, records, root).ok())
    }

    pub fn apply_scan(
        &mut self,
        folder_id: i64,
        changed: &[FileDetails],
        removed: &[String],
        root_hash: &str,
        tree: Option<&crate::merkle::MerkleTree>,
    ) -> Result<()> {
        let state = self.state.borrow();
        let mut folder = state.folder(folder_id)?.clone();
        if changed.is_empty() && removed.is_empty() && folder.root_hash == root_hash {
            return Ok(());
        }
        let mut sequence = self.store.get::<i64>("seq/file")?.unwrap_or(0);
        let mut changes = Vec::new();
        let mut paths = std::collections::BTreeSet::new();
        for details in changed {
            crate::merkle::validate_relative_path(&details.file.relative_path)?;
            if !paths.insert(&details.file.relative_path) {
                return Err(Error::InvalidInput("duplicate changed path".into()));
            }
            let old = state.by_path(folder_id, &details.file.relative_path);
            if old.is_some_and(|file| file.record_hash == details.file.record_hash) {
                continue;
            }
            let mut file = details.file.clone();
            file.folder_id = folder_id;
            file.id = if let Some(old) = old {
                old.id
            } else {
                sequence = sequence
                    .checked_add(1)
                    .ok_or_else(|| Error::Storage("file IDs exhausted".into()))?;
                sequence
            };
            if details
                .text
                .as_deref()
                .map(|text| crate::trailer::hash_bytes(text.as_bytes()))
                != file.text_hash
            {
                return Err(Error::InvalidInput(
                    "text hash does not match the extracted text".into(),
                ));
            }
            changes.push(Change::put(store::id_key("files", file.id), &file)?);
            // Recover a crash after the trailer was replaced but before completion
            // was recorded. Only this generation's exact result acknowledges the job.
            if let Some(saved) = self
                .store
                .get::<jobs::SavedResult>(&store::id_key("results", file.id))?
            {
                let request = state.jobs.get(&file.id).and_then(|job| job.id);
                if saved.request_id == request
                    && saved.image_hash == file.image_hash
                    && file.text_hash.as_deref() == Some(&saved.text_hash)
                    && file.processor.as_deref() == Some(&saved.processor)
                {
                    changes.push(Change::remove(store::id_key("jobs", file.id)));
                    changes.push(Change::remove(store::id_key("results", file.id)));
                }
            }
            if let Some(text) = &details.text {
                changes.push(Change::put(
                    format!("texts/{}", file.text_hash.as_ref().unwrap()),
                    text,
                )?);
                if let Some(processor) = &file.processor {
                    changes.extend(cache::store_changes(&file.image_hash, processor, text)?);
                }
            }
            if old.is_some_and(|old| old.image_hash != file.image_hash) {
                changes.push(Change::remove(store::id_key("jobs", file.id)));
                changes.push(Change::remove(store::id_key("results", file.id)));
            }
            changes.extend(self.recovery().discard_changes(
                file.id,
                &file.image_hash,
                file.processor.is_some(),
            )?);
        }
        for path in removed {
            if let Some(file) = state.by_path(folder_id, path) {
                changes.push(Change::remove(store::id_key("files", file.id)));
                changes.push(Change::remove(store::id_key("jobs", file.id)));
                changes.push(Change::remove(store::id_key("results", file.id)));
                changes.extend(self.recovery().discard_changes(file.id, "", true)?);
            }
        }
        folder.root_hash = root_hash.into();
        changes.push(Change::put(store::id_key("folders", folder_id), &folder)?);
        changes.push(Change::put("seq/file", &sequence)?);
        if let Some(tree) = tree {
            let paths = changed
                .iter()
                .map(|details| details.file.relative_path.clone())
                .chain(removed.iter().cloned())
                .collect::<Vec<_>>();
            for (path, children) in tree.checkpoints(&paths) {
                let key = format!("merkle/{folder_id:020}/{path}");
                if let Some(children) = children {
                    changes.push(Change::put(key, &children)?);
                } else {
                    changes.push(Change::remove(key));
                }
            }
        }
        drop(state);
        self.commit(changes)
    }
}
