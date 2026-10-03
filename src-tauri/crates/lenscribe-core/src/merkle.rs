//! A directory Merkle tree. Names are sorted by UTF-8 bytes, and fields are length-prefixed.
//! File and directory nodes use distinct domain prefixes. Absolute paths and times are excluded.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::{Error, Result};

type Hash = [u8; 32];

#[derive(Clone, Copy)]
struct Child {
    kind: u8,
    hash: Hash,
}

#[derive(Clone, Default)]
pub struct MerkleTree {
    directories: BTreeMap<String, BTreeMap<String, Child>>,
}

impl MerkleTree {
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
