mod service;
#[cfg(test)]
mod tests;

use std::{sync::Arc, time::Duration};

use lenscribe_core::daemon::Daemon;
#[cfg(target_os = "linux")]
use tauri::Manager;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

pub(super) use service::Package;
pub use service::UpdateStatus;
use service::{Lifecycle, Provider, UpdateInfo, UpdateService};

pub type Updates = UpdateService<NativeProvider, DaemonLifecycle>;

pub struct NativeProvider(AppHandle);
impl Provider for NativeProvider {
    type Package = Update;

    async fn check(&self) -> Result<Option<Update>, String> {
        self.0
            .updater_builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| "Cannot initialize updater")?
            .check()
            .await
            .map_err(|_| "Cannot check update".into())
    }
}

impl Package for Update {
    fn info(&self) -> UpdateInfo {
        UpdateInfo {
            version: self.version.clone(),
            notes: self.body.clone().unwrap_or_default(),
        }
    }

    async fn download(
        &self,
        progress: impl Fn(Option<u8>) + Send + Sync + 'static,
    ) -> Result<Vec<u8>, String> {
        let mut update = self.clone();
        update.timeout = Some(Duration::from_secs(300));
        let mut downloaded = 0u64;
        let bytes = update
            .download(
                move |chunk, total| {
                    downloaded = downloaded.saturating_add(chunk as u64);
                    progress(
                        total
                            .filter(|total| *total > 0)
                            .map(|total| (downloaded.saturating_mul(100) / total).min(99) as u8),
                    );
                },
                || {},
            )
            .await
            .map_err(|_| "Cannot download or verify update")?;
        // download() has verified the signature, including the signed version.
        Ok(bytes)
    }

    async fn install(&self, bytes: Vec<u8>) -> Result<(), String> {
        let update = self.clone();
        tauri::async_runtime::spawn_blocking(move || update.install(bytes))
            .await
            .map_err(|_| "Update installer task failed")?
            .map_err(|_| "Update installer failed".into())
    }
}

pub struct DaemonLifecycle {
    app: AppHandle,
    daemon: Arc<Daemon>,
}
impl Lifecycle for DaemonLifecycle {
    async fn shutdown(&self) -> Result<(), String> {
        self.daemon
            .shutdown()
            .await
            .map_err(|_| "Cannot stop monitoring".into())
    }

    async fn resume(&self) -> Result<(), String> {
        self.daemon
            .restart()
            .await
            .map(|_| ())
            .map_err(|_| "Cannot resume monitoring".into())
    }

    fn restart(&self) {
        self.app.restart();
    }
}

pub fn create(app: &AppHandle, daemon: Arc<Daemon>) -> Arc<Updates> {
    let support_message = support_message(app);
    let handle = app.clone();
    UpdateService::new(
        NativeProvider(app.clone()),
        DaemonLifecycle {
            app: app.clone(),
            daemon,
        },
        support_message,
        move |status| {
            let _ = handle.emit("lenscribe://update", status);
        },
    )
}

fn support_message(_app: &AppHandle) -> Option<String> {
    if cfg!(debug_assertions) {
        return Some("Updates are available in installed release builds.".into());
    }
    #[cfg(target_os = "linux")]
    if _app.env().appimage.is_none() {
        return Some("Install Linux package updates through your package manager. In-app updates are available for AppImage installations.".into());
    }
    None
}

#[tauri::command]
pub fn app_update_status(updates: tauri::State<'_, Arc<Updates>>) -> UpdateStatus {
    updates.status()
}

#[tauri::command]
pub async fn check_app_update(
    updates: tauri::State<'_, Arc<Updates>>,
) -> Result<UpdateStatus, String> {
    updates.check().await
}

#[tauri::command]
pub async fn install_app_update(
    version: String,
    updates: tauri::State<'_, Arc<Updates>>,
) -> Result<(), String> {
    updates.inner().install(&version).await
}
