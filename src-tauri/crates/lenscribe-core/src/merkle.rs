//! A directory Merkle tree. Names are sorted by UTF-8 bytes, and fields are length-prefixed.
//! File and directory nodes use distinct domain prefixes. Absolute paths and times are excluded.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::{Error, Result};

type Hash = [u8; 32];

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct Child {
    kind: u8,
    hash: Hash,
}

#[derive(Clone, Default)]
pub struct MerkleTree {
    directories: BTreeMap<String, BTreeMap<String, Child>>,
}

impl MerkleTree {
    /// Persist only directories on the changed paths, including tombstones for
    /// directories that became empty. File bytes and timestamps are excluded.
    pub(crate) fn checkpoints(
        &self,
        paths: &[String],
    ) -> BTreeMap<String, Option<BTreeMap<String, Child>>> {
        let mut directories = std::collections::BTreeSet::from([String::new()]);
        for path in paths {
            let mut parent = split_path(path).0;
            loop {
                directories.insert(parent.to_owned());
                if parent.is_empty() {
                    break;
                }
                parent = split_path(parent).0;
            }
        }
        directories
            .into_iter()
            .map(|path| {
                let children = self.directories.get(&path).cloned();
                (path, children)
            })
            .collect()
    }

    pub(crate) fn from_checkpoints(
        directories: BTreeMap<String, BTreeMap<String, Child>>,
        records: &[crate::FileRecord],
        root: &str,
    ) -> Result<Self> {
        let tree = Self { directories };
        let mut leaves = BTreeMap::new();
        for (path, children) in &tree.directories {
            if !path.is_empty() {
                validate_relative_path(path)?;
                let (parent, name) = split_path(path);
                if tree
                    .directories
                    .get(parent)
                    .and_then(|children| children.get(name))
                    .is_none_or(|child| {
                        child.kind != 1 || child.hash != directory_hash(Some(children))
                    })
                {
                    return Err(Error::InvalidInput("disconnected Merkle checkpoint".into()));
                }
            }
            for (name, child) in children {
                if name.contains('/') {
                    return Err(Error::InvalidInput("invalid Merkle child name".into()));
                }
                validate_relative_path(name)?;
                let full = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}/{name}")
                };
                match child.kind {
                    0 => {
                        leaves.insert(full, hex::encode(child.hash));
                    }
                    1 => {
                        if !tree.directories.contains_key(&full)
                            || directory_hash(tree.directories.get(&full)) != child.hash
                        {
                            return Err(Error::InvalidInput(
                                "invalid Merkle directory checkpoint".into(),
                            ));
                        }
                    }
                    _ => return Err(Error::InvalidInput("invalid Merkle node type".into())),
                }
            }
        }
        if tree.root_hash() != root
            || leaves.len() != records.len()
            || records
                .iter()
                .any(|record| leaves.get(&record.relative_path) != Some(&record.record_hash))
        {
            return Err(Error::InvalidInput(
                "Merkle checkpoint does not match canonical files".into(),
            ));
        }
        Ok(tree)
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn root_hash(&self) -> String {
        hex::encode(directory_hash(self.directories.get("")))
    }

    pub fn insert(&mut self, relative_path: &str, record_hash: &str) -> Result<()> {
        validate_relative_path(relative_path)?;
        let hash: Hash = hex::decode(record_hash)
            .map_err(|_| Error::InvalidInput("invalid record hash".into()))?
            .try_into()
            .map_err(|_| Error::InvalidInput("record hash must be SHA-256".into()))?;
        let (parent, name) = split_path(relative_path);
        self.directories
            .entry(parent.into())
            .or_default()
            .insert(name.into(), Child { kind: 0, hash });
        self.update_ancestors(parent);
        Ok(())
    }

    pub fn remove(&mut self, relative_path: &str) -> Result<()> {
        validate_relative_path(relative_path)?;
        let (parent, name) = split_path(relative_path);
        if let Some(children) = self.directories.get_mut(parent) {
            children.remove(name);
        }
        self.update_ancestors(parent);
        Ok(())
    }

    fn update_ancestors(&mut self, mut directory: &str) {
        loop {
            if directory.is_empty() {
                break;
            }
            let (parent, name) = split_path(directory);
            let empty = self
                .directories
                .get(directory)
                .is_none_or(BTreeMap::is_empty);
            if empty {
                self.directories.remove(directory);
                if let Some(children) = self.directories.get_mut(parent) {
                    children.remove(name);
                }
            } else {
                let hash = directory_hash(self.directories.get(directory));
                self.directories
                    .entry(parent.into())
                    .or_default()
                    .insert(name.into(), Child { kind: 1, hash });
            }
            directory = parent;
        }
    }
}

pub fn record_hash(image_hash: &str, text: Option<(&str, &str)>) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"lenscribe:file:v1\0");
    field(&mut hasher, image_hash.as_bytes());
    match text {
        None => hasher.update([0]),
        Some((text, processor)) => {
            hasher.update([1]);
            field(&mut hasher, Sha256::digest(text.as_bytes()).as_slice());
            field(&mut hasher, processor.as_bytes());
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn validate_relative_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(Error::InvalidInput(
            "use a relative path with '/' separators and no '.' or '..' components".into(),
        ));
    }
    Ok(())
}

fn directory_hash(children: Option<&BTreeMap<String, Child>>) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(b"lenscribe:directory:v1\0");
    for (name, child) in children.into_iter().flatten() {
        field(&mut hasher, name.as_bytes());
        hasher.update([child.kind]);
        hasher.update(child.hash);
    }
    hasher.finalize().into()
}

fn field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}
