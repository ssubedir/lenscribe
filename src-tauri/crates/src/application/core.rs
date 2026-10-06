use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use super::scan::ScanState;
use crate::{
    domain::{
        image::path_string,
        merkle,
        model::*,
        watch::{WatchEvent, WatchStatus},
    },
    ports::{
        images::ImageFiles,
        index::IndexRepository,
        vision::VisionFactory,
        watch::{FolderWatch, WatchTarget, Watcher},
    },
    Error, Result,
};

pub struct Core {
    pub(crate) index: Mutex<Box<dyn IndexRepository>>,
    pub(crate) images: Arc<dyn ImageFiles>,
    pub(crate) vision: Arc<dyn VisionFactory>,
    watcher: Arc<dyn Watcher>,
    /// Serialize writes and scans while allowing search during filesystem work.
    pub(super) work: Mutex<()>,
    watches: Mutex<BTreeMap<i64, Box<dyn FolderWatch>>>,
    pub(super) scan_state: Mutex<BTreeMap<i64, ScanState>>,
}

impl Core {
    /// Build application services from ports. Default desktop wiring lives in composition.
    pub fn new(
        index: Box<dyn IndexRepository>,
        images: Arc<dyn ImageFiles>,
        watcher: Arc<dyn Watcher>,
        vision: Arc<dyn VisionFactory>,
    ) -> Self {
        Self {
            index: Mutex::new(index),
            images,
            vision,
            watcher,
            work: Mutex::new(()),
            watches: Mutex::new(BTreeMap::new()),
            scan_state: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn scan_folder(&self, path: impl AsRef<Path>) -> Result<ScanReport> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        self.scan_locked(path.as_ref())
    }

    pub fn folders(&self) -> Result<Vec<FolderRecord>> {
        self.index.lock().map_err(|_| Error::Poisoned)?.folders()
    }

    pub fn folder(&self, folder_id: i64) -> Result<FolderRecord> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folder(folder_id)
    }

    pub fn folder_progress(&self) -> Result<Vec<FolderProgress>> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folder_progress()
    }

    pub fn snapshot(&self, folder_id: i64) -> Result<FolderSnapshot> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .snapshot(folder_id)
    }

    pub fn file(&self, file_id: i64) -> Result<FileDetails> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .file(file_id)
    }

    pub fn search(
        &self,
        query: &str,
        folder_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .search(query, folder_id, limit)
    }

    pub fn search_page(
        &self,
        query: &str,
        folder_id: Option<i64>,
        offset: usize,
        limit: usize,
        fuzzy: bool,
    ) -> Result<SearchPage> {
        if let Some(id) = folder_id {
            self.folder(id)?;
        }
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .search_page(query, folder_id, offset, limit, fuzzy)
    }

    pub fn prepare_image(&self, folder_id: i64, relative_path: &str) -> Result<PreparedImage> {
        let path = self.image_path(folder_id, relative_path)?;
        let image = self.images.inspect(&path)?;
        if !self.allowed_image(folder_id, relative_path, image.image_length)? {
            return Err(Error::InvalidInput(
                "image is excluded by its folder rules".into(),
            ));
        }
        let bytes = self.images.original_bytes(&path, &image.image_hash)?;
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
        let current = self.images.inspect(&path)?;
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
        self.images
            .write_text(&path, expected_image_hash, text, processor)?;
        let folder = self
            .index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folder(folder_id)?;
        self.scan_scopes_locked(Path::new(&folder.path), &[relative_path.into()], true)?;
        let database = self.index.lock().map_err(|_| Error::Poisoned)?;
        let file = database.file_by_path(folder_id, relative_path)?;
        database.clear_job(file.file.id)?;
        Ok(file)
    }

    pub fn list_files(&self, folder_id: i64, query: &str, offset: usize) -> Result<FilePage> {
        self.find_files(folder_id, query, offset, false)
    }

    pub fn find_files(
        &self,
        folder_id: i64,
        query: &str,
        offset: usize,
        fuzzy: bool,
    ) -> Result<FilePage> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .find_files(folder_id, query, offset, fuzzy)
    }

    pub fn extraction_jobs(&self, folder_id: i64) -> Result<Vec<ExtractionJob>> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .extraction_jobs(folder_id)
    }

    pub fn cached_extraction(&self, image_hash: &str, processor: &str) -> Result<Option<String>> {
        self.index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .cached_extraction(image_hash, processor)
    }

    pub fn queue_file(&self, file_id: i64, expected_image_hash: &str, force: bool) -> Result<()> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        let file = self.file(file_id)?;
        let image = self.prepare_image(file.file.folder_id, &file.file.relative_path)?;
        if image.image_hash != expected_image_hash || file.file.image_hash != expected_image_hash {
            return Err(Error::ImageChanged);
        }
        self.index.lock().map_err(|_| Error::Poisoned)?.queue_file(
            file_id,
            expected_image_hash,
            force,
        )
    }

    pub fn complete_extraction(
        &self,
        job: &ExtractionJob,
        text: &str,
        processor: &str,
    ) -> Result<FileDetails> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        {
            let database = self.index.lock().map_err(|_| Error::Poisoned)?;
            let current = database.file(job.file.id)?;
            if current.file.record_hash != job.file.record_hash
                || database.request_id(job.file.id)? != job.request_id
            {
                return Err(Error::ImageChanged);
            }
            // Persist the response before touching the image, so locks and restarts
            // can retry the trailer commit without another model request.
            database.save_result(job, text, processor)?;
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
        let root = self.images.canonical_folder(path.as_ref())?;
        let folder_id = self
            .index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .ensure_folder(path_string(&root)?)?;
        // Register before scanning so files arriving during the initial scan trigger a follow-up scan.
        let mut watches = self.watches.lock().map_err(|_| Error::Poisoned)?;
        if let std::collections::btree_map::Entry::Vacant(entry) = watches.entry(folder_id) {
            let target: Arc<dyn WatchTarget> = self.clone();
            entry.insert(self.watcher.start(
                Arc::downgrade(&target),
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
            .map(|watch| watch.status())
            .collect()
    }

    pub fn stop_watches(&self) -> Result<()> {
        let watches = std::mem::take(&mut *self.watches.lock().map_err(|_| Error::Poisoned)?);
        drop(watches);
        Ok(())
    }

    pub fn persist(&self) -> Result<()> {
        self.index.lock().map_err(|_| Error::Poisoned)?.persist()
    }

    pub(super) fn image_path(&self, folder_id: i64, relative_path: &str) -> Result<PathBuf> {
        merkle::validate_relative_path(relative_path)?;
        let folder = self
            .index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folder(folder_id)?;
        self.images
            .resolve_image(Path::new(&folder.path), relative_path)
    }
}

impl WatchTarget for Core {
    fn scan_folder(&self, root: &Path) -> Result<ScanReport> {
        Core::scan_folder(self, root)
    }
    fn scan_paths(&self, folder: i64, paths: &[PathBuf]) -> Result<ScanReport> {
        Core::scan_paths(self, folder, paths)
    }
    fn reconcile_folder(&self, root: &Path) -> Result<ScanReport> {
        Core::reconcile_folder(self, root)
    }
}
