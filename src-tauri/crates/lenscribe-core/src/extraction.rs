//! A bounded background worker. The index is the durable source of pending images.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::task::{JoinHandle, JoinSet};

use crate::{
    llm::{ExtractionError, VisionClient},
    settings::ExtractionSettings,
    Core, Error, FileRecord, Result,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ExtractionPhase {
    #[default]
    Disabled,
    Paused,
    Idle,
    Extracting,
    NeedsConfiguration,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ExtractionStatus {
    pub phase: ExtractionPhase,
    pub current_file: Option<String>,
    pub active_files: Vec<String>,
    pub completed_images: u64,
    pub failed_images: usize,
    pub last_error: Option<String>,
    pub issues: Vec<ExtractionIssue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ExtractionIssue {
    pub folder_id: i64,
    pub relative_path: String,
    pub error: String,
    pub retrying: bool,
    pub attempts: u32,
    pub retry_at_ms: Option<u64>,
}

#[derive(Clone, PartialEq, Eq)]
struct WorkerConfig {
    settings: ExtractionSettings,
    folder_ids: Vec<i64>,
    paused: bool,
}

#[derive(Default)]
struct RunningWorker {
    config: Option<WorkerConfig>,
    task: Option<JoinHandle<()>>,
    cancelled: Arc<AtomicBool>,
}

pub(crate) struct ExtractionController {
    running: Mutex<RunningWorker>,
    status: Arc<Mutex<ExtractionStatus>>,
    commit_gate: Arc<tokio::sync::Mutex<()>>,
}

impl ExtractionController {
    pub fn new(commit_gate: Arc<tokio::sync::Mutex<()>>) -> Self {
        Self {
            running: Mutex::new(RunningWorker::default()),
            status: Arc::new(Mutex::new(ExtractionStatus {
                phase: ExtractionPhase::Disabled,
                ..ExtractionStatus::default()
            })),
            commit_gate,
        }
    }

    /// Called under the daemon's lifecycle lock; commits use the same lock so changing
    /// configuration cannot append an old in-flight response after the change is applied.
    pub fn configure(
        &self,
        core: Arc<Core>,
        settings: ExtractionSettings,
        folder_ids: Vec<i64>,
        paused: bool,
        force_retry: bool,
    ) -> Result<()> {
        let config = WorkerConfig {
            settings,
            folder_ids,
            paused,
        };
        let mut running = self.running.lock().map_err(|_| Error::Poisoned)?;
        if !force_retry && running.config.as_ref() == Some(&config) {
            return Ok(());
        }
        running.cancelled.store(true, Ordering::Relaxed);
        if let Some(task) = running.task.take() {
            task.abort();
        }
        running.cancelled = Arc::new(AtomicBool::new(false));
        if force_retry {
            if let Ok(client) = VisionClient::new(config.settings.clone()) {
                let recovery = recovery_id(&client, &config.settings);
                core.database
                    .lock()
                    .map_err(|_| Error::Poisoned)?
                    .recovery()
                    .clear(&recovery)?;
            }
        }
        {
            let mut status = self.status.lock().map_err(|_| Error::Poisoned)?;
            status.phase = if !config.settings.enabled {
                ExtractionPhase::Disabled
            } else if config.paused {
                ExtractionPhase::Paused
            } else {
                ExtractionPhase::Idle
            };
            status.current_file = None;
            status.active_files.clear();
            status.failed_images = 0;
            status.last_error = None;
            status.issues.clear();
        }
        if config.settings.enabled && !config.paused {
            running.task = Some(tokio::spawn(run_worker(
                core,
                config.clone(),
                self.status.clone(),
                self.commit_gate.clone(),
                running.cancelled.clone(),
            )));
        }
        running.config = Some(config);
        Ok(())
    }

    pub fn status(&self) -> Result<ExtractionStatus> {
        Ok(self.status.lock().map_err(|_| Error::Poisoned)?.clone())
    }

    pub fn stop(&self) -> Result<()> {
        let mut running = self.running.lock().map_err(|_| Error::Poisoned)?;
        running.cancelled.store(true, Ordering::Relaxed);
        if let Some(task) = running.task.take() {
            task.abort();
        }
        Ok(())
    }
}

impl Drop for ExtractionController {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct Failure {
    file: FileRecord,
    error: String,
    attempts: u32,
    retry_at_ms: Option<u64>,
}

type JobKey = (i64, String, Option<i64>);

fn job_key(job: &crate::ExtractionJob) -> JobKey {
    (job.file.id, job.file.image_hash.clone(), job.request_id)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as u64
}

fn recovery_id(client: &VisionClient, settings: &ExtractionSettings) -> String {
    // Credentials affect recovery, but never the extraction cache or image trailer identity.
    crate::trailer::hash_bytes(
        format!("{}\0{}", client.processor(), settings.api_key.trim()).as_bytes(),
    )
}

async fn run_worker(
    core: Arc<Core>,
    config: WorkerConfig,
    status: Arc<Mutex<ExtractionStatus>>,
    gate: Arc<tokio::sync::Mutex<()>>,
    cancelled: Arc<AtomicBool>,
) {
    if let Err(error) = worker(core, config, status.clone(), gate, cancelled).await {
        log::error!("Extraction worker stopped: {error}");
        if let Ok(mut status) = status.lock() {
            status.phase = ExtractionPhase::NeedsConfiguration;
            status.current_file = None;
            status.active_files.clear();
            status.last_error = Some(
                "Cannot read or persist extraction recovery state. Check the log and retry.".into(),
            );
        }
    }
}

async fn worker(
    core: Arc<Core>,
    config: WorkerConfig,
    status: Arc<Mutex<ExtractionStatus>>,
    gate: Arc<tokio::sync::Mutex<()>>,
    cancelled: Arc<AtomicBool>,
) -> Result<()> {
    let client = match VisionClient::new(config.settings.clone()) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            if let Ok(mut status) = status.lock() {
                status.phase = ExtractionPhase::NeedsConfiguration;
                status.last_error = Some(error.message);
            }
            return Ok(());
        }
    };
    let recovery = recovery_id(&client, &config.settings);
    let (stored, endpoint) = core
        .database
        .lock()
        .map_err(|_| Error::Poisoned)?
        .recovery()
        .load(&recovery)?;
    let mut failures: BTreeMap<JobKey, Failure> = stored
        .into_iter()
        .filter(|failure| config.folder_ids.contains(&failure.file.folder_id))
        .map(|failure| {
            (
                (
                    failure.file.id,
                    failure.file.image_hash.clone(),
                    failure.request_id,
                ),
                Failure {
                    file: failure.file,
                    error: failure.error,
                    attempts: failure.attempts,
                    retry_at_ms: failure.retry_at_ms,
                },
            )
        })
        .collect();
    let endpoint = Arc::new(tokio::sync::Mutex::new(endpoint));
    let mut tasks = JoinSet::new();
    let mut active = BTreeSet::new();
    let mut duplicate_gates: BTreeMap<String, Weak<tokio::sync::Mutex<()>>> = BTreeMap::new();
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let snapshot_core = core.clone();
        let folder_ids = config.folder_ids.clone();
        let pending = tokio::task::spawn_blocking(move || {
            let mut pending = vec![];
            for id in folder_ids {
                pending.extend(snapshot_core.extraction_jobs(id)?);
            }
            Ok::<_, Error>(pending)
        })
        .await
        .map_err(|e| Error::InvalidInput(e.to_string()))??;
        failures.retain(|key, _| pending.iter().any(|job| job_key(job) == *key));
        publish_failures(&status, &failures);
        let endpoint_state = endpoint.lock().await.clone();
        if let Some(error) = endpoint_state.blocked_error {
            if let Ok(mut status) = status.lock() {
                status.phase = ExtractionPhase::NeedsConfiguration;
                status.current_file = None;
                status.active_files.clear();
                status.last_error = Some(error);
            }
            return Ok(());
        }
        let can_start = endpoint_state.retry_at_ms.is_none_or(|at| at <= now_ms());
        if can_start {
            for job in pending {
                if tasks.len() >= config.settings.concurrency {
                    break;
                }
                let key = job_key(&job);
                if active.contains(&key)
                    || failures
                        .get(&key)
                        .is_some_and(|failure| failure.retry_at_ms.is_none_or(|at| at > now_ms()))
                {
                    continue;
                }
                active.insert(key);
                duplicate_gates.retain(|_, weak| weak.strong_count() > 0);
                let image_gate = duplicate_gates
                    .get(&job.file.image_hash)
                    .and_then(Weak::upgrade)
                    .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
                duplicate_gates.insert(job.file.image_hash.clone(), Arc::downgrade(&image_gate));
                let context = JobContext {
                    core: core.clone(),
                    client: client.clone(),
                    gate: gate.clone(),
                    cancelled: cancelled.clone(),
                    endpoint: endpoint.clone(),
                    recovery: recovery.clone(),
                    requests_per_minute: config.settings.requests_per_minute,
                };
                tasks.spawn(async move {
                    let _duplicate = image_gate.lock().await;
                    let result = process_job(&context, &job).await;
                    (job, result)
                });
            }
        }
        if let Ok(mut status) = status.lock() {
            status.active_files = active
                .iter()
                .filter_map(|key| core.file(key.0).ok().map(|file| file.file.relative_path))
                .collect();
            status.current_file = status.active_files.first().cloned();
            status.phase = if active.is_empty() {
                ExtractionPhase::Idle
            } else {
                ExtractionPhase::Extracting
            };
        }
        if tasks.is_empty() {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }
        let completed = tokio::select! {
            result = tasks.join_next() => result,
            _ = tokio::time::sleep(Duration::from_millis(500)) => continue,
        };
        let Some(completed) = completed else {
            continue;
        };
        let (job, result) = completed.map_err(|e| Error::InvalidInput(e.to_string()))?;
        let key = job_key(&job);
        active.remove(&key);
        let Some(result) = result else {
            continue;
        };
        match result {
            Ok(()) => {
                log::info!(
                    "Extraction completed for file {}: {:?}",
                    job.file.id,
                    job.file.relative_path
                );
                failures.remove(&key);
                if let Ok(mut status) = status.lock() {
                    status.completed_images += 1;
                }
            }
            Err(error) => {
                let _commit = gate.lock().await;
                if cancelled.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let attempts = failures
                    .get(&key)
                    .map_or(1, |failure| failure.attempts.saturating_add(1));
                let delay = error
                    .retry_after_seconds
                    .unwrap_or(5 * (1_u64 << attempts.min(6)))
                    .clamp(1, 900);
                let retry_at_ms =
                    (error.retryable && attempts < 5).then(|| now_ms() + delay * 1000);
                let mut state = endpoint.lock().await;
                let mut next_state = state.clone();
                if error.retryable {
                    next_state.retry_at_ms = Some(
                        next_state
                            .retry_at_ms
                            .unwrap_or(0)
                            .max(now_ms() + delay * 1000),
                    );
                }
                if error.blocks_queue {
                    next_state.blocked_error = Some(error.message.clone());
                }
                if !core
                    .database
                    .lock()
                    .map_err(|_| Error::Poisoned)?
                    .recovery()
                    .save_failure(
                        &recovery,
                        &job,
                        &error.message,
                        attempts,
                        retry_at_ms,
                        &next_state,
                    )?
                {
                    continue;
                }
                log::warn!("Extraction failed for file {}: attempt={}; retry_seconds={:?}; blocks_queue={}: {}", job.file.id, attempts, retry_at_ms.map(|_| delay), error.blocks_queue, error.message);
                *state = next_state;
                failures.insert(
                    key,
                    Failure {
                        file: job.file,
                        error: error.message,
                        attempts,
                        retry_at_ms,
                    },
                );
            }
        }
    }
}

struct JobContext {
    core: Arc<Core>,
    client: Arc<VisionClient>,
    gate: Arc<tokio::sync::Mutex<()>>,
    cancelled: Arc<AtomicBool>,
    endpoint: Arc<tokio::sync::Mutex<crate::database::EndpointState>>,
    recovery: String,
    requests_per_minute: u32,
}

async fn reserve_request(context: &JobContext) -> std::result::Result<bool, ExtractionError> {
    loop {
        if context.cancelled.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let mut state = context.endpoint.lock().await;
        if state.blocked_error.is_some() {
            return Ok(false);
        }
        let now = now_ms();
        let due = state
            .retry_at_ms
            .unwrap_or(0)
            .max(if context.requests_per_minute == 0 {
                0
            } else {
                state.next_request_ms
            });
        if due <= now {
            state.next_request_ms = now
                + if context.requests_per_minute == 0 {
                    0
                } else {
                    60_000_u64.div_ceil(context.requests_per_minute as u64)
                };
            context
                .core
                .database
                .lock()
                .map_err(|_| ExtractionError::permanent("Cannot reserve an extraction request"))?
                .recovery()
                .save_endpoint(&context.recovery, &state)
                .map_err(|_| ExtractionError::permanent("Cannot persist request pacing"))?;
            return Ok(true);
        }
        drop(state);
        tokio::time::sleep(Duration::from_millis((due - now).min(500))).await;
    }
}

async fn process_job(
    context: &JobContext,
    job: &crate::ExtractionJob,
) -> Option<std::result::Result<(), ExtractionError>> {
    let prepare_core = context.core.clone();
    let file = job.file.clone();
    let processor = context.client.processor().to_owned();
    let cache_processor = processor.clone();
    let force = job.force;
    let prepared = tokio::task::spawn_blocking(move || {
        let image = prepare_core.prepare_image(file.folder_id, &file.relative_path)?;
        let cached = if force {
            None
        } else {
            prepare_core.cached_extraction(&image.image_hash, &cache_processor)?
        };
        Ok::<_, Error>((image, cached))
    })
    .await;
    let (image, cached) = match prepared {
        Ok(Ok(prepared)) if prepared.0.image_hash == job.file.image_hash => prepared,
        Ok(Ok(_)) => return None,
        _ => return Some(Err(ExtractionError::permanent(
            "Cannot read this image for extraction; it may have moved, changed, or been excluded",
        ))),
    };
    let text = if let Some(text) = cached {
        log::info!("Reusing cached extraction for file {}", job.file.id);
        text
    } else {
        match reserve_request(context).await {
            Ok(true) => (),
            Ok(false) => return None,
            Err(error) => return Some(Err(error)),
        }
        match context.client.extract(&image).await {
            Ok(text) => text,
            Err(error) => return Some(Err(error)),
        }
    };
    let _commit = context.gate.lock().await;
    if context.cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let core = context.core.clone();
    let job = job.clone();
    let cancelled = context.cancelled.clone();
    match tokio::task::spawn_blocking(move || {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::ImageChanged);
        }
        core.complete_extraction(&job, &text, &processor)
    })
    .await
    {
        Ok(Ok(_)) => Some(Ok(())),
        Ok(Err(Error::ImageChanged | Error::NotFound(_))) => None,
        _ => Some(Err(ExtractionError::permanent(
            "Cannot save extracted text to the image",
        ))),
    }
}

fn publish_failures(status: &Mutex<ExtractionStatus>, failures: &BTreeMap<JobKey, Failure>) {
    if let Ok(mut status) = status.lock() {
        status.failed_images = failures.len();
        status.issues = failures
            .values()
            .take(20)
            .map(|failure| ExtractionIssue {
                folder_id: failure.file.folder_id,
                relative_path: failure.file.relative_path.clone(),
                error: failure.error.clone(),
                retrying: failure.retry_at_ms.is_some(),
                attempts: failure.attempts,
                retry_at_ms: failure.retry_at_ms,
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::ExtractionPhase;

    #[test]
    fn phases_preserve_the_frontend_wire_values() {
        for (phase, value) in [
            (ExtractionPhase::Disabled, "disabled"),
            (ExtractionPhase::Paused, "paused"),
            (ExtractionPhase::Idle, "idle"),
            (ExtractionPhase::Extracting, "extracting"),
            (ExtractionPhase::NeedsConfiguration, "needsConfiguration"),
        ] {
            let json = serde_json::to_string(&phase).unwrap();
            assert_eq!(json, format!("\"{value}\""));
            assert_eq!(
                serde_json::from_str::<ExtractionPhase>(&json).unwrap(),
                phase
            );
        }
    }
}
