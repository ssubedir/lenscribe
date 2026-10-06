//! Configuration owns the background lifecycle. A UI is an optional client of this controller.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use crate::{
    extraction::{ExtractionController, ExtractionStatus},
    http::ApiServer,
    settings::Settings,
    Core, Error, FileRecord, Result, WatchEvent, WatchStatus,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DaemonIssue {
    pub source: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    pub settings: Settings,
    pub watchers: Vec<WatchStatus>,
    pub pending_images: usize,
    pub total_images: usize,
    pub processed_images: usize,
    pub folder_statuses: Vec<DaemonFolderStatus>,
    pub api_url: Option<String>,
    pub issues: Vec<DaemonIssue>,
    pub extraction: ExtractionStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DaemonFolderStatus {
    pub path: String,
    pub enabled: bool,
    pub folder_id: Option<i64>,
    pub watching: bool,
    pub image_count: usize,
    pub pending_images: usize,
    pub root_hash: Option<String>,
    pub last_error: Option<String>,
}

struct RuntimeState {
    settings: Settings,
    api: Option<ApiServer>,
    api_port: Option<u16>,
    issues: Vec<DaemonIssue>,
}

pub struct Daemon {
    core: Arc<Core>,
    settings_path: PathBuf,
    state: Mutex<RuntimeState>,
    lifecycle: Arc<tokio::sync::Mutex<()>>,
    extraction: ExtractionController,
    supervisor: Mutex<Option<JoinHandle<()>>>,
    stopped: Arc<AtomicBool>,
    on_event: Arc<dyn Fn(WatchEvent) + Send + Sync>,
}

impl Daemon {
    pub fn load(
        core: Arc<Core>,
        settings_path: impl AsRef<Path>,
        on_event: Arc<dyn Fn(WatchEvent) + Send + Sync>,
    ) -> Result<Arc<Self>> {
        let settings_path = settings_path.as_ref().to_path_buf();
        let settings = Settings::load(&settings_path)?;
        let lifecycle = Arc::new(tokio::sync::Mutex::new(()));
        Ok(Arc::new(Self {
            core,
            settings_path,
            state: Mutex::new(RuntimeState {
                settings,
                api: None,
                api_port: None,
                issues: vec![],
            }),
            extraction: ExtractionController::new(lifecycle.clone()),
            lifecycle,
            supervisor: Mutex::new(None),
            stopped: Arc::new(AtomicBool::new(false)),
            on_event,
        }))
    }

    pub fn settings(&self) -> Result<Settings> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::Poisoned)?
            .settings
            .clone())
    }

    pub fn pending_images(&self) -> Result<Vec<FileRecord>> {
        let mut files = vec![];
        let configured: BTreeSet<_> = self
            .settings()?
            .folders
            .iter()
            .filter(|folder| folder.enabled)
            .filter_map(|folder| Path::new(&folder.path).canonicalize().ok())
            .collect();
        for folder in self.core.folders()? {
            if configured.contains(Path::new(&folder.path)) {
                files.extend(
                    self.core
                        .extraction_jobs(folder.id)?
                        .into_iter()
                        .map(|job| job.file),
                );
            }
        }
        Ok(files)
    }

    pub fn status(&self) -> Result<DaemonStatus> {
        let state = self.state.lock().map_err(|_| Error::Poisoned)?;
        let mut status = DaemonStatus {
            settings: state.settings.clone(),
            watchers: vec![],
            pending_images: 0,
            total_images: 0,
            processed_images: 0,
            folder_statuses: vec![],
            api_url: state.api.as_ref().map(ApiServer::url),
            issues: state.issues.clone(),
            extraction: self.extraction.status()?,
        };
        drop(state);
        status.watchers = self.core.watch_status()?;
        // Polling the settings window needs aggregate counts, not every file record.
        let progress = self.core.folder_progress()?;
        for configured in &status.settings.folders {
            let root = Path::new(&configured.path).canonicalize().ok();
            let indexed = progress
                .iter()
                .find(|entry| root.as_deref() == Some(Path::new(&entry.folder.path)));
            let watcher = status
                .watchers
                .iter()
                .find(|watcher| root.as_deref() == Some(Path::new(&watcher.path)));
            let image_count = indexed.map_or(0, |entry| entry.folder.image_count);
            let pending_images = indexed.map_or(0, |entry| entry.pending_images);
            if configured.enabled {
                status.total_images += image_count;
                status.pending_images += pending_images;
            }
            let last_error = watcher
                .and_then(|watcher| watcher.last_error.clone())
                .or_else(|| {
                    status
                        .issues
                        .iter()
                        .find(|issue| issue.source == configured.path)
                        .map(|issue| issue.error.clone())
                });
            status.folder_statuses.push(DaemonFolderStatus {
                path: configured.path.clone(),
                enabled: configured.enabled,
                folder_id: indexed.map(|entry| entry.folder.id),
                watching: watcher.is_some(),
                image_count,
                pending_images,
                root_hash: indexed.map(|entry| entry.folder.root_hash.clone()),
                last_error,
            });
        }
        status.processed_images = status.total_images - status.pending_images;
        for watcher in &status.watchers {
            if let Some(error) = &watcher.last_error {
                status.issues.push(DaemonIssue {
                    source: watcher.path.clone(),
                    error: error.clone(),
                });
            }
        }
        Ok(status)
    }

    pub async fn start(self: &Arc<Self>) -> Result<DaemonStatus> {
        if self.stopped.load(Ordering::Relaxed) {
            return Err(Error::InvalidInput("daemon has been stopped".into()));
        }
        let status = self.reconcile().await?;
        let mut supervisor = self.supervisor.lock().map_err(|_| Error::Poisoned)?;
        if supervisor.is_none() {
            let weak = Arc::downgrade(self);
            *supervisor = Some(tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    let Some(daemon) = weak.upgrade() else {
                        break;
                    };
                    if daemon.stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Err(error) = daemon.reconcile().await {
                        log::error!("Daemon reconciliation failed: {error}");
                    }
                }
            }));
        }
        log::info!(
            "Daemon started; monitoring_paused={}",
            status.settings.monitoring_paused
        );
        Ok(status)
    }

    pub async fn update_settings(&self, settings: Settings) -> Result<DaemonStatus> {
        let _lifecycle = self.lifecycle.lock().await;
        if self.stopped.load(Ordering::Relaxed) {
            return Err(Error::InvalidInput("daemon has been stopped".into()));
        }
        let path = self.settings_path.clone();
        let saved = settings.clone();
        tokio::task::spawn_blocking(move || saved.save(&path))
            .await
            .map_err(task_error)??;
        self.state.lock().map_err(|_| Error::Poisoned)?.settings = settings;
        log::info!("Settings saved");
        self.reconcile_locked(false).await
    }

    pub async fn reconcile(&self) -> Result<DaemonStatus> {
        let _lifecycle = self.lifecycle.lock().await;
        if self.stopped.load(Ordering::Relaxed) {
            return Err(Error::InvalidInput("daemon has been stopped".into()));
        }
        self.reconcile_locked(false).await
    }

    pub async fn retry(&self) -> Result<DaemonStatus> {
        let _lifecycle = self.lifecycle.lock().await;
        if self.stopped.load(Ordering::Relaxed) {
            return Err(Error::InvalidInput("daemon has been stopped".into()));
        }
        self.reconcile_locked(true).await
    }

    fn actionable_file(&self, file_id: i64) -> Result<()> {
        let file = self.core.file(file_id)?;
        let folder = self.core.folder(file.file.folder_id)?;
        if !self.settings()?.folders.iter().any(|configured| {
            configured.enabled
                && Path::new(&configured.path)
                    .canonicalize()
                    .is_ok_and(|path| path == Path::new(&folder.path))
        }) {
            return Err(Error::InvalidInput(
                "Enable this watched folder before changing its files".into(),
            ));
        }
        Ok(())
    }

    pub async fn queue_file(
        &self,
        file_id: i64,
        expected_image_hash: String,
        force: bool,
    ) -> Result<DaemonStatus> {
        let _lifecycle = self.lifecycle.lock().await;
        self.actionable_file(file_id)?;
        let core = self.core.clone();
        tokio::task::spawn_blocking(move || core.queue_file(file_id, &expected_image_hash, force))
            .await
            .map_err(task_error)??;
        let restart_endpoint = self.extraction.status()?.phase
            == crate::extraction::ExtractionPhase::NeedsConfiguration;
        self.reconcile_locked(restart_endpoint).await
    }

    pub async fn edit_file(
        &self,
        file_id: i64,
        expected_image_hash: String,
        expected_record_hash: String,
        text: String,
    ) -> Result<crate::FileDetails> {
        let _lifecycle = self.lifecycle.lock().await;
        self.actionable_file(file_id)?;
        let core = self.core.clone();
        tokio::task::spawn_blocking(move || {
            core.edit_file(file_id, &expected_image_hash, &expected_record_hash, &text)
        })
        .await
        .map_err(task_error)?
    }

    async fn reconcile_locked(&self, force_retry: bool) -> Result<DaemonStatus> {
        let settings = self.settings()?;
        let core = self.core.clone();
        let configured = settings.clone();
        let on_event = self.on_event.clone();
        let stopped = self.stopped.clone();
        let mut issues = tokio::task::spawn_blocking(move || {
            let mut keep = BTreeSet::new();
            let mut issues = vec![];
            if !configured.monitoring_paused {
                for folder in configured.folders.iter().filter(|folder| folder.enabled) {
                    if stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    let root = match Path::new(&folder.path).canonicalize() {
                        Ok(root) => root,
                        Err(error) => {
                            issues.push(DaemonIssue {
                                source: folder.path.clone(),
                                error: error.to_string(),
                            });
                            continue;
                        }
                    };
                    let current = core.watch_status()?;
                    let rules_changed = match core.set_folder_rules(&root, folder) {
                        Ok(changed) => changed,
                        Err(error) => {
                            issues.push(DaemonIssue {
                                source: folder.path.clone(),
                                error: error.to_string(),
                            });
                            continue;
                        }
                    };
                    if let Some(active) = current
                        .iter()
                        .find(|watcher| Path::new(&watcher.path) == root)
                    {
                        keep.insert(active.folder_id);
                        if rules_changed {
                            match core.reconcile_folder(&root) {
                                Ok(report) => on_event(WatchEvent::Updated(report)),
                                Err(error) => issues.push(DaemonIssue {
                                    source: folder.path.clone(),
                                    error: error.to_string(),
                                }),
                            }
                        }
                        continue;
                    }
                    match core.watch_folder(&root, on_event.clone()) {
                        Ok(report) => {
                            log::info!(
                                "Folder {} initial scan: indexed={}, inspected={}, issues={}",
                                report.folder.id,
                                report.folder.image_count,
                                report.inspected,
                                report.issues.len()
                            );
                            keep.insert(report.folder.id);
                            on_event(WatchEvent::Updated(report));
                        }
                        Err(error) => {
                            // A scan can fail after the OS watch was registered. Keep it for recovery.
                            for active in core
                                .watch_status()?
                                .iter()
                                .filter(|watcher| Path::new(&watcher.path) == root)
                            {
                                keep.insert(active.folder_id);
                            }
                            issues.push(DaemonIssue {
                                source: folder.path.clone(),
                                error: error.to_string(),
                            });
                        }
                    }
                }
            }
            for watcher in core.watch_status()? {
                if stopped.load(Ordering::Relaxed) || !keep.contains(&watcher.folder_id) {
                    core.unwatch_folder(watcher.folder_id)?;
                }
            }
            Ok::<_, Error>(issues)
        })
        .await
        .map_err(task_error)??;

        if self.stopped.load(Ordering::Relaxed) {
            return Err(Error::InvalidInput("daemon has been stopped".into()));
        }

        let start_api = {
            let mut state = self.state.lock().map_err(|_| Error::Poisoned)?;
            if !settings.api.enabled {
                state.api = None;
                state.api_port = None;
                false
            } else {
                state.api.is_none() || state.api_port != Some(settings.api.port)
            }
        };
        if start_api {
            match ApiServer::start(self.core.clone(), settings.api.port).await {
                Ok(server) => {
                    let mut state = self.state.lock().map_err(|_| Error::Poisoned)?;
                    if self.stopped.load(Ordering::Relaxed) {
                        return Err(Error::InvalidInput("daemon has been stopped".into()));
                    }
                    state.api = Some(server);
                    state.api_port = Some(settings.api.port);
                }
                Err(error) => issues.push(DaemonIssue {
                    source: "Local API".into(),
                    error: error.to_string(),
                }),
            }
        }
        {
            let mut state = self.state.lock().map_err(|_| Error::Poisoned)?;
            for issue in &issues {
                if !state.issues.iter().any(|previous| {
                    previous.source == issue.source && previous.error == issue.error
                }) {
                    log::warn!("Daemon issue for {:?}: {}", issue.source, issue.error);
                }
            }
            for previous in &state.issues {
                if !issues.iter().any(|issue| issue.source == previous.source) {
                    log::info!("Daemon issue resolved for {:?}", previous.source);
                }
            }
            state.issues = issues;
        }
        let ids = self
            .core
            .watch_status()?
            .iter()
            .map(|watcher| watcher.folder_id)
            .collect();
        self.extraction.configure(
            self.core.clone(),
            settings.extraction,
            ids,
            settings.monitoring_paused,
            force_retry,
        )?;
        if self.stopped.load(Ordering::Relaxed) {
            self.extraction.stop()?;
            return Err(Error::InvalidInput("daemon has been stopped".into()));
        }
        self.status()
    }

    pub fn stop(&self) -> Result<()> {
        log::info!("Stopping daemon");
        self.stopped.store(true, Ordering::Relaxed);
        self.extraction.stop()?;
        if let Some(supervisor) = self.supervisor.lock().map_err(|_| Error::Poisoned)?.take() {
            supervisor.abort();
        }
        self.state.lock().map_err(|_| Error::Poisoned)?.api = None;
        self.core.stop_watches()?;
        self.core.persist()
    }

    /// Wait for configuration changes and atomic extraction commits before installation.
    pub async fn shutdown(&self) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        let api = self.state.lock().map_err(|_| Error::Poisoned)?.api.take();
        let supervisor = self.supervisor.lock().map_err(|_| Error::Poisoned)?.take();
        self.stop()?;
        if let Some(supervisor) = supervisor {
            supervisor.abort();
            let _ = supervisor.await;
        }
        self.extraction.wait_stopped().await?;
        if let Some(api) = api {
            api.stop().await;
        }
        self.core.persist()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn task_error(error: tokio::task::JoinError) -> Error {
    Error::Io(std::io::Error::other(error.to_string()))
}
