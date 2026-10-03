use rusqlite::Row;

use crate::{FileRecord, FolderRecord};

// Keep column ordering and decoding shared across file, search, and job queries.
pub(super) const FILE_COLUMNS: &str = "f.id, f.folder_id, f.relative_path, f.image_hash, f.image_length, f.text_hash, f.record_hash, f.processor";

pub(super) fn file_row(row: &Row<'_>) -> rusqlite::Result<FileRecord> {
    Ok(FileRecord {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        relative_path: row.get(2)?,
        image_hash: row.get(3)?,
        image_length: row.get::<_, i64>(4)? as u64,
        text_hash: row.get(5)?,
        record_hash: row.get(6)?,
        processor: row.get(7)?,
    })
}

pub(super) fn folder_row(row: &Row<'_>) -> rusqlite::Result<FolderRecord> {
    Ok(FolderRecord {
        id: row.get(0)?,
        path: row.get(1)?,
        root_hash: row.get(2)?,
        image_count: row.get::<_, i64>(3)? as usize,
    })
}
