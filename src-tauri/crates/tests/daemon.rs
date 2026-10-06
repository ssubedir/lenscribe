use std::{
    fs,
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use lenscribe_core::{
    daemon::Daemon,
    settings::{ApiSettings, FolderSettings, Settings, Theme},
    Core,
};
use tempfile::{tempdir, TempDir};

fn setup() -> (TempDir, PathBuf, Arc<Core>, Arc<Daemon>) {
    let temporary = tempdir().unwrap();
    let images = temporary.path().join("images");
    fs::create_dir(&images).unwrap();
    fs::write(
        images.join("first.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    let core = Arc::new(Core::open(temporary.path().join("index.wedb")).unwrap());
    let daemon = Daemon::load(
        core.clone(),
        temporary.path().join("settings.json"),
        Arc::new(|_| {}),
    )
    .unwrap();
    (temporary, images, core, daemon)
}

fn configured(images: &Path) -> Settings {
    Settings {
        folders: vec![FolderSettings {
            path: images.to_str().unwrap().into(),
            enabled: true,
            ..lenscribe_core::settings::FolderSettings::default()
        }],
        ..Settings::default()
    }
}

#[tokio::test]
async fn progress_counts_empty_transcriptions_and_tracks_folder_health() {
    let (temporary, images, core, daemon) = setup();
    let empty = temporary.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let mut settings = configured(&images);
    settings.folders.push(FolderSettings {
        path: empty.to_str().unwrap().into(),
        enabled: true,
        ..lenscribe_core::settings::FolderSettings::default()
    });
    let status = daemon.update_settings(settings.clone()).await.unwrap();
    assert_eq!(
        (
            status.total_images,
            status.processed_images,
            status.pending_images
        ),
        (1, 0, 1)
    );
    assert_eq!(status.folder_statuses.len(), 2);
    assert!(status.folder_statuses.iter().all(|folder| folder.watching));
    assert_eq!(status.folder_statuses[1].image_count, 0);
    assert_eq!(status.folder_statuses[1].pending_images, 0);

    let pending = daemon.pending_images().unwrap();
    let file = &pending[0];
    // A successful image with no readable text is processed, not pending.
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        "",
        "fixture/v1",
    )
    .unwrap();
    let status = daemon.status().unwrap();
    assert_eq!(
        (
            status.total_images,
            status.processed_images,
            status.pending_images
        ),
        (1, 1, 0)
    );
    assert_eq!(status.folder_statuses[0].pending_images, 0);
    assert!(status.folder_statuses[0].root_hash.is_some());

    let later = images.join("later.jpg");
    fs::write(&later, include_bytes!("fixtures/pixel.jpg")).unwrap();
    core.scan_folder(&images).unwrap();
    let added = daemon.status().unwrap();
    assert_eq!(
        (
            added.total_images,
            added.processed_images,
            added.pending_images
        ),
        (2, 1, 1)
    );
    fs::remove_file(&later).unwrap();
    core.scan_folder(&images).unwrap();
    let removed = daemon.status().unwrap();
    assert_eq!(
        (
            removed.total_images,
            removed.processed_images,
            removed.pending_images
        ),
        (1, 1, 0)
    );

    settings.monitoring_paused = true;
    let paused = daemon.update_settings(settings.clone()).await.unwrap();
    assert_eq!(paused.processed_images, 1);
    assert!(paused.folder_statuses.iter().all(|folder| !folder.watching));
    settings.folders[0].enabled = false;
    let disabled = daemon.update_settings(settings).await.unwrap();
    assert_eq!(disabled.total_images, 0);
    assert_eq!(disabled.folder_statuses[0].image_count, 1);
    assert!(!disabled.folder_statuses[0].enabled);
    daemon.shutdown().await.unwrap();
}

#[test]
fn settings_round_trip_and_invalid_saves_preserve_previous_configuration() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("settings.json");
    assert_eq!(Settings::load(&path).unwrap(), Settings::default());
    let mut settings = configured(temporary.path());
    settings.theme = Theme::Dark;
    settings.save(&path).unwrap();
    assert_eq!(Settings::load(&path).unwrap(), settings);
    let before = fs::read(&path).unwrap();
    let mut invalid = settings.clone();
    invalid.folders[0].path = "relative/folder".into();
    assert!(invalid.save(&path).is_err());
    invalid = settings.clone();
    invalid.folders.push(invalid.folders[0].clone());
    assert!(invalid.save(&path).is_err());
    invalid = settings;
    invalid.version = 99;
    assert!(invalid.save(&path).is_err());
    assert_eq!(before, fs::read(path).unwrap());
}

#[test]
fn existing_settings_default_to_light_without_losing_preferences() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("settings.json");
    fs::write(
        &path,
        br#"{"version":1,"startMinimized":true,"api":{"enabled":true,"port":47841}}"#,
    )
    .unwrap();
    let settings = Settings::load(&path).unwrap();
    assert_eq!(settings.theme, Theme::Light);
    assert!(settings.start_minimized);
    assert!(settings.api.enabled);
    assert_eq!(settings.api.port, 47841);
}

#[test]
fn malformed_settings_are_reported_without_being_silently_reset() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("settings.json");
    fs::write(&path, b"{broken JSON").unwrap();
    assert!(Settings::load(&path).is_err());
    assert_eq!(fs::read(path).unwrap(), b"{broken JSON");
}

#[tokio::test]
async fn saved_configuration_restores_watches_api_and_backlog_without_a_ui() {
    let (temporary, images, core, daemon) = setup();
    daemon.start().await.unwrap();
    let mut settings = configured(&images);
    settings.start_minimized = true;
    settings.api = ApiSettings {
        enabled: true,
        port: 0,
    };
    let status = daemon.update_settings(settings.clone()).await.unwrap();
    assert_eq!(status.watchers.len(), 1);
    assert_eq!(status.pending_images, 1);
    assert!(status.api_url.unwrap().starts_with("http://127.0.0.1:"));
    daemon.shutdown().await.unwrap();
    assert!(core.watch_status().unwrap().is_empty());
    drop(daemon);
    drop(core);

    fs::write(
        images.join("while-offline.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    let core = Arc::new(Core::open(temporary.path().join("index.wedb")).unwrap());
    let restored = Daemon::load(
        core.clone(),
        temporary.path().join("settings.json"),
        Arc::new(|_| {}),
    )
    .unwrap();
    let status = restored.start().await.unwrap();
    assert_eq!(status.settings, settings);
    assert_eq!(status.watchers.len(), 1);
    assert_eq!(status.pending_images, 2);
    assert!(status.api_url.is_some());
    fs::write(
        images.join("while-hidden.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while restored.status().unwrap().pending_images != 3 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(restored.status().unwrap().pending_images, 3);
    restored.stop().unwrap();
}

#[tokio::test]
async fn failed_install_recovery_restores_watches_api_and_forced_jobs_without_changing_settings() {
    let (temporary, images, core, daemon) = setup();
    daemon.start().await.unwrap();
    let mut settings = configured(&images);
    settings.api = ApiSettings {
        enabled: true,
        port: 0,
    };
    settings.start_minimized = true;
    daemon.update_settings(settings.clone()).await.unwrap();
    let file = core
        .list_files(core.folders().unwrap()[0].id, "", 0)
        .unwrap()
        .files[0]
        .clone();
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        "Keep this text",
        "fixture/v1",
    )
    .unwrap();
    core.queue_file(file.id, &file.image_hash, true).unwrap();
    let saved = fs::read(temporary.path().join("settings.json")).unwrap();
    daemon.shutdown().await.unwrap();
    assert!(core.watch_status().unwrap().is_empty());
    assert!(daemon.status().unwrap().api_url.is_none());
    fs::write(
        images.join("during-install.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    let resumed = daemon.restart().await.unwrap();
    assert_eq!(resumed.settings, settings);
    assert_eq!(
        fs::read(temporary.path().join("settings.json")).unwrap(),
        saved
    );
    assert_eq!(resumed.watchers.len(), 1);
    assert!(resumed.api_url.is_some());
    assert_eq!(
        core.file(file.id).unwrap().text.as_deref(),
        Some("Keep this text")
    );
    assert!(daemon
        .pending_images()
        .unwrap()
        .iter()
        .any(|pending| pending.id == file.id));
    assert_eq!(resumed.total_images, 2);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn installer_recovery_preserves_a_user_paused_monitor() {
    let (_temporary, images, _core, daemon) = setup();
    let mut settings = configured(&images);
    settings.monitoring_paused = true;
    daemon.update_settings(settings.clone()).await.unwrap();
    daemon.shutdown().await.unwrap();
    let resumed = daemon.restart().await.unwrap();
    assert_eq!(resumed.settings, settings);
    assert!(resumed.watchers.is_empty());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pausing_keeps_backlog_and_resuming_discovers_images_saved_during_pause() {
    let (_temporary, images, core, daemon) = setup();
    daemon.start().await.unwrap();
    let mut settings = configured(&images);
    daemon.update_settings(settings.clone()).await.unwrap();
    settings.monitoring_paused = true;
    let status = daemon.update_settings(settings.clone()).await.unwrap();
    assert!(status.watchers.is_empty());
    assert_eq!(status.pending_images, 1);
    fs::write(
        images.join("during-pause.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    assert_eq!(daemon.status().unwrap().pending_images, 1);
    settings.monitoring_paused = false;
    let status = daemon.update_settings(settings.clone()).await.unwrap();
    assert_eq!(status.watchers.len(), 1);
    assert_eq!(status.pending_images, 2);
    let pending = daemon.pending_images().unwrap();
    let file = &pending[0];
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        "extracted text",
        "fixture/v1",
    )
    .unwrap();
    assert_eq!(daemon.pending_images().unwrap().len(), 1);
    settings.folders[0].enabled = false;
    let status = daemon.update_settings(settings).await.unwrap();
    assert!(status.watchers.is_empty());
    assert_eq!(status.pending_images, 0);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn unavailable_folders_do_not_stop_healthy_watches_and_can_recover() {
    let (temporary, images, _core, daemon) = setup();
    let missing = temporary.path().join("removable");
    let mut settings = configured(&images);
    settings.folders.push(FolderSettings {
        path: missing.to_str().unwrap().into(),
        enabled: true,
        ..lenscribe_core::settings::FolderSettings::default()
    });
    let status = daemon.update_settings(settings).await.unwrap();
    assert_eq!(status.watchers.len(), 1);
    assert_eq!(status.issues.len(), 1);
    assert!(status.folder_statuses[1].last_error.is_some());
    assert!(!status.folder_statuses[1].watching);
    fs::create_dir(&missing).unwrap();
    fs::write(
        missing.join("new.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    let recovered = daemon.reconcile().await.unwrap();
    assert_eq!(recovered.watchers.len(), 2);
    assert!(recovered.issues.is_empty());
    assert!(recovered
        .folder_statuses
        .iter()
        .all(|folder| folder.watching));
    assert!(recovered
        .folder_statuses
        .iter()
        .all(|folder| folder.last_error.is_none()));
    assert_eq!(recovered.pending_images, 2);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn occupied_api_ports_are_reported_and_retried_without_stopping_monitoring() {
    let (_temporary, images, _core, daemon) = setup();
    let blocker = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let mut settings = configured(&images);
    settings.api = ApiSettings {
        enabled: true,
        port: blocker.local_addr().unwrap().port(),
    };
    let status = daemon.update_settings(settings).await.unwrap();
    assert_eq!(status.watchers.len(), 1);
    assert!(status.api_url.is_none());
    assert_eq!(status.issues[0].source, "Local API");
    drop(blocker);
    let recovered = daemon.reconcile().await.unwrap();
    assert!(recovered.api_url.is_some());
    assert!(recovered.issues.is_empty());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_settings_do_not_change_live_watches_or_saved_settings() {
    let (temporary, images, _core, daemon) = setup();
    let settings = configured(&images);
    daemon.update_settings(settings.clone()).await.unwrap();
    let mut invalid = settings.clone();
    invalid.folders[0].path = "relative".into();
    assert!(daemon.update_settings(invalid).await.is_err());
    assert_eq!(daemon.status().unwrap().watchers.len(), 1);
    assert_eq!(daemon.settings().unwrap(), settings);
    assert_eq!(
        Settings::load(&temporary.path().join("settings.json")).unwrap(),
        settings
    );
    daemon.shutdown().await.unwrap();
    assert!(daemon.reconcile().await.is_err());
}
