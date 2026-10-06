//! Exercise application workflows with replaced I/O, rather than testing adapter forwarding.
use lenscribe_core::{
    adapters::{
        filesystem::{images::LocalImageFiles, settings::JsonSettingsStore},
        wedb,
    },
    daemon::Daemon,
    domain::{image::InspectedImage, rules::FolderRules},
    ports::{
        images::{FileStamp, ImageFiles, ImagePaths},
        settings::SettingsStore,
        vision::{ExtractionError, ExtractionFuture, VisionFactory, VisionProvider},
        watch::{FolderWatch, WatchCallback, WatchTarget, Watcher},
    },
    settings::{ExtractionSettings, FolderSettings, Settings, Theme},
    Core, Error, PreparedImage, Result, WatchStatus,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};

#[derive(Default)]
struct StubVision {
    calls: AtomicUsize,
}
impl VisionProvider for StubVision {
    fn processor(&self) -> &str {
        "fixture/injected-v1"
    }
    fn extract<'a>(&'a self, image: &'a PreparedImage) -> ExtractionFuture<'a> {
        Box::pin(async move {
            assert_eq!(image.bytes, include_bytes!("fixtures/pixel.jpg"));
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok("Coffee receipt\nTotal: 12.50".into())
        })
    }
}
struct StubVisionFactory(Arc<StubVision>);
impl VisionFactory for StubVisionFactory {
    fn create(
        &self,
        _: ExtractionSettings,
    ) -> std::result::Result<Arc<dyn VisionProvider>, ExtractionError> {
        Ok(self.0.clone())
    }
}

struct StubWatch(WatchStatus);
impl FolderWatch for StubWatch {
    fn status(&self) -> Result<WatchStatus> {
        Ok(self.0.clone())
    }
}
struct StubWatcher;
impl Watcher for StubWatcher {
    fn start(
        &self,
        _: Weak<dyn WatchTarget>,
        folder_id: i64,
        root: PathBuf,
        _: WatchCallback,
    ) -> Result<Box<dyn FolderWatch>> {
        Ok(Box::new(StubWatch(WatchStatus {
            folder_id,
            path: root.to_string_lossy().into_owned(),
            last_error: None,
        })))
    }
}

struct MemorySettings(Mutex<Settings>);
impl SettingsStore for MemorySettings {
    fn load(&self) -> Result<Settings> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, settings: &Settings) -> Result<()> {
        settings.validate()?;
        *self.0.lock().unwrap() = settings.clone();
        Ok(())
    }
}

#[tokio::test]
async fn injected_vision_and_settings_drive_the_background_workflow() {
    let temp = tempfile::tempdir().unwrap();
    let images = temp.path().join("images");
    fs::create_dir(&images).unwrap();
    for name in ["first.jpg", "duplicate.jpg"] {
        fs::write(images.join(name), include_bytes!("fixtures/pixel.jpg")).unwrap();
    }
    let vision = Arc::new(StubVision::default());
    let core = Arc::new(Core::new(
        wedb::open(&temp.path().join("index.wedb")).unwrap(),
        Arc::new(LocalImageFiles),
        Arc::new(StubWatcher),
        Arc::new(StubVisionFactory(vision.clone())),
    ));
    let store = Arc::new(MemorySettings(Mutex::new(Settings {
        folders: vec![FolderSettings {
            path: images.to_string_lossy().into_owned(),
            ..Default::default()
        }],
        extraction: ExtractionSettings {
            enabled: true,
            model: "injected-test-provider".into(),
            // An accidental concrete provider dependency would fail against this endpoint.
            base_url: "http://127.0.0.1:1/v1".into(),
            ..Default::default()
        },
        ..Default::default()
    })));
    let daemon = Daemon::new(core.clone(), store.clone(), Arc::new(|_| {})).unwrap();
    daemon.start().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if daemon.status().unwrap().processed_images == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("injected provider should finish both images");
    assert_eq!(vision.calls.load(Ordering::Relaxed), 1);
    assert_eq!(core.search("cofee", None, 10).unwrap().len(), 2);
    let mut settings = daemon.settings().unwrap();
    settings.theme = Theme::Dark;
    daemon.update_settings(settings).await.unwrap();
    assert_eq!(store.load().unwrap().theme, Theme::Dark);
    daemon.shutdown().await.unwrap();
    assert!(core.watch_status().unwrap().is_empty());
    let reopened = Daemon::new(core, store, Arc::new(|_| {})).unwrap();
    assert_eq!(reopened.settings().unwrap().theme, Theme::Dark);
    reopened.shutdown().await.unwrap();
}

struct LockedOnceImages(AtomicBool);
impl ImageFiles for LockedOnceImages {
    fn canonical_folder(&self, path: &Path) -> Result<PathBuf> {
        LocalImageFiles.canonical_folder(path)
    }
    fn resolve_image(&self, root: &Path, relative: &str) -> Result<PathBuf> {
        LocalImageFiles.resolve_image(root, relative)
    }
    fn event_relative_path(&self, root: &Path, event: &Path) -> Result<String> {
        LocalImageFiles.event_relative_path(root, event)
    }
    fn image_paths<'a>(
        &'a self,
        root: &'a Path,
        scope: &str,
        rules: &'a FolderRules,
    ) -> Result<ImagePaths<'a>> {
        LocalImageFiles.image_paths(root, scope, rules)
    }
    fn stamp(&self, path: &Path) -> Result<FileStamp> {
        LocalImageFiles.stamp(path)
    }
    fn inspect(&self, path: &Path) -> Result<InspectedImage> {
        LocalImageFiles.inspect(path)
    }
    fn original_bytes(&self, path: &Path, expected: &str) -> Result<Vec<u8>> {
        LocalImageFiles.original_bytes(path, expected)
    }
    fn write_text(
        &self,
        path: &Path,
        expected: &str,
        text: &str,
        processor: &str,
    ) -> Result<InspectedImage> {
        if self.0.swap(false, Ordering::Relaxed) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "fixture image is locked",
            )));
        }
        LocalImageFiles.write_text(path, expected, text, processor)
    }
}

#[test]
fn failed_image_port_retains_the_synced_response_for_a_retry() {
    let temp = tempfile::tempdir().unwrap();
    let images = temp.path().join("images");
    fs::create_dir(&images).unwrap();
    let image = images.join("receipt.jpg");
    fs::write(&image, include_bytes!("fixtures/pixel.jpg")).unwrap();
    let core = Core::new(
        wedb::open(&temp.path().join("index.wedb")).unwrap(),
        Arc::new(LockedOnceImages(AtomicBool::new(true))),
        Arc::new(StubWatcher),
        Arc::new(StubVisionFactory(Arc::new(StubVision::default()))),
    );
    let folder = core.scan_folder(&images).unwrap().folder.id;
    let job = core.extraction_jobs(folder).unwrap().remove(0);
    assert!(matches!(
        core.complete_extraction(&job, "Coffee receipt", "fixture/v1"),
        Err(Error::Io(_))
    ));
    assert_eq!(
        fs::read(&image).unwrap(),
        include_bytes!("fixtures/pixel.jpg")
    );
    assert!(core.file(job.file.id).unwrap().text.is_none());
    assert_eq!(core.extraction_jobs(folder).unwrap().len(), 1);
    let saved = core
        .cached_extraction(&job.file.image_hash, "fixture/v1")
        .unwrap()
        .unwrap();
    core.complete_extraction(&job, &saved, "fixture/v1")
        .unwrap();
    assert!(core.extraction_jobs(folder).unwrap().is_empty());
    assert_eq!(
        core.file(job.file.id).unwrap().text.as_deref(),
        Some("Coffee receipt")
    );
}

#[test]
fn json_settings_adapter_rejects_canonical_folder_aliases_on_load_and_save() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("child")).unwrap();
    let settings = Settings {
        folders: vec![root.clone(), root.join("child/..")]
            .into_iter()
            .map(|path| FolderSettings {
                path: path.to_string_lossy().into_owned(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    settings.validate().unwrap();
    let path = temp.path().join("settings.json");
    let store = JsonSettingsStore::new(&path);
    assert!(store.save(&settings).is_err());
    assert!(!path.exists());
    fs::write(&path, serde_json::to_vec(&settings).unwrap()).unwrap();
    assert!(store.load().is_err());
}

struct UnvalidatedSettings(Mutex<Settings>, AtomicUsize);
impl SettingsStore for UnvalidatedSettings {
    fn load(&self) -> Result<Settings> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, settings: &Settings) -> Result<()> {
        self.1.fetch_add(1, Ordering::Relaxed);
        *self.0.lock().unwrap() = settings.clone();
        Ok(())
    }
}

#[tokio::test]
async fn runtime_enforces_domain_validation_even_with_a_custom_settings_store() {
    let temp = tempfile::tempdir().unwrap();
    let core = Arc::new(Core::open(temp.path().join("index.wedb")).unwrap());
    let store = Arc::new(UnvalidatedSettings(
        Mutex::new(Settings {
            version: 99,
            ..Default::default()
        }),
        AtomicUsize::new(0),
    ));
    assert!(Daemon::new(core.clone(), store.clone(), Arc::new(|_| {})).is_err());
    *store.0.lock().unwrap() = Settings::default();
    let daemon = Daemon::new(core, store.clone(), Arc::new(|_| {})).unwrap();
    let mut invalid = daemon.settings().unwrap();
    invalid.extraction.concurrency = 0;
    assert!(daemon.update_settings(invalid).await.is_err());
    assert_eq!(store.1.load(Ordering::Relaxed), 0);
    assert_eq!(daemon.settings().unwrap(), Settings::default());
    daemon.shutdown().await.unwrap();
}
