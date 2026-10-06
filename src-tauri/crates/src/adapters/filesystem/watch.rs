use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, Weak,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::{
    domain::watch::{WatchEvent, WatchFailure, WatchStatus},
    ports::watch::{WatchCallback, WatchTarget, Watcher},
    Error, Result,
};

pub(crate) struct FolderWatch {
    _watcher: RecommendedWatcher,
    status: Arc<Mutex<WatchStatus>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FolderWatch {
    pub fn start(
        core: Weak<dyn WatchTarget>,
        folder_id: i64,
        root: PathBuf,
        on_event: Arc<dyn Fn(WatchEvent) + Send + Sync>,
    ) -> Result<Self> {
        let status = Arc::new(Mutex::new(WatchStatus {
            folder_id,
            path: root.to_string_lossy().into_owned(),
            last_error: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        // A bounded wakeup channel plus a bounded path set coalesces bursts without losing paths.
        let (sender, receiver) = mpsc::sync_channel(1);
        let pending = Arc::new(Mutex::new((BTreeSet::<PathBuf>::new(), false)));
        let callback_pending = pending.clone();
        let callback_status = status.clone();
        let callback_events = on_event.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                match event {
                    Ok(event)
                        if matches!(event.kind, EventKind::Access(_)) && !event.need_rescan() =>
                    {
                        return
                    }
                    Ok(event) => {
                        if let Ok(mut pending) = callback_pending.lock() {
                            if event.need_rescan() || event.paths.is_empty() {
                                pending.1 = true;
                            }
                            if !pending.1 {
                                pending.0.extend(event.paths);
                            }
                            if pending.0.len() > 1024 {
                                pending.1 = true;
                            }
                            if pending.1 {
                                pending.0.clear();
                            }
                        }
                    }
                    Err(error) => {
                        let error = error.to_string();
                        log::warn!("Folder {} watch error: {}", folder_id, error);
                        if let Ok(mut status) = callback_status.lock() {
                            status.last_error = Some(error.clone());
                        }
                        callback_events(WatchEvent::Failed(WatchFailure { folder_id, error }));
                        if let Ok(mut pending) = callback_pending.lock() {
                            pending.1 = true;
                            pending.0.clear();
                        }
                    }
                }
                let _ = sender.try_send(());
            })?;
        watcher.watch(&root, RecursiveMode::Recursive)?;
        log::info!("Folder {} watch registered: {:?}", folder_id, root);
        let worker_stop = stop.clone();
        let worker_status = status.clone();
        let worker = thread::Builder::new()
            .name(format!("lenscribe-watch-{folder_id}"))
            .spawn(move || {
                let mut dirty_since = None;
                let mut last_event = Instant::now();
                let mut last_scan = Instant::now();
                while !worker_stop.load(Ordering::Relaxed) {
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(()) => {
                            let now = Instant::now();
                            dirty_since.get_or_insert(now);
                            last_event = now;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => (),
                    }
                    let ready = dirty_since.is_some_and(|first: Instant| {
                        last_event.elapsed() >= Duration::from_millis(500)
                            || first.elapsed() >= Duration::from_secs(2)
                    });
                    if !ready && last_scan.elapsed() < Duration::from_secs(30) {
                        continue;
                    }
                    let Some(core) = core.upgrade() else {
                        break;
                    };
                    let (paths, full) = match pending.lock() {
                        Ok(mut pending) => std::mem::take(&mut *pending),
                        Err(_) => break,
                    };
                    let result = if full {
                        core.scan_folder(&root)
                    } else if ready && !paths.is_empty() {
                        core.scan_paths(folder_id, &paths.into_iter().collect::<Vec<_>>())
                    } else {
                        core.reconcile_folder(&root)
                    };
                    match result {
                        Ok(report) => {
                            if let Ok(mut status) = worker_status.lock() {
                                status.last_error = report
                                    .issues
                                    .first()
                                    .map(|issue| format!("{}: {}", issue.path, issue.error));
                            }
                            if report.changed > 0 || report.removed > 0 || !report.issues.is_empty()
                            {
                                log::info!(
                                    "Folder {} updated: indexed={}, changed={}, removed={}, inspected={}, issues={}",
                                    folder_id, report.folder.image_count, report.changed, report.removed, report.inspected, report.issues.len()
                                );
                                for issue in &report.issues {
                                    log::warn!("Folder {} file {:?}: {}", folder_id, issue.path, issue.error);
                                }
                                on_event(WatchEvent::Updated(report));
                            }
                        }
                        Err(error) => {
                            let error = error.to_string();
                            log::warn!("Folder {} scan failed: {}", folder_id, error);
                            if let Ok(mut status) = worker_status.lock() {
                                status.last_error = Some(error.clone());
                            }
                            on_event(WatchEvent::Failed(WatchFailure { folder_id, error }));
                        }
                    }
                    dirty_since = None;
                    last_scan = Instant::now();
                }
            })?;
        Ok(Self {
            _watcher: watcher,
            status,
            stop,
            thread: Some(worker),
        })
    }

    pub fn status(&self) -> Result<WatchStatus> {
        Ok(self.status.lock().map_err(|_| Error::Poisoned)?.clone())
    }
}

impl Drop for FolderWatch {
    fn drop(&mut self) {
        if let Ok(status) = self.status.lock() {
            log::info!("Folder {} watch stopped", status.folder_id);
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // The last Core reference can be released by this worker; never join the current thread.
            if thread.thread().id() != thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

pub struct NotifyWatcher;
impl Watcher for NotifyWatcher {
    fn start(
        &self,
        target: Weak<dyn WatchTarget>,
        folder: i64,
        root: PathBuf,
        on_event: WatchCallback,
    ) -> Result<Box<dyn crate::ports::watch::FolderWatch>> {
        Ok(Box::new(FolderWatch::start(
            target, folder, root, on_event,
        )?))
    }
}
impl crate::ports::watch::FolderWatch for FolderWatch {
    fn status(&self) -> Result<WatchStatus> {
        FolderWatch::status(self)
    }
}
impl From<notify::Error> for Error {
    fn from(error: notify::Error) -> Self {
        Self::Watch(error.to_string())
    }
}
