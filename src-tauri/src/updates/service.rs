use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Checking,
    Available,
    Downloading,
    Installing,
    Restarting,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub revision: u64,
    pub supported: bool,
    pub support_message: Option<String>,
    pub phase: Phase,
    pub available: Option<UpdateInfo>,
    pub progress: Option<u8>,
    pub last_checked: Option<u64>,
    pub error: Option<String>,
}

pub trait Package: Clone + Send + Sync + 'static {
    fn info(&self) -> UpdateInfo;
    /// Must return bytes only after verifying the package signature.
    fn download(
        &self,
        progress: impl Fn(Option<u8>) + Send + Sync + 'static,
    ) -> impl Future<Output = Result<Vec<u8>, String>> + Send;
    fn install(&self, bytes: Vec<u8>) -> impl Future<Output = Result<(), String>> + Send;
}

pub trait Provider: Send + Sync + 'static {
    type Package: Package;
    fn check(&self) -> impl Future<Output = Result<Option<Self::Package>, String>> + Send;
}

pub trait Lifecycle: Send + Sync + 'static {
    fn shutdown(&self) -> impl Future<Output = Result<(), String>> + Send;
    fn resume(&self) -> impl Future<Output = Result<(), String>> + Send;
    fn restart(&self);
}

struct State<P> {
    status: UpdateStatus,
    package: Option<P>,
}

pub struct UpdateService<P: Provider, L: Lifecycle> {
    provider: P,
    lifecycle: L,
    state: Mutex<State<P::Package>>,
    operation: tokio::sync::Mutex<()>,
    publish: Box<dyn Fn(UpdateStatus) + Send + Sync>,
}

impl<P: Provider, L: Lifecycle> UpdateService<P, L> {
    pub fn new(
        provider: P,
        lifecycle: L,
        support_message: Option<String>,
        publish: impl Fn(UpdateStatus) + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            provider,
            lifecycle,
            state: Mutex::new(State {
                status: UpdateStatus {
                    revision: 0,
                    supported: support_message.is_none(),
                    support_message,
                    phase: Phase::Idle,
                    available: None,
                    progress: None,
                    last_checked: None,
                    error: None,
                },
                package: None,
            }),
            operation: tokio::sync::Mutex::new(()),
            publish: Box::new(publish),
        })
    }

    pub fn status(&self) -> UpdateStatus {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .status
            .clone()
    }

    pub fn installing(&self) -> bool {
        matches!(
            self.status().phase,
            Phase::Downloading | Phase::Installing | Phase::Restarting
        )
    }

    fn change(&self, change: impl FnOnce(&mut State<P::Package>)) -> UpdateStatus {
        let status = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            change(&mut state);
            state.status.revision += 1;
            state.status.clone()
        };
        (self.publish)(status.clone());
        status
    }

    pub async fn check(&self) -> Result<UpdateStatus, String> {
        if !self.status().supported {
            return Ok(self.status());
        }
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| "An update operation is already in progress.")?;
        self.change(|state| {
            state.status.phase = Phase::Checking;
            state.status.error = None;
        });
        let result = self.provider.check().await;
        Ok(self.change(|state| match result {
            Ok(package) => {
                // The stable feed should never offer a prerelease, even if misconfigured.
                state.package = package.filter(|package| !package.info().version.contains('-'));
                state.status.available = state.package.as_ref().map(Package::info);
                state.status.phase = if state.package.is_some() {
                    Phase::Available
                } else {
                    Phase::Idle
                };
                state.status.last_checked = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .map(|time| time.as_secs());
            }
            Err(_) => {
                log::warn!("Update check failed");
                // Retain the last known package so an offline check does not erase the notice.
                state.status.phase = Phase::Error;
                state.status.error = Some(
                    "Could not check for updates. Check your connection and try again.".into(),
                );
            }
        }))
    }

    pub async fn install(self: &Arc<Self>, version: &str) -> Result<(), String> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| "An update operation is already in progress.")?;
        let package = {
            let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if !state.status.supported {
                return Err(state.status.support_message.clone().unwrap_or_default());
            }
            state
                .package
                .clone()
                .filter(|package| package.info().version == version)
                .ok_or("The available update changed. Check for updates again before installing.")?
        };
        self.change(|state| {
            state.status.phase = Phase::Downloading;
            state.status.progress = None;
            state.status.error = None;
        });
        let weak = Arc::downgrade(self);
        let downloaded = package
            .download(move |progress| {
                if let Some(service) = weak.upgrade() {
                    if service.status().progress != progress {
                        service.change(|state| state.status.progress = progress);
                    }
                }
            })
            .await;
        let bytes = match downloaded {
            Ok(bytes) => bytes,
            Err(_) => return self.failed("Could not download or verify the update. Monitoring is still running; try again.", false).await,
        };
        self.change(|state| {
            state.status.phase = Phase::Installing;
            state.status.progress = Some(100);
        });
        if self.lifecycle.shutdown().await.is_err() {
            return self
                .failed("Could not prepare Lenscribe for installation.", true)
                .await;
        }
        if package.install(bytes).await.is_err() {
            return self.failed("Could not install the update.", true).await;
        }
        self.change(|state| state.status.phase = Phase::Restarting);
        self.lifecycle.restart();
        Ok(())
    }

    async fn failed(&self, message: &str, resume: bool) -> Result<(), String> {
        let message = if !resume {
            message.to_owned()
        } else if self.lifecycle.resume().await.is_ok() {
            format!("{message} Monitoring has resumed; try again.")
        } else {
            format!("{message} Restart Lenscribe to resume monitoring.")
        };
        log::warn!("{message}");
        self.change(|state| {
            state.status.phase = Phase::Error;
            state.status.progress = None;
            state.status.error = Some(message.clone());
        });
        Err(message)
    }

    pub async fn run_background(self: Arc<Self>, startup: Duration, interval: Duration) {
        if !self.status().supported {
            return;
        }
        tokio::time::sleep(startup).await;
        loop {
            let _ = self.check().await;
            tokio::time::sleep(interval).await;
        }
    }
}
