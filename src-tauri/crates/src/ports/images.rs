use crate::{
    domain::{image::InspectedImage, rules::FolderRules},
    Result,
};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub length: u64,
    pub modified: SystemTime,
}

pub type ImagePaths<'a> = Box<dyn Iterator<Item = Result<PathBuf>> + 'a>;

/// Local path safety, traversal, and trailer I/O are one replaceable boundary.
pub trait ImageFiles: Send + Sync {
    fn canonical_folder(&self, path: &Path) -> Result<PathBuf>;
    fn resolve_image(&self, root: &Path, relative: &str) -> Result<PathBuf>;
    fn event_relative_path(&self, root: &Path, event: &Path) -> Result<String>;
    fn image_paths<'a>(
        &'a self,
        root: &'a Path,
        scope: &str,
        rules: &'a FolderRules,
    ) -> Result<ImagePaths<'a>>;
    fn stamp(&self, path: &Path) -> Result<FileStamp>;
    fn inspect(&self, path: &Path) -> Result<InspectedImage>;
    fn original_bytes(&self, path: &Path, expected_hash: &str) -> Result<Vec<u8>>;
    fn write_text(
        &self,
        path: &Path,
        expected_hash: &str,
        text: &str,
        processor: &str,
    ) -> Result<InspectedImage>;
}
