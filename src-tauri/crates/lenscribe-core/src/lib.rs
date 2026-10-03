//! The desktop-independent Lenscribe engine. AI extraction is an adapter added by the application.

#[cfg(feature = "bindings")]
pub mod bindings;
pub mod daemon;
mod database;
mod error;
pub mod extraction;
pub mod http;
pub mod llm;
pub mod merkle;
mod migrations;
pub mod model;
mod scan;
pub mod settings;
pub mod trailer;
mod watch;

pub use error::{Error, Result};
pub use model::*;
pub use watch::{WatchEvent, WatchFailure, WatchStatus};

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use database::Database;
use scan::ScanState;
use watch::FolderWatch;

pub struct Core {
    database: Mutex<Database>,
    /// Serialize writes and scans while allowing search during filesystem work.
    work: Mutex<()>,
    watches: Mutex<BTreeMap<i64, FolderWatch>>,
    scan_state: Mutex<BTreeMap<i64, ScanState>>,
}

impl Core {
    pub fn open(database_path: impl AsRef<Path>) -> Result<Self> {
        let path = database_path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        Ok(Self {
            database: Mutex::new(Database::open(path)?),
            work: Mutex::new(()),
            watches: Mutex::new(BTreeMap::new()),
            scan_state: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn scan_folder(&self, path: impl AsRef<Path>) -> Result<ScanReport> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        self.scan_locked(path.as_ref())
    }

    pub fn folders(&self) -> Result<Vec<FolderRecord>> {
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .list()
    }

    pub fn folder(&self, folder_id: i64) -> Result<FolderRecord> {
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .get(folder_id)
    }

    pub fn folder_progress(&self) -> Result<Vec<FolderProgress>> {
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .progress()
    }

    pub fn snapshot(&self, folder_id: i64) -> Result<FolderSnapshot> {
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .snapshot(folder_id)
    }

    pub fn file(&self, file_id: i64) -> Result<FileDetails> {
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .files()
            .get(file_id)
    }

    pub fn search(
        &self,
        query: &str,
        folder_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .search()
            .query(query, folder_id, limit)
    }

    pub fn prepare_image(&self, folder_id: i64, relative_path: &str) -> Result<PreparedImage> {
        let path = self.image_path(folder_id, relative_path)?;
        let image = trailer::inspect(&path)?;
        if !self.allowed_image(folder_id, relative_path, image.image_length)? {
            return Err(Error::InvalidInput(
                "image is excluded by its folder rules".into(),
            ));
        }
        let bytes = trailer::original_bytes(&path, &image.image_hash)?;
        Ok(PreparedImage {
            folder_id,
            relative_path: relative_path.into(),
            image_hash: image.image_hash,
            mime_type: image.mime_type.into(),
            bytes,
        })
    }

    pub fn attach_text(
        &self,
        folder_id: i64,
        relative_path: &str,
        expected_image_hash: &str,
        text: &str,
        processor: &str,
    ) -> Result<FileDetails> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        self.attach_locked(
            folder_id,
            relative_path,
            expected_image_hash,
            None,
            text,
            processor,
        )
    }

    fn attach_locked(
        &self,
        folder_id: i64,
        relative_path: &str,
        expected_image_hash: &str,
        expected_record_hash: Option<&str>,
        text: &str,
        processor: &str,
    ) -> Result<FileDetails> {
        let path = self.image_path(folder_id, relative_path)?;
        let current = trailer::inspect(&path)?;
        let current_record = merkle::record_hash(
            &current.image_hash,
            current
                .trailer
                .as_ref()
                .map(|value| (value.text.as_str(), value.processor.as_str())),
        )?;
        if expected_record_hash.is_some_and(|expected| expected != current_record) {
            return Err(Error::ImageChanged);
        }
        if !self.allowed_image(folder_id, relative_path, current.image_length)? {
            return Err(Error::InvalidInput(
                "image is excluded by its folder rules".into(),
            ));
        }
        trailer::write_text(&path, expected_image_hash, text, processor)?;
        let folder = self
            .database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .get(folder_id)?;
        self.scan_scopes_locked(Path::new(&folder.path), &[relative_path.into()], true)?;
        let database = self.database.lock().map_err(|_| Error::Poisoned)?;
        let file = database.files().by_path(folder_id, relative_path)?;
        database.jobs().clear(file.file.id)?;
        Ok(file)
    }

    pub fn list_files(&self, folder_id: i64, query: &str, offset: usize) -> Result<FilePage> {
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .files()
            .list(folder_id, query, offset)
    }

    pub fn extraction_jobs(&self, folder_id: i64) -> Result<Vec<ExtractionJob>> {
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .jobs()
            .list(folder_id)
    }

    pub fn cached_extraction(&self, image_hash: &str, processor: &str) -> Result<Option<String>> {
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .cache()
            .get(image_hash, processor)
    }

    pub fn queue_file(&self, file_id: i64, expected_image_hash: &str, force: bool) -> Result<()> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        let file = self.file(file_id)?;
        let image = self.prepare_image(file.file.folder_id, &file.file.relative_path)?;
        if image.image_hash != expected_image_hash || file.file.image_hash != expected_image_hash {
            return Err(Error::ImageChanged);
        }
        self.database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .jobs()
            .queue(file_id, expected_image_hash, force)
    }

    pub fn complete_extraction(
        &self,
        job: &ExtractionJob,
        text: &str,
        processor: &str,
    ) -> Result<FileDetails> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        {
            let database = self.database.lock().map_err(|_| Error::Poisoned)?;
            let current = database.files().get(job.file.id)?;
            if current.file.record_hash != job.file.record_hash
                || database.jobs().request_id(job.file.id)? != job.request_id
            {
                return Err(Error::ImageChanged);
            }
        }
        self.attach_locked(
            job.file.folder_id,
            &job.file.relative_path,
            &job.file.image_hash,
            Some(&job.file.record_hash),
            text,
            processor,
        )
    }

    pub fn edit_file(
        &self,
        file_id: i64,
        expected_image_hash: &str,
        expected_record_hash: &str,
        text: &str,
    ) -> Result<FileDetails> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        let file = self.file(file_id)?.file;
        if file.image_hash != expected_image_hash || file.record_hash != expected_record_hash {
            return Err(Error::ImageChanged);
        }
        self.attach_locked(
            file.folder_id,
            &file.relative_path,
            expected_image_hash,
            Some(expected_record_hash),
            text,
            "manual/v1",
        )
    }

    pub fn watch_folder(
        self: &Arc<Self>,
        path: impl AsRef<Path>,
        on_event: Arc<dyn Fn(WatchEvent) + Send + Sync>,
    ) -> Result<ScanReport> {
        let root = canonical_folder(path.as_ref())?;
        let folder_id = self
            .database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .ensure(path_string(&root)?)?;
        // Register before scanning so files arriving during the initial scan trigger a follow-up scan.
        let mut watches = self.watches.lock().map_err(|_| Error::Poisoned)?;
        if let std::collections::btree_map::Entry::Vacant(entry) = watches.entry(folder_id) {
            entry.insert(FolderWatch::start(
                Arc::downgrade(self),
                folder_id,
                root.clone(),
                on_event,
            )?);
        }
        drop(watches);
        self.scan_folder(root)
    }

    pub fn unwatch_folder(&self, folder_id: i64) -> Result<()> {
        let watch = self
            .watches
            .lock()
            .map_err(|_| Error::Poisoned)?
            .remove(&folder_id);
        drop(watch);
        Ok(())
    }

    pub fn watch_status(&self) -> Result<Vec<WatchStatus>> {
        self.watches
            .lock()
            .map_err(|_| Error::Poisoned)?
            .values()
            .map(FolderWatch::status)
            .collect()
    }

    pub fn stop_watches(&self) -> Result<()> {
        let watches = std::mem::take(&mut *self.watches.lock().map_err(|_| Error::Poisoned)?);
        drop(watches);
        Ok(())
    }

    fn image_path(&self, folder_id: i64, relative_path: &str) -> Result<PathBuf> {
        merkle::validate_relative_path(relative_path)?;
        let folder = self
            .database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .get(folder_id)?;
        let root = canonical_folder(Path::new(&folder.path))?;
        let candidate = root.join(relative_path);
        // Reject symlinks anywhere in a write/extraction path, including intermediate directories.
        let mut component_path = root.clone();
        for part in relative_path.split('/') {
            component_path.push(part);
            if fs::symlink_metadata(&component_path)?
                .file_type()
                .is_symlink()
            {
                return Err(Error::InvalidInput(
                    "image paths cannot contain symbolic links".into(),
                ));
            }
        }
        let path = candidate.canonicalize()?;
        if !path.starts_with(&root) || !path.is_file() || !trailer::supported_path(&path) {
            return Err(Error::InvalidInput(
                "image must be a PNG, JPEG, or WebP inside the indexed folder".into(),
            ));
        }
        Ok(path)
    }
}

fn canonical_folder(path: &Path) -> Result<PathBuf> {
    let root = path.canonicalize()?;
    if !root.is_dir() {
        return Err(Error::InvalidInput("select a directory to index".into()));
    }
    Ok(root)
}

fn path_string(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| Error::InvalidInput("paths must be valid Unicode".into()))
}
