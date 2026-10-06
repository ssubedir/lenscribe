//! Default desktop/headless wiring. Application constructors accept ports for alternate adapters.
use crate::{
    adapters::{
        filesystem::{images::LocalImageFiles, settings::JsonSettingsStore, watch::NotifyWatcher},
        llm::GenaiVisionFactory,
        wedb,
    },
    daemon::Daemon,
    ports::{index::IndexRepository, watch::WatchCallback},
    Core, Result,
};
use std::{fs, path::Path, sync::Arc};

pub fn core_with_repository(repository: Box<dyn IndexRepository>) -> Core {
    Core::new(
        repository,
        Arc::new(LocalImageFiles),
        Arc::new(NotifyWatcher),
        Arc::new(GenaiVisionFactory),
    )
}

impl Core {
    pub fn open(database_path: impl AsRef<Path>) -> Result<Self> {
        let path = database_path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        Ok(core_with_repository(wedb::open(path)?))
    }

    /// Restore a verified backup before starting the runtime.
    pub fn restore_database(
        backup: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<Self> {
        wedb::restore(backup.as_ref(), destination.as_ref())?;
        Self::open(destination)
    }
}

impl Daemon {
    pub fn load(
        core: Arc<Core>,
        settings_path: impl AsRef<Path>,
        on_event: WatchCallback,
    ) -> Result<Arc<Self>> {
        Self::new(
            core,
            Arc::new(JsonSettingsStore::new(settings_path.as_ref())),
            on_event,
        )
    }
}
