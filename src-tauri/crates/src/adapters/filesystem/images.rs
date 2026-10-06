use super::trailer;
use crate::{
    domain::{
        image::{path_string, InspectedImage},
        merkle,
        rules::FolderRules,
    },
    ports::images::{FileStamp, ImageFiles, ImagePaths},
    Error, Result,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct LocalImageFiles;

impl ImageFiles for LocalImageFiles {
    fn canonical_folder(&self, path: &Path) -> Result<PathBuf> {
        let root = path.canonicalize()?;
        if !root.is_dir() {
            return Err(Error::InvalidInput("select a directory to index".into()));
        }
        Ok(root)
    }

    fn resolve_image(&self, root: &Path, relative: &str) -> Result<PathBuf> {
        merkle::validate_relative_path(relative)?;
        let root = self.canonical_folder(root)?;
        let candidate = root.join(relative);
        // Check every component before resolving it, so in-folder symlinks cannot hide a write target.
        let mut component = root.clone();
        for part in relative.split('/') {
            component.push(part);
            if fs::symlink_metadata(&component)?.file_type().is_symlink() {
                return Err(Error::InvalidInput(
                    "image paths cannot contain symbolic links".into(),
                ));
            }
        }
        let path = candidate.canonicalize()?;
        if !path.starts_with(&root) || !path.is_file() || !trailer::supported_path(&path) {
            return Err(Error::InvalidInput(
                "image must be a PNG, JPEG, or WebP inside the indexed folder".into(),
            ));
        }
        Ok(path)
    }

    fn event_relative_path(&self, root: &Path, event: &Path) -> Result<String> {
        let event = canonical_event_path(root, event)?;
        let root = plain_path(root);
        let relative = event
            .strip_prefix(root)
            .map_err(|_| Error::InvalidInput("Event path is outside its watched folder".into()))?;
        Ok(path_string(relative)?.replace('\\', "/"))
    }

    fn image_paths<'a>(
        &'a self,
        root: &'a Path,
        scope: &str,
        rules: &'a FolderRules,
    ) -> Result<ImagePaths<'a>> {
        if !scope.is_empty() && (rules.excludes(scope) || has_symlink(root, scope)?) {
            return Ok(Box::new(std::iter::empty()));
        }
        let path = root.join(scope);
        match fs::symlink_metadata(&path) {
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Box::new(std::iter::empty()))
            }
            Err(error) => return Err(error.into()),
        }
        let walker = walkdir::WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_entry(move |entry| {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap_or(Path::new(""))
                    .to_string_lossy()
                    .replace('\\', "/");
                relative.is_empty() || !rules.excludes(&relative)
            });
        Ok(Box::new(walker.filter_map(|entry| match entry {
            Ok(entry) if entry.file_type().is_file() && trailer::supported_path(entry.path()) => {
                Some(Ok(entry.into_path()))
            }
            Ok(_) => None,
            Err(error) => {
                Some(Err(Error::Io(error.into_io_error().unwrap_or_else(|| {
                    std::io::Error::other("folder traversal failed")
                }))))
            }
        })))
    }

    fn stamp(&self, path: &Path) -> Result<FileStamp> {
        let metadata = fs::metadata(path)?;
        Ok(FileStamp {
            length: metadata.len(),
            modified: metadata.modified()?,
        })
    }
    fn inspect(&self, path: &Path) -> Result<InspectedImage> {
        trailer::inspect(path)
    }
    fn original_bytes(&self, path: &Path, expected_hash: &str) -> Result<Vec<u8>> {
        trailer::original_bytes(path, expected_hash)
    }
    fn write_text(
        &self,
        path: &Path,
        expected_hash: &str,
        text: &str,
        processor: &str,
    ) -> Result<InspectedImage> {
        trailer::write_text(path, expected_hash, text, processor)
    }
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
