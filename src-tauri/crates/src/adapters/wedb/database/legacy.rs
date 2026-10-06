//! One-time, read-only SQLite import. SQLite is never used for normal operation.
use super::{
    jobs::JobState,
    recovery::{EndpointState, FailureState},
    store::{id_key, Change},
};
use crate::{Error, FileRecord, FolderRecord, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
pub(super) fn import(path: &Path) -> Result<Vec<Change>> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    let transaction = connection.unchecked_transaction()?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if !(1..=4).contains(&version) {
        return Err(Error::InvalidInput(
            "unsupported SQLite schema; original database was left untouched".into(),
        ));
    }
    let integrity: String = transaction.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(Error::InvalidInput("SQLite integrity check failed".into()));
    }
    let mut changes = Vec::new();
    let mut folders = transaction.prepare("SELECT id,path,root_hash FROM folders ORDER BY id")?;
    let rows = folders.query_map([], |row| {
        Ok(FolderRecord {
            id: row.get(0)?,
            path: row.get(1)?,
            root_hash: row.get(2)?,
            image_count: 0,
        })
    })?;
    for row in rows {
        let folder = row?;
        changes.push(Change::put(id_key("folders", folder.id), &folder)?);
    }
    let mut files=transaction.prepare("SELECT id,folder_id,relative_path,image_hash,image_length,text_hash,record_hash,processor,text FROM files ORDER BY id")?;
    let rows = files.query_map([], |row| {
        Ok((
            FileRecord {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                relative_path: row.get(2)?,
                image_hash: row.get(3)?,
                image_length: row.get::<_, i64>(4)?.max(0) as u64,
                text_hash: row.get(5)?,
                record_hash: row.get(6)?,
                processor: row.get(7)?,
            },
            row.get::<_, Option<String>>(8)?,
        ))
    })?;
    for row in rows {
        let (file, text) = row?;
        changes.push(Change::put(id_key("files", file.id), &file)?);
        if let Some(text) = text {
            let hash = crate::trailer::hash_bytes(text.as_bytes());
            if file.text_hash.as_deref() != Some(&hash) {
                return Err(Error::InvalidInput("SQLite text checksum mismatch".into()));
            }
            changes.push(Change::put(format!("texts/{hash}"), &text)?);
            if version == 1 {
                if let Some(processor) = &file.processor {
                    changes.extend(super::cache::store_changes(
                        &file.image_hash,
                        processor,
                        &text,
                    )?);
                }
            }
        }
    }
    if version >= 2 {
        let mut cache =
            transaction.prepare("SELECT image_hash,processor,text FROM extraction_cache")?;
        for row in cache.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })? {
            let (image, processor, text) = row?;
            changes.extend(super::cache::store_changes(&image, &processor, &text)?);
        }
        let sql = if version >= 4 {
            "SELECT id,file_id,image_hash,force,ready_at_ms FROM extraction_jobs"
        } else {
            "SELECT id,file_id,image_hash,force,0 FROM extraction_jobs"
        };
        let mut jobs = transaction.prepare(sql)?;
        for row in jobs.query_map([], |row| {
            Ok(JobState {
                id: Some(row.get(0)?),
                file_id: row.get(1)?,
                image_hash: row.get(2)?,
                force: row.get(3)?,
                ready_at_ms: row.get::<_, i64>(4)?.max(0) as u64,
                lease_owner: None,
                lease_until_ms: 0,
            })
        })? {
            let job = row?;
            changes.push(Change::put(id_key("jobs", job.file_id), &job)?);
        }
    }
    if version >= 3 {
        let mut failures=transaction.prepare("SELECT file_id,recovery_id,image_hash,request_id,error,attempts,retry_at_ms FROM extraction_failures")?;
        for row in failures.query_map([], |row| {
            Ok(FailureState {
                file_id: row.get(0)?,
                recovery_id: row.get(1)?,
                image_hash: row.get(2)?,
                request_id: row.get(3)?,
                error: row.get(4)?,
                attempts: row.get(5)?,
                retry_at_ms: row.get::<_, Option<i64>>(6)?.map(|at| at.max(0) as u64),
            })
        })? {
            let failure = row?;
            changes.push(Change::put(
                super::recovery::failure_key(&failure.recovery_id, failure.file_id),
                &failure,
            )?);
        }
        let mut endpoints=transaction.prepare("SELECT recovery_id,retry_at_ms,next_request_ms,blocked_error FROM extraction_endpoints")?;
        for row in endpoints.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                EndpointState {
                    retry_at_ms: row.get::<_, Option<i64>>(1)?.map(|at| at.max(0) as u64),
                    next_request_ms: row.get::<_, i64>(2)?.max(0) as u64,
                    blocked_error: row.get(3)?,
                },
            ))
        })? {
            let (key, endpoint) = row?;
            changes.push(Change::put(format!("endpoints/{key}"), &endpoint)?);
        }
    }
    for (table, sequence) in [
        ("folders", "folder"),
        ("files", "file"),
        ("extraction_jobs", "job"),
    ] {
        let id: i64 = transaction.query_row(
            "SELECT coalesce(max(seq),0) FROM sqlite_sequence WHERE name=?1",
            [table],
            |row| row.get(0),
        )?;
        changes.push(Change::put(format!("seq/{sequence}"), &id)?);
    }
    // Normalize duplicate immutable text keys before validation/import.
    let mut records = std::collections::BTreeMap::new();
    records.insert("schema".to_owned(), serde_json::json!(super::store::SCHEMA));
    for change in &changes {
        if let Some(bytes) = &change.value {
            records.insert(
                change.key.clone(),
                serde_json::from_slice::<serde_json::Value>(bytes)?,
            );
        }
    }
    super::maintenance::validate(&records.into_iter().collect::<Vec<_>>())?;
    Ok(changes)
}
