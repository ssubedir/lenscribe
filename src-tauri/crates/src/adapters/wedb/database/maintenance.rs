use super::{
    cache::CacheEntry,
    jobs::SavedResult,
    store::{Change, Store, SCHEMA},
    Database,
};
use crate::{Error, MaintenanceStatus, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
};
#[derive(Serialize, Deserialize)]
struct Backup {
    format: String,
    version: u32,
    checksum: String,
    records: Vec<(String, serde_json::Value)>,
}
pub(crate) struct MaintenanceRepository<'a> {
    database: &'a Database,
}
impl<'a> MaintenanceRepository<'a> {
    pub(super) fn new(database: &'a Database) -> Self {
        Self { database }
    }
    fn references(&self) -> BTreeSet<(String, String)> {
        self.database
            .state
            .borrow()
            .files
            .values()
            .filter_map(|file| {
                file.processor
                    .as_ref()
                    .map(|processor| (file.image_hash.clone(), processor.clone()))
            })
            .collect()
    }
    pub fn status(&self) -> Result<MaintenanceStatus> {
        let cache = self.database.store.scan::<CacheEntry>("cache/")?;
        let references = self.references();
        let mut body_lengths = BTreeMap::new();
        let mut bytes = 0;
        for (_, entry) in &cache {
            let length = if let Some(length) = body_lengths.get(&entry.text_hash) {
                *length
            } else {
                let length = self
                    .database
                    .store
                    .get::<String>(&format!("texts/{}", entry.text_hash))?
                    .ok_or_else(|| Error::Storage("missing cached text body".into()))?
                    .len() as u64;
                body_lengths.insert(entry.text_hash.clone(), length);
                length
            };
            bytes += length;
        }
        Ok(MaintenanceStatus {
            indexed_files: self.database.state.borrow().files.len(),
            cached_extractions: cache.len(),
            unused_cached_extractions: cache
                .iter()
                .filter(|(_, entry)| {
                    !references.contains(&(entry.image_hash.clone(), entry.processor.clone()))
                })
                .count(),
            cache_bytes: bytes,
        })
    }
    pub fn cleanup_cache(&self) -> Result<usize> {
        let cache = self.database.store.scan::<CacheEntry>("cache/")?;
        let references = self.references();
        let mut changes = Vec::new();
        let mut retained = BTreeSet::new();
        let mut removed = 0;
        for (key, entry) in cache {
            if references.contains(&(entry.image_hash.clone(), entry.processor.clone())) {
                retained.insert(entry.text_hash);
            } else {
                changes.push(Change::remove(key));
                removed += 1;
            }
        }
        retained.extend(
            self.database
                .state
                .borrow()
                .files
                .values()
                .filter_map(|file| file.text_hash.clone()),
        );
        retained.extend(
            self.database
                .store
                .scan::<SavedResult>("results/")?
                .into_iter()
                .map(|(_, result)| result.text_hash),
        );
        for (key, _) in self.database.store.scan::<String>("texts/")? {
            if !retained.contains(key.trim_start_matches("texts/")) {
                changes.push(Change::remove(key));
            }
        }
        self.database.commit(changes)?;
        Ok(removed)
    }
    pub fn backup(&self, destination: &Path) -> Result<()> {
        if destination.exists() {
            return Err(Error::InvalidInput(
                "choose a new backup filename; destination already exists".into(),
            ));
        }
        let records = self.database.store.scan::<serde_json::Value>("")?;
        let checksum = crate::trailer::hash_bytes(&serde_json::to_vec(&records)?);
        let backup = Backup {
            format: "lenscribe-backup".into(),
            version: SCHEMA,
            checksum,
            records,
        };
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut temporary, &backup)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(destination)
            .map_err(|error| Error::Io(error.error))?;
        Ok(())
    }
}
pub(crate) fn restore(backup: &Path, destination: &Path) -> Result<()> {
    if destination == Path::new(":memory:") || destination.exists() {
        return Err(Error::InvalidInput(
            "restore requires a new storage directory".into(),
        ));
    }
    let input: Backup = serde_json::from_reader(std::fs::File::open(backup)?)?;
    if input.format != "lenscribe-backup"
        || input.version != SCHEMA
        || crate::trailer::hash_bytes(&serde_json::to_vec(&input.records)?) != input.checksum
    {
        return Err(Error::InvalidInput(
            "unsupported or damaged Lenscribe backup".into(),
        ));
    }
    validate(&input.records)?;
    let changes = input
        .records
        .into_iter()
        .map(|(key, value)| Change::put(key, &value))
        .collect::<Result<Vec<_>>>()?;
    // Reserve the destination atomically so an existing store can never be merged
    // into or overwritten, including if another process creates it during validation.
    if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(destination)?;
    // A single batch includes the schema marker: an interrupted restore cannot look complete.
    let store = Store::open(destination)?;
    store.commit(&changes)?;
    let state = super::store::State::load(&store)?;
    super::search::SearchIndex::load(&store, &state)?;
    Ok(())
}
pub(super) fn validate(records: &[(String, serde_json::Value)]) -> Result<()> {
    let mut map = std::collections::BTreeMap::new();
    for (key, value) in records {
        if map.insert(key.as_str(), value).is_some() {
            return Err(Error::InvalidInput("duplicate backup record".into()));
        }
        if key != "schema"
            && ![
                "seq/",
                "folders/",
                "files/",
                "texts/",
                "cache/",
                "jobs/",
                "results/",
                "failures/",
                "endpoints/",
                "merkle/",
            ]
            .iter()
            .any(|prefix| key.starts_with(prefix))
        {
            return Err(Error::InvalidInput("unknown backup record".into()));
        }
    }
    if map.get("schema").and_then(|value| value.as_u64()) != Some(SCHEMA as u64) {
        return Err(Error::InvalidInput("unsupported storage schema".into()));
    }
    let mut paths = BTreeSet::new();
    let mut folder_paths = BTreeSet::new();
    let mut maxima = [0_i64; 3];
    let missing_body = |hash: &str| -> Result<()> {
        if !map.contains_key(format!("texts/{hash}").as_str()) {
            return Err(Error::InvalidInput("missing referenced text body".into()));
        }
        Ok(())
    };
    for (key, value) in records {
        if key.starts_with("folders/") {
            let folder: crate::FolderRecord = serde_json::from_value(value.clone())?;
            if folder.id <= 0
                || key != &super::store::id_key("folders", folder.id)
                || !folder_paths.insert(folder.path)
            {
                return Err(Error::InvalidInput("invalid folder record".into()));
            }
            maxima[0] = maxima[0].max(folder.id);
        }
        if let Some(hash) = key.strip_prefix("texts/") {
            let text = value
                .as_str()
                .ok_or_else(|| Error::InvalidInput("invalid text body".into()))?;
            if crate::trailer::hash_bytes(text.as_bytes()) != hash {
                return Err(Error::InvalidInput("text body checksum mismatch".into()));
            }
        }
        if key.starts_with("files/") {
            let file: crate::FileRecord = serde_json::from_value(value.clone())?;
            if file.id <= 0
                || !paths.insert((file.folder_id, file.relative_path.clone()))
                || file.processor.is_some() != file.text_hash.is_some()
                || key != &super::store::id_key("files", file.id)
                || !map.contains_key(super::store::id_key("folders", file.folder_id).as_str())
            {
                return Err(Error::InvalidInput("invalid file reference".into()));
            }
            maxima[1] = maxima[1].max(file.id);
            crate::merkle::validate_relative_path(&file.relative_path)?;
            let text = file
                .text_hash
                .as_ref()
                .map(|hash| {
                    map.get(format!("texts/{hash}").as_str())
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| Error::InvalidInput("missing text body".into()))
                })
                .transpose()?;
            if crate::merkle::record_hash(&file.image_hash, text.zip(file.processor.as_deref()))?
                != file.record_hash
            {
                return Err(Error::InvalidInput("file record checksum mismatch".into()));
            }
        }
        if key.starts_with("cache/") {
            let entry: CacheEntry = serde_json::from_value(value.clone())?;
            if key != &super::cache::cache_key(&entry.image_hash, &entry.processor) {
                return Err(Error::InvalidInput("invalid cache reference".into()));
            }
            missing_body(&entry.text_hash)?;
        }
        if key.starts_with("jobs/") {
            let job: super::jobs::JobState = serde_json::from_value(value.clone())?;
            if key != &super::store::id_key("jobs", job.file_id)
                || !map.contains_key(super::store::id_key("files", job.file_id).as_str())
                || job.id.is_some_and(|id| id <= 0)
            {
                return Err(Error::InvalidInput("invalid queued job".into()));
            }
            maxima[2] = maxima[2].max(job.id.unwrap_or(0));
        }
        if key.starts_with("results/") {
            let result: SavedResult = serde_json::from_value(value.clone())?;
            if key != &super::store::id_key("results", result.file_id)
                || !map.contains_key(super::store::id_key("files", result.file_id).as_str())
            {
                return Err(Error::InvalidInput("invalid saved result".into()));
            }
            missing_body(&result.text_hash)?;
        }
        if key.starts_with("failures/") {
            let failure: super::recovery::FailureState = serde_json::from_value(value.clone())?;
            if key != &super::recovery::failure_key(&failure.recovery_id, failure.file_id)
                || !map.contains_key(super::store::id_key("files", failure.file_id).as_str())
            {
                return Err(Error::InvalidInput("invalid recovery record".into()));
            }
        }
        if key.starts_with("endpoints/") {
            let _: super::recovery::EndpointState = serde_json::from_value(value.clone())?;
        }
        if let Some(rest) = key.strip_prefix("merkle/") {
            let (folder, path) = rest
                .split_once('/')
                .ok_or_else(|| Error::InvalidInput("invalid Merkle key".into()))?;
            let id = folder
                .parse::<i64>()
                .map_err(|_| Error::InvalidInput("invalid Merkle folder".into()))?;
            if !map.contains_key(super::store::id_key("folders", id).as_str()) {
                return Err(Error::InvalidInput(
                    "invalid Merkle folder reference".into(),
                ));
            }
            if !path.is_empty() {
                crate::merkle::validate_relative_path(path)?;
            }
            let _: std::collections::BTreeMap<String, crate::merkle::Child> =
                serde_json::from_value(value.clone())?;
        }
    }
    for (index, name) in ["folder", "file", "job"].into_iter().enumerate() {
        if map
            .get(format!("seq/{name}").as_str())
            .and_then(|value| value.as_i64())
            .unwrap_or(0)
            < maxima[index]
        {
            return Err(Error::InvalidInput("invalid ID sequence".into()));
        }
    }
    Ok(())
}
