use crate::{domain::settings::FolderSettings, Error, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

pub struct FolderRules {
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

    pub(crate) fn same_as(&self, other: &Self) -> bool {
        self.patterns == other.patterns && self.max_bytes == other.max_bytes
    }

    pub fn allows(&self, relative: &str, image_length: u64) -> bool {
        !self.globs.is_match(relative) && (self.max_bytes == 0 || image_length <= self.max_bytes)
    }

    pub(crate) fn excludes(&self, relative: &str) -> bool {
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
