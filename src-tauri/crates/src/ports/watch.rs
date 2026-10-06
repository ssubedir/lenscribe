use crate::{
    domain::{
        model::ScanReport,
        watch::{WatchEvent, WatchStatus},
    },
    Result,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

pub type WatchCallback = Arc<dyn Fn(WatchEvent) + Send + Sync>;

/// Driving port invoked by filesystem event adapters.
pub trait WatchTarget: Send + Sync {
    fn scan_folder(&self, root: &Path) -> Result<ScanReport>;
    fn scan_paths(&self, folder: i64, paths: &[PathBuf]) -> Result<ScanReport>;
    fn reconcile_folder(&self, root: &Path) -> Result<ScanReport>;
}

pub trait FolderWatch: Send {
    fn status(&self) -> Result<WatchStatus>;
}

pub trait Watcher: Send + Sync {
    fn start(
        &self,
        target: Weak<dyn WatchTarget>,
        folder: i64,
        root: PathBuf,
        on_event: WatchCallback,
    ) -> Result<Box<dyn FolderWatch>>;
}
