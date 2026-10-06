use crate::{domain::settings::Settings, ports::settings::SettingsStore, Error, Result};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub struct JsonSettingsStore {
    path: PathBuf,
}
impl JsonSettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}
impl SettingsStore for JsonSettingsStore {
    fn load(&self) -> Result<Settings> {
        Settings::load(&self.path)
    }
    fn save(&self, settings: &Settings) -> Result<()> {
        settings.save(&self.path)
    }
}

// Compatibility convenience methods delegate to the filesystem adapter.
impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error.into()),
        };
        let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if let Some(extraction) = value
            .get_mut("extraction")
            .and_then(serde_json::Value::as_object_mut)
        {
            // Discard the old variable-name field without reading or importing its value.
            // Authenticated setups need the key entered in Settings before extraction resumes.
            if extraction
                .remove("apiKeyEnv")
                .is_some_and(|legacy| legacy.as_str().is_some_and(|name| !name.is_empty()))
            {
                extraction.insert("enabled".into(), false.into());
            }
        }
        let settings: Self = serde_json::from_value(value)?;
        validate_paths(&settings)?;
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        validate_paths(self)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".lenscribe-settings-")
            .tempfile_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, self)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(path)
            .map_err(|error| Error::Io(error.error))?;
        Ok(())
    }
}

fn validate_paths(settings: &Settings) -> Result<()> {
    settings.validate()?;
    let mut paths = BTreeSet::new();
    for folder in &settings.folders {
        let path = Path::new(&folder.path);
        let key = path
            .canonicalize()
            .unwrap_or_else(|_| path.into())
            .to_string_lossy()
            .into_owned();
        #[cfg(windows)]
        let key = key.to_lowercase();
        if !paths.insert(key) {
            return Err(Error::InvalidInput(
                "a watched folder appears more than once".into(),
            ));
        }
    }
    Ok(())
}
