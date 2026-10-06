use crate::{domain::settings::Settings, Result};

pub trait SettingsStore: Send + Sync {
    fn load(&self) -> Result<Settings>;
    fn save(&self, settings: &Settings) -> Result<()>;
}
