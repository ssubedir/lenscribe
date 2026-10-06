use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

use super::service::*;

#[derive(Default)]
struct Fake {
    order: Mutex<Vec<&'static str>>,
    failure: Mutex<Option<&'static str>>,
    version: Mutex<Option<String>>,
    checks: AtomicUsize,
    hold_download: AtomicBool,
    resume_failed: AtomicBool,
    download_started: tokio::sync::Notify,
    finish_download: tokio::sync::Notify,
}
impl Fake {
    fn record(&self, stage: &'static str) -> Result<(), String> {
        self.order.lock().unwrap().push(stage);
        if *self.failure.lock().unwrap() == Some(stage)
            || (stage == "resume" && self.resume_failed.load(Ordering::Relaxed))
        {
            Err("Sensitive internal detail".into())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
struct FakePackage(Arc<Fake>, String);
impl Package for FakePackage {
    fn info(&self) -> UpdateInfo {
        UpdateInfo {
            version: self.1.clone(),
            notes: "Sample notes".into(),
        }
    }
    async fn download(
        &self,
        progress: impl Fn(Option<u8>) + Send + Sync + 'static,
    ) -> Result<Vec<u8>, String> {
        self.0.download_started.notify_one();
        progress(Some(40));
        if self.0.hold_download.load(Ordering::Relaxed) {
            self.0.finish_download.notified().await;
        }
        self.0.record("download")?;
        Ok(vec![1, 2, 3])
    }
    async fn install(&self, bytes: Vec<u8>) -> Result<(), String> {
        assert_eq!(bytes, vec![1, 2, 3]);
        self.0.record("install")
    }
}
impl Provider for Arc<Fake> {
    type Package = FakePackage;
    async fn check(&self) -> Result<Option<FakePackage>, String> {
        self.checks.fetch_add(1, Ordering::Relaxed);
        self.record("check")?;
        Ok(self
            .version
            .lock()
            .unwrap()
            .clone()
            .map(|version| FakePackage(self.clone(), version)))
    }
}
impl Lifecycle for Arc<Fake> {
    async fn shutdown(&self) -> Result<(), String> {
        self.record("shutdown")
    }
    async fn resume(&self) -> Result<(), String> {
        self.record("resume")
    }
    fn restart(&self) {
        self.record("restart").unwrap();
    }
}
type Service = UpdateService<Arc<Fake>, Arc<Fake>>;
fn fixture() -> (Arc<Fake>, Arc<Service>) {
    let fake = Arc::new(Fake::default());
    *fake.version.lock().unwrap() = Some("0.2.0".into());
    let service = UpdateService::new(fake.clone(), fake.clone(), None, |_| {});
    (fake, service)
}

#[tokio::test]
async fn verified_download_precedes_shutdown_install_and_restart() {
    let (fake, service) = fixture();
    service.check().await.unwrap();
    service.install("0.2.0").await.unwrap();
    assert_eq!(
        *fake.order.lock().unwrap(),
        ["check", "download", "shutdown", "install", "restart"]
    );
    assert_eq!(service.status().phase, Phase::Restarting);
    assert_eq!(service.status().progress, Some(100));
}

#[tokio::test]
async fn failed_download_never_stops_monitoring_and_can_be_retried() {
    let (fake, service) = fixture();
    service.check().await.unwrap();
    *fake.failure.lock().unwrap() = Some("download");
    let error = service.install("0.2.0").await.unwrap_err();
    assert!(error.contains("Monitoring is still running"));
    assert!(!error.contains("Sensitive"));
    assert_eq!(*fake.order.lock().unwrap(), ["check", "download"]);
    assert!(service.status().available.is_some());
    *fake.failure.lock().unwrap() = None;
    service.install("0.2.0").await.unwrap();
}

#[tokio::test]
async fn shutdown_and_installer_failures_resume_monitoring() {
    for stage in ["shutdown", "install"] {
        let (fake, service) = fixture();
        service.check().await.unwrap();
        *fake.failure.lock().unwrap() = Some(stage);
        assert!(service
            .install("0.2.0")
            .await
            .unwrap_err()
            .contains("Monitoring has resumed"));
        let order = fake.order.lock().unwrap();
        assert_eq!(order.last(), Some(&"resume"));
        assert!(!order.contains(&"restart"));
    }
}

#[tokio::test]
async fn a_recovery_failure_explains_how_to_restart() {
    let (fake, service) = fixture();
    service.check().await.unwrap();
    *fake.failure.lock().unwrap() = Some("install");
    fake.resume_failed.store(true, Ordering::Relaxed);
    assert!(service
        .install("0.2.0")
        .await
        .unwrap_err()
        .contains("Restart Lenscribe"));
    assert_eq!(service.status().phase, Phase::Error);
}

#[tokio::test]
async fn competing_checks_and_installs_cannot_replace_a_downloading_package() {
    let (fake, service) = fixture();
    service.check().await.unwrap();
    fake.hold_download.store(true, Ordering::Relaxed);
    let installing = tokio::spawn({
        let service = service.clone();
        async move { service.install("0.2.0").await }
    });
    fake.download_started.notified().await;
    assert_eq!(service.status().phase, Phase::Downloading);
    assert_eq!(service.status().progress, Some(40));
    assert!(service.check().await.is_err());
    assert!(service.install("0.2.0").await.is_err());
    fake.finish_download.notify_one();
    installing.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_stale_version_is_rejected_before_downloading() {
    let (fake, service) = fixture();
    service.check().await.unwrap();
    assert!(service.install("0.1.0").await.is_err());
    assert_eq!(*fake.order.lock().unwrap(), ["check"]);
}

#[tokio::test]
async fn offline_checks_retain_the_available_update_and_success_clears_errors() {
    let (fake, service) = fixture();
    let available = service.check().await.unwrap();
    *fake.failure.lock().unwrap() = Some("check");
    let failed = service.check().await.unwrap();
    assert_eq!(failed.available, available.available);
    assert_eq!(failed.last_checked, available.last_checked);
    assert!(!failed.error.unwrap().contains("Sensitive"));
    *fake.failure.lock().unwrap() = None;
    *fake.version.lock().unwrap() = None;
    let current = service.check().await.unwrap();
    assert!(current.available.is_none());
    assert!(current.error.is_none());
    assert!(current.revision > failed.revision);
}

#[tokio::test(start_paused = true)]
async fn background_checks_work_without_a_window_and_wait_a_day_between_attempts() {
    let (fake, service) = fixture();
    let background = tokio::spawn(service.run_background(
        std::time::Duration::from_secs(30),
        std::time::Duration::from_secs(86400),
    ));
    tokio::task::yield_now().await;
    assert_eq!(fake.checks.load(Ordering::Relaxed), 0);
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.checks.load(Ordering::Relaxed), 1);
    tokio::time::advance(std::time::Duration::from_secs(86399)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.checks.load(Ordering::Relaxed), 1);
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.checks.load(Ordering::Relaxed), 2);
    background.abort();
}

#[tokio::test]
async fn unsupported_installations_make_no_requests_and_prereleases_are_not_offered() {
    let (fake, service) = fixture();
    *fake.version.lock().unwrap() = Some("0.2.0-beta.1".into());
    assert!(service.check().await.unwrap().available.is_none());
    let disabled = UpdateService::new(
        fake.clone(),
        fake.clone(),
        Some("Use package manager".into()),
        |_| {},
    );
    let checks = fake.checks.load(Ordering::Relaxed);
    assert!(!disabled.check().await.unwrap().supported);
    assert!(disabled.install("0.2.0-beta.1").await.is_err());
    assert_eq!(fake.checks.load(Ordering::Relaxed), checks);
}
