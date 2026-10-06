use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use crate::{
    domain::{
        image::{hash_bytes, path_string},
        merkle::MerkleTree,
        rules::FolderRules,
        settings::FolderSettings,
    },
    ports::images::FileStamp,
    Core, Error, FileDetails, FileRecord, Result, ScanIssue, ScanReport,
};

struct CachedFile {
    record: FileRecord,
    stamp: Option<FileStamp>,
}

pub(crate) struct ScanState {
    rules: FolderRules,
    files: BTreeMap<String, CachedFile>,
    tree: MerkleTree,
    issues: BTreeMap<String, String>,
    initialized: bool,
}

impl Core {
    pub(crate) fn image_stamp(&self, folder_id: i64, relative: &str) -> Result<FileStamp> {
        self.images.stamp(&self.image_path(folder_id, relative)?)
    }
    pub fn set_folder_rules(&self, path: &Path, settings: &FolderSettings) -> Result<bool> {
        let rules = FolderRules::new(settings)?;
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        let root = self.images.canonical_folder(path)?;
        let id = self
            .index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .ensure_folder(path_string(&root)?)?;
        let mut states = self.scan_state.lock().map_err(|_| Error::Poisoned)?;
        self.ensure_scan_state(&mut states, id)?;
        let state = states.get_mut(&id).unwrap();
        let changed = !state.rules.same_as(&rules);
        state.rules = rules;
        Ok(changed)
    }

    fn ensure_scan_state(&self, states: &mut BTreeMap<i64, ScanState>, id: i64) -> Result<()> {
        if let std::collections::btree_map::Entry::Vacant(entry) = states.entry(id) {
            let snapshot = self.snapshot(id)?;
            let checkpoint = self
                .index
                .lock()
                .map_err(|_| Error::Poisoned)?
                .merkle_checkpoint(id, &snapshot.files, &snapshot.folder.root_hash)?;
            let restored = checkpoint.is_some();
            let mut tree = checkpoint.unwrap_or_default();
            let mut files = BTreeMap::new();
            for record in snapshot.files {
                if !restored {
                    tree.insert(&record.relative_path, &record.record_hash)?;
                }
                files.insert(
                    record.relative_path.clone(),
                    CachedFile {
                        record,
                        stamp: None,
                    },
                );
            }
            entry.insert(ScanState {
                rules: FolderRules::new(&FolderSettings::default())?,
                files,
                tree,
                issues: BTreeMap::new(),
                initialized: false,
            });
        }
        Ok(())
    }

    /// Metadata reconciliation catches missed events without rehashing unchanged images.
    pub fn reconcile_folder(&self, path: impl AsRef<Path>) -> Result<ScanReport> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        self.scan_scopes_locked(path.as_ref(), &[], false)
    }

    /// Events are hints about paths, not instructions to trust their create/delete ordering.
    pub fn scan_paths(&self, folder_id: i64, paths: &[PathBuf]) -> Result<ScanReport> {
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        let folder = self
            .index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folder(folder_id)?;
        let root = self.images.canonical_folder(Path::new(&folder.path))?;
        let mut scopes = BTreeSet::new();
        for path in paths {
            let relative = match self.images.event_relative_path(&root, path) {
                Ok(relative) => relative,
                Err(_) => return self.scan_scopes_locked(&root, &[], true),
            };
            if relative.is_empty() {
                scopes.insert(relative);
                continue;
            }
            crate::merkle::validate_relative_path(&relative)?;
            scopes.insert(relative);
        }
        self.scan_scopes_locked(&root, &scopes.into_iter().collect::<Vec<_>>(), true)
    }

    pub(crate) fn allowed_image(
        &self,
        folder_id: i64,
        relative: &str,
        length: u64,
    ) -> Result<bool> {
        let states = self.scan_state.lock().map_err(|_| Error::Poisoned)?;
        Ok(states.get(&folder_id).is_none_or(|state| {
            !state.rules.excludes(relative) && state.rules.allows(relative, length)
        }))
    }

    pub(crate) fn scan_locked(&self, path: &Path) -> Result<ScanReport> {
        self.scan_scopes_locked(path, &[], true)
    }

    pub(crate) fn scan_scopes_locked(
        &self,
        path: &Path,
        requested: &[String],
        force: bool,
    ) -> Result<ScanReport> {
        let root = self.images.canonical_folder(path)?;
        let id = self
            .index
            .lock()
            .map_err(|_| Error::Poisoned)?
            .ensure_folder(path_string(&root)?)?;
        let mut states = self.scan_state.lock().map_err(|_| Error::Poisoned)?;
        self.ensure_scan_state(&mut states, id)?;
        let state = states.get_mut(&id).unwrap();
        let scopes = if !state.initialized || requested.is_empty() {
            vec![String::new()]
        } else {
            requested
                .iter()
                .filter(|path| {
                    !requested
                        .iter()
                        .any(|ancestor| ancestor != *path && under(path, ancestor))
                })
                .cloned()
                .collect()
        };
        let mut present = BTreeSet::new();
        let mut upserts = vec![];
        let mut stamps = vec![];
        let mut issues = BTreeMap::new();
        let mut inspected = 0;
        for scope in &scopes {
            let walker = self.images.image_paths(&root, scope, &state.rules)?;
            for path in walker {
                let path = path?;
                let relative = path_string(
                    path.strip_prefix(&root)
                        .map_err(|_| Error::InvalidInput("file escaped its folder".into()))?,
                )?
                .replace('\\', "/");
                if present.contains(&relative) {
                    continue;
                }
                let before = match self.images.stamp(&path) {
                    Ok(stamp) => stamp,
                    Err(error) => {
                        if error.is_transient_read() && state.files.contains_key(&relative) {
                            present.insert(relative.clone());
                        }
                        issues.insert(relative, error.to_string());
                        continue;
                    }
                };
                let force_file = force
                    && (requested.is_empty() || requested.iter().any(|path| path == &relative));
                if !force_file
                    && state.files.get(&relative).is_some_and(|cached| {
                        cached.stamp.as_ref() == Some(&before)
                            && state.rules.allows(&relative, cached.record.image_length)
                    })
                {
                    present.insert(relative);
                    continue;
                }
                inspected += 1;
                let image = match self.images.inspect(&path).and_then(|image| {
                    if self.images.stamp(&path)? != before {
                        return Err(Error::ImageChanged);
                    }
                    Ok(image)
                }) {
                    Ok(image) => image,
                    Err(error) => {
                        if error.is_transient_read() && state.files.contains_key(&relative) {
                            present.insert(relative.clone());
                        }
                        issues.insert(relative, error.to_string());
                        continue;
                    }
                };
                if !state.rules.allows(&relative, image.image_length) {
                    continue;
                }
                let text = image.trailer.as_ref().map(|value| value.text.clone());
                let processor = image.trailer.as_ref().map(|value| value.processor.clone());
                let record_hash = crate::merkle::record_hash(
                    &image.image_hash,
                    text.as_deref().zip(processor.as_deref()),
                )?;
                let details = FileDetails {
                    file: FileRecord {
                        id: 0,
                        folder_id: id,
                        relative_path: relative.clone(),
                        image_hash: image.image_hash,
                        image_length: image.image_length,
                        text_hash: text.as_ref().map(|value| hash_bytes(value.as_bytes())),
                        record_hash,
                        processor,
                    },
                    text,
                };
                if state
                    .files
                    .get(&relative)
                    .is_none_or(|cached| cached.record.record_hash != details.file.record_hash)
                {
                    upserts.push(details);
                }
                stamps.push((relative.clone(), before));
                present.insert(relative);
            }
        }
        let removed: Vec<_> = state
            .files
            .keys()
            .filter(|path| {
                scopes.iter().any(|scope| under(path, scope)) && !present.contains(*path)
            })
            .cloned()
            .collect();
        for details in &upserts {
            state
                .tree
                .insert(&details.file.relative_path, &details.file.record_hash)?;
        }
        for path in &removed {
            state.tree.remove(path)?;
        }
        let root_hash = state.tree.root_hash();
        let mut database = self.index.lock().map_err(|_| Error::Poisoned)?;
        if let Err(error) =
            database.apply_scan(id, &upserts, &removed, &root_hash, Some(&state.tree))
        {
            // Restore the old tree, retaining exclusions even when a transaction fails.
            state.tree = MerkleTree::new();
            for cached in state.files.values() {
                state
                    .tree
                    .insert(&cached.record.relative_path, &cached.record.record_hash)?;
            }
            state.initialized = false;
            return Err(error);
        }
        for path in &removed {
            state.files.remove(path);
        }
        for details in &upserts {
            let record = database.file_by_path(id, &details.file.relative_path)?.file;
            state.files.insert(
                record.relative_path.clone(),
                CachedFile {
                    record,
                    stamp: None,
                },
            );
        }
        for (path, stamp) in stamps {
            if let Some(cached) = state.files.get_mut(&path) {
                cached.stamp = Some(stamp);
            }
        }
        state
            .issues
            .retain(|path, _| !scopes.iter().any(|scope| under(path, scope)));
        state.issues.extend(issues);
        state.initialized = true;
        Ok(ScanReport {
            folder: database.folder(id)?,
            changed: upserts.len(),
            removed: removed.len(),
            inspected,
            issues: state
                .issues
                .iter()
                .map(|(path, error)| ScanIssue {
                    path: path.clone(),
                    error: error.clone(),
                })
                .collect(),
        })
    }
}

fn under(path: &str, scope: &str) -> bool {
    scope.is_empty()
        || path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with('/'))
}
