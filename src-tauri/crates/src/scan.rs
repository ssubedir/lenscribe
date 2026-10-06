use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::{
    merkle::MerkleTree, settings::FolderSettings, trailer, Core, Error, FileDetails, FileRecord,
    Result, ScanIssue, ScanReport,
};

pub(crate) struct FolderRules {
    patterns: Vec<String>,
    globs: GlobSet,
    max_bytes: u64,
}

impl FolderRules {
    pub fn new(settings: &FolderSettings) -> Result<Self> {
        if settings.exclusions.len() > 100 || settings.max_image_mib > 131072 {
            return Err(Error::InvalidInput(
                "Use at most 100 exclusions and a size limit below 131072 MiB".into(),
            ));
        }
        let mut builder = GlobSetBuilder::new();
        for pattern in &settings.exclusions {
            if pattern.trim().is_empty()
                || pattern.len() > 1024
                || pattern.contains('\\')
                || pattern.starts_with('/')
                || pattern.contains(':')
                || pattern.split('/').any(|part| part == "..")
            {
                return Err(Error::InvalidInput("Exclusions must be relative patterns using / separators, such as temp/** or **/*-thumbnail.png".into()));
            }
            let normalized = pattern.trim_end_matches('/');
            let mut patterns = vec![normalized.to_owned()];
            // Bare filenames match at every depth; a directory pattern excludes its descendants.
            if !normalized.contains('/') {
                patterns.push(format!("**/{normalized}"));
            }
            patterns.push(format!("{normalized}/**"));
            if !normalized.contains('/') {
                patterns.push(format!("**/{normalized}/**"));
            }
            for pattern in patterns {
                let glob = GlobBuilder::new(&pattern)
                    .literal_separator(true)
                    .case_insensitive(cfg!(windows))
                    .build()
                    .map_err(|error| Error::InvalidInput(format!("Invalid exclusion: {error}")))?;
                builder.add(glob);
            }
        }
        Ok(Self {
            patterns: settings.exclusions.clone(),
            globs: builder
                .build()
                .map_err(|error| Error::InvalidInput(error.to_string()))?,
            max_bytes: u64::from(settings.max_image_mib) * 1024 * 1024,
        })
    }

    fn same_as(&self, other: &Self) -> bool {
        self.patterns == other.patterns && self.max_bytes == other.max_bytes
    }

    pub fn allows(&self, relative: &str, image_length: u64) -> bool {
        !self.globs.is_match(relative) && (self.max_bytes == 0 || image_length <= self.max_bytes)
    }

    fn excludes(&self, relative: &str) -> bool {
        let mut prefix = String::new();
        for part in relative.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if self.globs.is_match(&prefix) {
                return true;
            }
        }
        false
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Stamp {
    length: u64,
    modified: SystemTime,
}
impl Stamp {
    fn read(path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified()?,
        })
    }
}

struct CachedFile {
    record: FileRecord,
    stamp: Option<Stamp>,
}

pub(crate) struct ScanState {
    rules: FolderRules,
    files: BTreeMap<String, CachedFile>,
    tree: MerkleTree,
    issues: BTreeMap<String, String>,
    initialized: bool,
}

impl Core {
    pub(crate) fn image_stamp(&self, folder_id: i64, relative: &str) -> Result<Stamp> {
        Stamp::read(&self.image_path(folder_id, relative)?)
    }
    pub fn set_folder_rules(&self, path: &Path, settings: &FolderSettings) -> Result<bool> {
        let rules = FolderRules::new(settings)?;
        let _work = self.work.lock().map_err(|_| Error::Poisoned)?;
        let root = super::canonical_folder(path)?;
        let id = self
            .database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .ensure(super::path_string(&root)?)?;
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
                .database
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
            .database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .get(folder_id)?;
        let root = super::canonical_folder(Path::new(&folder.path))?;
        let mut scopes = BTreeSet::new();
        for path in paths {
            let event_path = match canonical_event_path(&root, path) {
                Ok(path) => path,
                Err(_) => return self.scan_scopes_locked(&root, &[], true),
            };
            let plain_root = plain_path(&root);
            let Ok(relative) = event_path.strip_prefix(&plain_root) else {
                return self.scan_scopes_locked(&root, &[], true);
            };
            let relative = super::path_string(relative)?.replace('\\', "/");
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
        let root = super::canonical_folder(path)?;
        let id = self
            .database
            .lock()
            .map_err(|_| Error::Poisoned)?
            .folders()
            .ensure(super::path_string(&root)?)?;
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
            if !scope.is_empty() && (state.rules.excludes(scope) || has_symlink(&root, scope)?) {
                continue;
            }
            let path = root.join(scope);
            match fs::symlink_metadata(&path) {
                Ok(_) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            }
            let walker = walkdir::WalkDir::new(&path)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| {
                    let relative = entry
                        .path()
                        .strip_prefix(&root)
                        .unwrap_or(Path::new(""))
                        .to_string_lossy()
                        .replace('\\', "/");
                    relative.is_empty() || !state.rules.excludes(&relative)
                });
            for entry in walker {
                let entry = entry.map_err(|error| {
                    Error::Io(
                        error
                            .into_io_error()
                            .unwrap_or_else(|| std::io::Error::other("folder traversal failed")),
                    )
                })?;
                if !entry.file_type().is_file() || !trailer::supported_path(entry.path()) {
                    continue;
                }
                let relative = super::path_string(
                    entry
                        .path()
                        .strip_prefix(&root)
                        .map_err(|_| Error::InvalidInput("file escaped its folder".into()))?,
                )?
                .replace('\\', "/");
                if present.contains(&relative) {
                    continue;
                }
                let before = match Stamp::read(entry.path()) {
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
                let image = match trailer::inspect(entry.path()).and_then(|image| {
                    if Stamp::read(entry.path())? != before {
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
                        text_hash: text
                            .as_ref()
                            .map(|value| trailer::hash_bytes(value.as_bytes())),
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
        let mut database = self.database.lock().map_err(|_| Error::Poisoned)?;
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
            let record = database
                .files()
                .by_path(id, &details.file.relative_path)?
                .file;
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
            folder: database.folders().get(id)?,
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

fn plain_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(rest) = value.strip_prefix("\\\\?\\UNC\\") {
            return PathBuf::from(format!("\\\\{rest}"));
        }
        if let Some(rest) = value.strip_prefix("\\\\?\\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

fn canonical_event_path(root: &Path, path: &Path) -> std::io::Result<PathBuf> {
    let root = plain_path(root);
    let path = plain_path(path);
    if path.starts_with(&root) {
        return Ok(path);
    }
    // Resolve aliases only up to the watched root, preserving missing suffixes
    // and symlinks inside the folder so they can still be excluded and pruned.
    // Events can use drive casing or aliases such as macOS /var -> /private/var.
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match ancestor.canonicalize() {
            Ok(canonical) if plain_path(&canonical) == root => {
                return Ok(root.join(path.strip_prefix(ancestor).unwrap()));
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "Event path is outside its watched folder",
    ))
}

fn has_symlink(root: &Path, relative: &str) -> Result<bool> {
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}
