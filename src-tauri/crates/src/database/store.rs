use crate::{Error, FileRecord, FolderRecord, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use wedb_embed::engine::KvEntry;
use wedb_embed::{Engine, Fjall, Partition, WeDb};

pub(super) const SCHEMA: u32 = 1;
pub(super) fn id_key(prefix: &str, id: i64) -> String {
    format!("{prefix}/{id:020}")
}
pub(super) fn storage_error(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

pub(super) struct Change {
    pub key: String,
    pub value: Option<Vec<u8>>,
}
impl Change {
    pub fn put(key: impl Into<String>, value: &impl Serialize) -> Result<Self> {
        Ok(Self {
            key: key.into(),
            value: Some(serde_json::to_vec(value)?),
        })
    }
    pub fn remove(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: None,
        }
    }
}

pub(super) struct Store {
    pub path: PathBuf,
    pub db: WeDb<Fjall>,
    pub records: wedb_embed::engine::fjall::FjallPartition,
    // Drop the engine before deleting an ephemeral database (important on Windows).
    _temporary: Option<tempfile::TempDir>,
    #[cfg(test)]
    pub fail_sync: std::cell::Cell<bool>,
}
impl Store {
    pub fn storage_path(path: &Path) -> PathBuf {
        if path.extension().is_some_and(|ext| ext == "sqlite") {
            path.with_extension("wedb")
        } else {
            path.to_owned()
        }
    }
    pub fn open(path: &Path) -> Result<Self> {
        let temporary = (path == Path::new(":memory:"))
            .then(tempfile::tempdir)
            .transpose()?;
        let path = temporary.as_ref().map_or_else(
            || Self::storage_path(path),
            |temp| temp.path().join("index.wedb"),
        );
        if path.is_file() {
            return Err(Error::InvalidInput(
                "WeDB storage must be a directory".into(),
            ));
        }
        let builder = Fjall::default_database_builder(&path)
            .cache_size(32 * 1024 * 1024)
            .max_journaling_size(128 * 1024 * 1024)
            .max_cached_files(Some(128))
            .worker_threads(2);
        let data = Fjall::default_data_partition_options().max_memtable_size(8 * 1024 * 1024);
        let meta = Fjall::default_meta_partition_options().max_memtable_size(4 * 1024 * 1024);
        let engine = Fjall::open_with_cfg(builder, data, meta).map_err(storage_error)?;
        let records = engine.partition("lenscribe-v1").map_err(storage_error)?;
        Ok(Self {
            path,
            db: WeDb::new(engine),
            records,
            _temporary: temporary,
            #[cfg(test)]
            fail_sync: std::cell::Cell::new(false),
        })
    }
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.records
            .get(key.as_bytes())
            .map_err(storage_error)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(Error::from))
            .transpose()
    }
    pub fn scan<T: DeserializeOwned>(&self, prefix: &str) -> Result<Vec<(String, T)>> {
        self.records
            .prefix(prefix.as_bytes())
            .map(|entry| {
                let entry = entry.map_err(storage_error)?;
                let key = std::str::from_utf8(entry.key())
                    .map_err(storage_error)?
                    .to_owned();
                Ok((key, serde_json::from_slice(entry.value())?))
            })
            .collect()
    }
    pub fn visit<T: DeserializeOwned>(
        &self,
        prefix: &str,
        mut visitor: impl FnMut(String, T) -> Result<()>,
    ) -> Result<()> {
        for entry in self.records.prefix(prefix.as_bytes()) {
            let entry = entry.map_err(storage_error)?;
            let key = std::str::from_utf8(entry.key())
                .map_err(storage_error)?
                .to_owned();
            visitor(key, serde_json::from_slice(entry.value())?)?;
        }
        Ok(())
    }
    pub fn commit(&self, changes: &[Change]) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        // Shared text bodies may appear repeatedly in a scan/import. Keep only
        // the final mutation for each key before constructing the WAL batch.
        let unique: BTreeMap<_, _> = changes.iter().map(|change| (&change.key, change)).collect();
        let mut batch = self.db.batch_with_capacity(unique.len());
        for change in unique.into_values() {
            if let Some(value) = &change.value {
                batch.insert(&self.records, change.key.as_bytes(), value);
            } else {
                batch.rm(&self.records, change.key.as_bytes());
            }
        }
        batch.commit().map_err(storage_error)?;
        #[cfg(test)]
        if self.fail_sync.replace(false) {
            return Err(Error::Storage("injected disk sync failure".into()));
        }
        self.db.persist().map_err(storage_error)
    }
}

#[derive(Default)]
pub(super) struct State {
    pub folders: BTreeMap<i64, FolderRecord>,
    pub files: BTreeMap<i64, FileRecord>,
    pub paths: BTreeMap<(i64, String), i64>,
    pub jobs: BTreeMap<i64, super::jobs::JobState>,
    pub failures: BTreeMap<(String, i64), super::recovery::FailureState>,
}
impl State {
    pub fn load(store: &Store) -> Result<Self> {
        let mut state = Self::default();
        store.visit("folders/", |key, folder: FolderRecord| {
            if key != id_key("folders", folder.id) {
                return Err(Error::Storage("invalid folder key".into()));
            }
            state.folders.insert(folder.id, folder);
            Ok(())
        })?;
        store.visit("files/", |key, file: FileRecord| {
            if key != id_key("files", file.id)
                || state
                    .paths
                    .insert((file.folder_id, file.relative_path.clone()), file.id)
                    .is_some()
            {
                return Err(Error::Storage("invalid file key or duplicate path".into()));
            }
            state.files.insert(file.id, file);
            Ok(())
        })?;
        store.visit("jobs/", |key, job: super::jobs::JobState| {
            if key != id_key("jobs", job.file_id) || !state.files.contains_key(&job.file_id) {
                return Err(Error::Storage("invalid job reference".into()));
            }
            state.jobs.insert(job.file_id, job);
            Ok(())
        })?;
        store.visit("failures/", |_, failure: super::recovery::FailureState| {
            state
                .failures
                .insert((failure.recovery_id.clone(), failure.file_id), failure);
            Ok(())
        })?;
        for file in state.files.values() {
            state.folder(file.folder_id)?;
        }
        Ok(state)
    }
    pub fn folder(&self, id: i64) -> Result<&FolderRecord> {
        self.folders
            .get(&id)
            .ok_or_else(|| Error::NotFound(format!("folder {id}")))
    }
    pub fn by_path(&self, folder: i64, path: &str) -> Option<&FileRecord> {
        self.paths
            .get(&(folder, path.into()))
            .and_then(|id| self.files.get(id))
    }
    pub fn apply(&mut self, change: &Change) -> Result<()> {
        let Some((table, id)) = change.key.split_once('/') else {
            return Ok(());
        };
        if table == "failures" {
            let (file, recovery) = id
                .split_once('/')
                .ok_or_else(|| Error::Storage("invalid failure key".into()))?;
            let id = file.parse::<i64>().map_err(storage_error)?;
            if let Some(value) = &change.value {
                self.failures
                    .insert((recovery.into(), id), serde_json::from_slice(value)?);
            } else {
                self.failures.remove(&(recovery.into(), id));
            }
            return Ok(());
        }
        if !matches!(table, "folders" | "files" | "jobs") {
            return Ok(());
        }
        let id: i64 = id.parse().map_err(storage_error)?;
        match table {
            "folders" => {
                if let Some(value) = &change.value {
                    self.folders.insert(id, serde_json::from_slice(value)?);
                } else {
                    self.folders.remove(&id);
                }
            }
            "files" => {
                if let Some(old) = self.files.remove(&id) {
                    self.paths.remove(&(old.folder_id, old.relative_path));
                }
                if let Some(value) = &change.value {
                    let file: FileRecord = serde_json::from_slice(value)?;
                    self.paths
                        .insert((file.folder_id, file.relative_path.clone()), id);
                    self.files.insert(id, file);
                }
            }
            "jobs" => {
                if let Some(value) = &change.value {
                    self.jobs.insert(id, serde_json::from_slice(value)?);
                } else {
                    self.jobs.remove(&id);
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
