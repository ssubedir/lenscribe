use std::{path::Path, sync::Arc};

use base64::{engine::general_purpose::STANDARD, Engine};
use lenscribe_core::{
    daemon::{Daemon, DaemonStatus},
    settings::Settings,
    Core, FileDetails, FolderRecord, FolderSnapshot, ScanReport, SearchHit, WatchStatus,
};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

pub struct AppState {
    pub core: Arc<Core>,
    pub daemon: Arc<Daemon>,
}

async fn run_core<T: Send + 'static>(
    core: Arc<Core>,
    operation: impl FnOnce(&Core) -> lenscribe_core::Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || operation(&core))
        .await
        .map_err(|error| {
            log::error!("Background command task failed");
            error.to_string()
        })?
        .map_err(|error| {
            log::warn!("File command failed: {error}");
            error.to_string()
        })
}

#[tauri::command]
pub async fn scan_folder(path: String, state: State<'_, AppState>) -> Result<ScanReport, String> {
    run_core(state.core.clone(), move |core| core.scan_folder(path)).await
}

#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>) -> Result<Vec<FolderRecord>, String> {
    run_core(state.core.clone(), Core::folders).await
}

#[tauri::command]
pub async fn folder_snapshot(
    folder_id: i64,
    state: State<'_, AppState>,
) -> Result<FolderSnapshot, String> {
    run_core(state.core.clone(), move |core| core.snapshot(folder_id)).await
}

#[tauri::command]
pub async fn file_details(file_id: i64, state: State<'_, AppState>) -> Result<FileDetails, String> {
    run_core(state.core.clone(), move |core| core.file(file_id)).await
}

#[tauri::command]
pub async fn maintenance_status(
    state: State<'_, AppState>,
) -> Result<lenscribe_core::MaintenanceStatus, String> {
    run_core(state.core.clone(), Core::maintenance_status).await
}

#[tauri::command]
pub async fn backup_database(path: String, state: State<'_, AppState>) -> Result<(), String> {
    run_core(state.core.clone(), move |core| core.backup_database(path)).await
}

#[tauri::command]
pub async fn rebuild_index(
    state: State<'_, AppState>,
) -> Result<lenscribe_core::MaintenanceReport, String> {
    run_core(state.core.clone(), Core::rebuild_index).await
}

#[tauri::command]
pub async fn cleanup_cache(state: State<'_, AppState>) -> Result<usize, String> {
    run_core(state.core.clone(), Core::cleanup_cache).await
}

#[tauri::command]
pub async fn attach_text(
    folder_id: i64,
    relative_path: String,
    expected_image_hash: String,
    text: String,
    processor: String,
    state: State<'_, AppState>,
) -> Result<FileDetails, String> {
    run_core(state.core.clone(), move |core| {
        core.attach_text(
            folder_id,
            &relative_path,
            &expected_image_hash,
            &text,
            &processor,
        )
    })
    .await
}

#[tauri::command]
pub async fn search_files(
    query: String,
    folder_id: Option<i64>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchHit>, String> {
    run_core(state.core.clone(), move |core| {
        core.search(&query, folder_id, limit.unwrap_or(20))
    })
    .await
}

#[tauri::command]
pub async fn watch_folder(path: String, state: State<'_, AppState>) -> Result<ScanReport, String> {
    let mut settings = state.daemon.settings().map_err(|error| error.to_string())?;
    if let Some(folder) = settings
        .folders
        .iter_mut()
        .find(|folder| folder.path == path)
    {
        folder.enabled = true;
    } else {
        settings
            .folders
            .push(lenscribe_core::settings::FolderSettings {
                path: path.clone(),
                enabled: true,
                ..lenscribe_core::settings::FolderSettings::default()
            });
    }
    state
        .daemon
        .update_settings(settings)
        .await
        .map_err(|error| error.to_string())?;
    run_core(state.core.clone(), move |core| core.scan_folder(path)).await
}

#[tauri::command]
pub async fn unwatch_folder(folder_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let snapshot = run_core(state.core.clone(), move |core| core.snapshot(folder_id)).await?;
    let mut settings = state.daemon.settings().map_err(|error| error.to_string())?;
    for folder in &mut settings.folders {
        if Path::new(&folder.path)
            .canonicalize()
            .is_ok_and(|path| path == Path::new(&snapshot.folder.path))
        {
            folder.enabled = false;
        }
    }
    state
        .daemon
        .update_settings(settings)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn watch_status(state: State<'_, AppState>) -> Result<Vec<WatchStatus>, String> {
    run_core(state.core.clone(), Core::watch_status).await
}

#[tauri::command]
pub async fn start_api(port: Option<u16>, state: State<'_, AppState>) -> Result<String, String> {
    let mut settings = state.daemon.settings().map_err(|error| error.to_string())?;
    settings.api.enabled = true;
    if let Some(port) = port {
        settings.api.port = port;
    }
    let status = state
        .daemon
        .update_settings(settings)
        .await
        .map_err(|error| error.to_string())?;
    status.api_url.ok_or_else(|| {
        status
            .issues
            .iter()
            .map(|issue| issue.error.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    })
}

#[tauri::command]
pub async fn stop_api(state: State<'_, AppState>) -> Result<(), String> {
    let mut settings = state.daemon.settings().map_err(|error| error.to_string())?;
    settings.api.enabled = false;
    state
        .daemon
        .update_settings(settings)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn daemon_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DaemonStatus, String> {
    let daemon = state.daemon.clone();
    let mut status = tauri::async_runtime::spawn_blocking(move || daemon.status())
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    match app.autolaunch().is_enabled() {
        Ok(enabled) => status.settings.start_at_login = enabled,
        Err(error) => status.issues.push(lenscribe_core::daemon::DaemonIssue {
            source: "Start at login".into(),
            error: error.to_string(),
        }),
    }
    Ok(status)
}

#[tauri::command]
pub async fn save_settings(
    settings: Settings,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DaemonStatus, String> {
    settings.validate().map_err(|error| error.to_string())?;
    let previous = app
        .autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())?;
    let requested = settings.start_at_login;
    if previous != requested {
        let result = if requested {
            app.autolaunch().enable()
        } else {
            app.autolaunch().disable()
        };
        result.map_err(|error| format!("Could not change start at login: {error}"))?;
    }
    let result = state
        .daemon
        .update_settings(settings)
        .await
        .map_err(|error| error.to_string());
    if result.is_err()
        && state
            .daemon
            .settings()
            .is_ok_and(|settings| settings.start_at_login != requested)
        && previous != requested
    {
        let rollback = if previous {
            app.autolaunch().enable()
        } else {
            app.autolaunch().disable()
        };
        if let Err(error) = rollback {
            return Err(format!(
                "Settings save failed; restoring start at login also failed: {error}"
            ));
        }
    }
    result
}

#[tauri::command]
pub async fn list_files(
    folder_id: i64,
    query: String,
    offset: usize,
    fuzzy: Option<bool>,
    state: State<'_, AppState>,
) -> Result<lenscribe_core::FilePage, String> {
    run_core(state.core.clone(), move |core| {
        core.find_files(folder_id, &query, offset, fuzzy.unwrap_or(false))
    })
    .await
}

#[tauri::command]
pub async fn file_preview(file_id: i64, state: State<'_, AppState>) -> Result<String, String> {
    run_core(state.core.clone(), move |core| {
        let file = core.file(file_id)?.file;
        if file.image_length > 20 * 1024 * 1024 {
            return Err(lenscribe_core::Error::InvalidInput(
                "Preview is limited to images below 20 MiB".into(),
            ));
        }
        let image = core.prepare_image(file.folder_id, &file.relative_path)?;
        if image.image_hash != file.image_hash {
            return Err(lenscribe_core::Error::ImageChanged);
        }
        Ok(format!(
            "data:{};base64,{}",
            image.mime_type,
            STANDARD.encode(image.bytes)
        ))
    })
    .await
}

#[tauri::command]
pub async fn queue_file(
    file_id: i64,
    expected_image_hash: String,
    force: bool,
    state: State<'_, AppState>,
) -> Result<DaemonStatus, String> {
    state
        .daemon
        .queue_file(file_id, expected_image_hash, force)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn edit_file(
    file_id: i64,
    expected_image_hash: String,
    expected_record_hash: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<FileDetails, String> {
    state
        .daemon
        .edit_file(file_id, expected_image_hash, expected_record_hash, text)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn retry_daemon(state: State<'_, AppState>) -> Result<DaemonStatus, String> {
    state
        .daemon
        .retry()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn discover_llm_models(
    settings: lenscribe_core::settings::ExtractionSettings,
) -> Result<lenscribe_core::llm::ModelCatalog, String> {
    lenscribe_core::llm::discover_models(settings)
        .await
        .map_err(|error| error.message)
}

#[tauri::command]
pub async fn prepare_update_install(state: State<'_, AppState>) -> Result<(), String> {
    state
        .daemon
        .shutdown()
        .await
        .map_err(|error| error.to_string())
}
