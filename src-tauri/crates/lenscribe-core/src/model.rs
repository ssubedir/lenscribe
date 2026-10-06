use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FolderRecord {
    pub id: i64,
    pub path: String,
    pub root_hash: String,
    pub image_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FolderProgress {
    #[serde(flatten)]
    pub folder: FolderRecord,
    pub pending_images: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub id: i64,
    pub folder_id: i64,
    pub relative_path: String,
    pub image_hash: String,
    pub image_length: u64,
    pub text_hash: Option<String>,
    pub record_hash: String,
    pub processor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FileDetails {
    #[serde(flatten)]
    pub file: FileRecord,
    pub text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FolderSnapshot {
    pub folder: FolderRecord,
    pub files: Vec<FileRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ScanIssue {
    pub path: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub folder: FolderRecord,
    pub changed: usize,
    pub removed: usize,
    pub inspected: usize,
    pub issues: Vec<ScanIssue>,
}

#[derive(Clone, Debug)]
pub struct ExtractionJob {
    pub file: FileRecord,
    pub request_id: Option<i64>,
    pub force: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FilePage {
    pub files: Vec<FileRecord>,
    pub total: usize,
    pub notice: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub total: usize,
    pub fuzzy_applied: bool,
    pub notice: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceStatus {
    pub indexed_files: usize,
    pub cached_extractions: usize,
    pub unused_cached_extractions: usize,
    pub cache_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceReport {
    pub scanned_folders: usize,
    pub changed_files: usize,
    pub removed_files: usize,
    pub issues: Vec<ScanIssue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    #[serde(flatten)]
    pub file: FileRecord,
    pub folder_path: String,
    pub snippet: String,
}

/// A provider consumes these bytes and returns text plus a stable model/settings identifier.
/// Save its response with Core::attach_text, passing this same image_hash.
#[derive(Debug)]
pub struct PreparedImage {
    pub folder_id: i64,
    pub relative_path: String,
    pub image_hash: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
