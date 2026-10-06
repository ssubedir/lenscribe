mod commands;
mod logging;
#[cfg(test)]
mod updater_tests;
mod updates;

use std::sync::Arc;

use lenscribe_core::{daemon::Daemon, Core};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

fn show_settings(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let pause = MenuItem::with_id(
        app,
        "pause",
        "Pause / resume monitoring",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Lenscribe", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &pause, &quit])?;
    let mut tray = TrayIconBuilder::with_id("lenscribe")
        .menu(&menu)
        .tooltip("Lenscribe folder monitor")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => show_settings(app),
            "pause" => {
                if app.state::<Arc<updates::Updates>>().installing() {
                    return;
                }
                let daemon = app.state::<commands::AppState>().daemon.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let result = async {
                        let mut settings = daemon.settings()?;
                        settings.monitoring_paused = !settings.monitoring_paused;
                        daemon.update_settings(settings).await
                    }
                    .await;
                    if let Err(error) = result {
                        log::error!("Cannot change monitoring state: {error}");
                        let _ = app.emit("lenscribe://daemon-error", error.to_string());
                        show_settings(&app);
                    }
                });
            }
            "quit" => app.exit(0),
            _ => (),
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_settings(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_settings(app)
        }))
        .plugin(logging::plugin())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .setup(|app| {
            log::info!("Lenscribe v{} starting", env!("CARGO_PKG_VERSION"));
            std::panic::set_hook(Box::new(|panic| {
                // A panic payload may contain arbitrary data; record its location only.
                if let Some(location) = panic.location() {
                    log::error!("Application panicked at {location}");
                } else {
                    log::error!("Application panicked");
                }
                log::logger().flush();
            }));
            let directory = app.path().app_data_dir()?;
            let core = Arc::new(Core::open(directory.join("index.wedb"))?);
            let handle = app.handle().clone();
            let daemon = Daemon::load(
                core.clone(),
                directory.join("settings.json"),
                Arc::new(move |event| {
                    let _ = handle.emit("lenscribe://watch", event);
                }),
            )?;
            let start_minimized = daemon.settings()?.start_minimized
                || std::env::args().any(|arg| arg == "--autostart");
            app.manage(commands::AppState {
                core,
                daemon: daemon.clone(),
            });
            let updates = updates::create(app.handle(), daemon.clone());
            app.manage(updates.clone());
            setup_tray(app)?;
            if !start_minimized {
                show_settings(app.handle());
            }
            tauri::async_runtime::spawn(async move {
                if let Err(error) = daemon.start().await {
                    log::error!("Cannot start Lenscribe daemon: {error}");
                }
                updates
                    .run_background(
                        std::time::Duration::from_secs(30),
                        std::time::Duration::from_secs(24 * 60 * 60),
                    )
                    .await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::scan_folder,
            commands::list_folders,
            commands::folder_snapshot,
            commands::file_details,
            commands::maintenance_status,
            commands::backup_database,
            commands::rebuild_index,
            commands::cleanup_cache,
            commands::attach_text,
            commands::search_files,
            commands::watch_folder,
            commands::unwatch_folder,
            commands::watch_status,
            commands::start_api,
            commands::stop_api,
            commands::daemon_status,
            commands::save_settings,
            commands::retry_daemon,
            commands::list_files,
            commands::file_preview,
            commands::queue_file,
            commands::edit_file,
            commands::discover_llm_models,
            updates::app_update_status,
            updates::check_app_update,
            updates::install_app_update,
        ])
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            log::error!("Cannot initialize Lenscribe: {error}");
            log::logger().flush();
            eprintln!("Cannot initialize Lenscribe: {error}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if let Some(state) = app.try_state::<commands::AppState>() {
                if let Err(error) = state.daemon.stop() {
                    log::error!("Cannot stop Lenscribe daemon: {error}");
                }
            }
            log::info!("Lenscribe stopped");
            log::logger().flush();
        }
    });
}
