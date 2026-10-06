use std::{
    collections::VecDeque,
    fs,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use lenscribe_core::{
    daemon::Daemon,
    llm::VisionClient,
    settings::{ExtractionSettings, FolderSettings, LlmProvider, Settings},
    trailer, Core, PreparedImage,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tokio::{sync::Semaphore, task::JoinHandle};

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Value,
    delay: Duration,
    gate: Option<Arc<Semaphore>>,
    retry_after: Option<&'static str>,
}

impl Reply {
    fn text(text: &str) -> Self {
        Self {
            status: StatusCode::OK,
            body: json!({"model":"fixture-model", "choices":[{"index":0, "finish_reason":"stop", "message":{"role":"assistant","content":text}}]}),
            delay: Duration::ZERO,
            gate: None,
            retry_after: None,
        }
    }

    fn transcription(provider: LlmProvider, text: &str) -> Self {
        let mut reply = Self::text(text);
        reply.body = match provider {
            LlmProvider::Anthropic => {
                json!({"model":"fixture-vision", "stop_reason":"end_turn", "content":[{"type":"text","text":text}]})
            }
            LlmProvider::Gemini => {
                json!({"modelVersion":"fixture-vision", "candidates":[{"finishReason":"STOP", "content":{"role":"model","parts":[{"text":text}]}}]})
            }
            LlmProvider::Ollama => {
                json!({"model":"fixture-vision", "done":true, "done_reason":"stop", "message":{"role":"assistant","content":text}})
            }
            _ => reply.body,
        };
        reply
    }
}

#[derive(Clone)]
struct MockState {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    paths: Arc<Mutex<Vec<String>>>,
    count: Arc<AtomicUsize>,
    times: Arc<Mutex<Vec<Instant>>>,
}

struct MockServer {
    url: String,
    state: MockState,
    task: JoinHandle<()>,
}

impl MockServer {
    async fn start(replies: Vec<Reply>) -> Self {
        let state = MockState {
            replies: Arc::new(Mutex::new(replies.into())),
            requests: Arc::new(Mutex::new(vec![])),
            paths: Arc::new(Mutex::new(vec![])),
            count: Arc::new(AtomicUsize::new(0)),
            times: Arc::new(Mutex::new(vec![])),
        };
        async fn complete(
            State(state): State<MockState>,
            uri: Uri,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> Response {
            state.paths.lock().unwrap().push(uri.to_string());
            state.times.lock().unwrap().push(Instant::now());
            state.requests.lock().unwrap().push((headers, body));
            state.count.fetch_add(1, Ordering::SeqCst);
            let reply = state
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Reply::text("Coffee receipt 12.00"));
            if let Some(gate) = reply.gate {
                gate.acquire().await.unwrap().forget();
            }
            if !reply.delay.is_zero() {
                tokio::time::sleep(reply.delay).await;
            }
            let mut response = (reply.status, Json(reply.body)).into_response();
            if let Some(delay) = reply.retry_after {
                response
                    .headers_mut()
                    .insert("retry-after", delay.parse().unwrap());
            }
            response
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/v1/chat/completions", post(complete))
            .route("/v1/messages", post(complete))
            .route("/v1/models/{model}", post(complete))
            .route("/v1/api/chat", post(complete))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, state, task }
    }

    fn settings(&self) -> ExtractionSettings {
        ExtractionSettings {
            enabled: true,
            base_url: self.url.clone(),
            model: "custom/vision-model".into(),
            api_key: String::new(),
            ..ExtractionSettings::default()
        }
    }

    async fn wait_for_requests(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.state.count.load(Ordering::SeqCst) < count && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(self.state.count.load(Ordering::SeqCst) >= count);
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn add_images(temporary: &TempDir, unique: bool) {
    for index in 0..2 {
        let mut bytes = include_bytes!("fixtures/pixel.jpg").to_vec();
        if unique {
            bytes.extend_from_slice(format!("original {index}").as_bytes());
        }
        fs::write(
            temporary.path().join(format!("images/extra-{index}.jpg")),
            bytes,
        )
        .unwrap();
    }
}

#[tokio::test]
async fn concurrency_is_bounded_and_reports_all_active_files() {
    let gate = Arc::new(Semaphore::new(0));
    let mut reply = Reply::text("read together");
    reply.gate = Some(gate.clone());
    let server = MockServer::start(vec![reply.clone(), reply.clone(), reply]).await;
    let (temporary, _core, daemon, mut settings) = setup(server.settings());
    settings.extraction.concurrency = 2;
    add_images(&temporary, true);
    daemon.update_settings(settings).await.unwrap();
    server.wait_for_requests(2).await;
    wait_for(&daemon, |status| status.extraction.active_files.len() == 2).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 2);
    gate.add_permits(1);
    server.wait_for_requests(3).await;
    gate.add_permits(2);
    wait_for(&daemon, |status| status.pending_images == 0).await;
    assert_eq!(daemon.status().unwrap().processed_images, 3);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn concurrent_identical_images_share_one_request_and_cached_text() {
    let gate = Arc::new(Semaphore::new(0));
    let mut reply = Reply::text("one shared extraction");
    reply.gate = Some(gate.clone());
    let server = MockServer::start(vec![reply]).await;
    let (temporary, _core, daemon, mut settings) = setup(server.settings());
    settings.extraction.concurrency = 3;
    add_images(&temporary, false);
    daemon.update_settings(settings).await.unwrap();
    server.wait_for_requests(1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 1);
    gate.add_permits(1);
    wait_for(&daemon, |status| status.pending_images == 0).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 1);
    for name in ["first.jpg", "extra-0.jpg", "extra-1.jpg"] {
        assert_eq!(
            trailer::inspect(&temporary.path().join("images").join(name))
                .unwrap()
                .trailer
                .unwrap()
                .text,
            "one shared extraction"
        );
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn concurrent_rate_limited_requests_complete_all_images() {
    let server = MockServer::start(vec![]).await;
    let (temporary, _core, daemon, mut settings) = setup(server.settings());
    add_images(&temporary, true);
    settings.extraction.concurrency = 3;
    settings.extraction.requests_per_minute = 300;
    daemon.update_settings(settings).await.unwrap();
    wait_for(&daemon, |status| status.pending_images == 0).await;
    // HTTP receive times include transport and server scheduling delays. Exact
    // admission spacing is covered by the worker's controlled-time tests.
    assert_eq!(server.state.count.load(Ordering::SeqCst), 3);
    assert_eq!(daemon.status().unwrap().processed_images, 3);
    for name in ["first.jpg", "extra-0.jpg", "extra-1.jpg"] {
        assert_eq!(
            trailer::inspect(&temporary.path().join("images").join(name))
                .unwrap()
                .trailer
                .unwrap()
                .text,
            "Coffee receipt 12.00"
        );
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn backoff_attempts_and_errors_survive_reopening_the_database() {
    let mut limited = Reply::text("unused");
    limited.status = StatusCode::TOO_MANY_REQUESTS;
    limited.retry_after = Some("3");
    let server = MockServer::start(vec![limited, Reply::text("recovered after restart")]).await;
    let (temporary, core, daemon, settings) = setup(server.settings());
    daemon.update_settings(settings).await.unwrap();
    wait_for(&daemon, |status| status.extraction.failed_images == 1).await;
    let before = daemon.status().unwrap().extraction.issues[0].clone();
    assert_eq!(before.attempts, 1);
    assert!(before.retry_at_ms.is_some());
    daemon.shutdown().await.unwrap();
    drop(daemon);
    drop(core);
    let reopened = Arc::new(Core::open(temporary.path().join("index.wedb")).unwrap());
    let restarted = Daemon::load(
        reopened,
        temporary.path().join("settings.json"),
        Arc::new(|_| {}),
    )
    .unwrap();
    restarted.start().await.unwrap();
    wait_for(&restarted, |status| status.extraction.failed_images == 1).await;
    let after = restarted.status().unwrap().extraction.issues[0].clone();
    assert_eq!(after.attempts, before.attempts);
    assert_eq!(after.retry_at_ms, before.retry_at_ms);
    assert_eq!(after.error, before.error);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 1);
    wait_for(&restarted, |status| {
        status.pending_images == 0 && status.extraction.failed_images == 0
    })
    .await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 2);
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn authentication_failure_stays_blocked_after_restart_until_the_key_changes() {
    let mut rejected = Reply::text("secret error body");
    rejected.status = StatusCode::UNAUTHORIZED;
    let server = MockServer::start(vec![rejected, Reply::text("new credentials work")]).await;
    let (temporary, core, daemon, mut settings) = setup(server.settings());
    settings.extraction.api_key = "old-key".into();
    daemon.update_settings(settings.clone()).await.unwrap();
    wait_for(&daemon, |status| {
        status.extraction.phase == lenscribe_core::extraction::ExtractionPhase::NeedsConfiguration
    })
    .await;
    daemon.shutdown().await.unwrap();
    drop(daemon);
    drop(core);
    let reopened = Arc::new(Core::open(temporary.path().join("index.wedb")).unwrap());
    let restarted = Daemon::load(
        reopened,
        temporary.path().join("settings.json"),
        Arc::new(|_| {}),
    )
    .unwrap();
    restarted.start().await.unwrap();
    wait_for(&restarted, |status| {
        status.extraction.phase == lenscribe_core::extraction::ExtractionPhase::NeedsConfiguration
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 1);
    assert_eq!(restarted.status().unwrap().extraction.issues[0].attempts, 1);
    settings.extraction.api_key = "replacement-key".into();
    restarted.update_settings(settings).await.unwrap();
    wait_for(&restarted, |status| status.pending_images == 0).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 2);
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn request_pacing_survives_restart_and_manual_retry() {
    let server = MockServer::start(vec![]).await;
    let (temporary, core, daemon, mut settings) = setup(server.settings());
    add_images(&temporary, true);
    settings.extraction.requests_per_minute = 1;
    daemon.update_settings(settings.clone()).await.unwrap();
    wait_for(&daemon, |status| status.processed_images == 1).await;
    daemon.shutdown().await.unwrap();
    drop(daemon);
    drop(core);
    let reopened = Arc::new(Core::open(temporary.path().join("index.wedb")).unwrap());
    let restarted = Daemon::load(
        reopened,
        temporary.path().join("settings.json"),
        Arc::new(|_| {}),
    )
    .unwrap();
    restarted.start().await.unwrap();
    restarted.retry().await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 1);
    settings.extraction.requests_per_minute = 0;
    restarted.update_settings(settings).await.unwrap();
    wait_for(&restarted, |status| status.pending_images == 0).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 3);
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn duplicates_reuse_matching_settings_but_reprocess_and_new_prompts_send_fresh_requests() {
    let server = MockServer::start(vec![
        Reply::text("original coffee"),
        Reply::text("fresh transcription"),
        Reply::text("new prompt result"),
    ])
    .await;
    let (temporary, core, daemon, mut settings) = setup(server.settings());
    daemon.update_settings(settings.clone()).await.unwrap();
    wait_for(&daemon, |status| {
        status.pending_images == 0 && status.total_images == 1
    })
    .await;
    fs::write(
        temporary.path().join("images/second.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    wait_for(&daemon, |status| {
        status.pending_images == 0 && status.total_images == 2
    })
    .await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 1);
    let file = core
        .snapshot(core.folders().unwrap()[0].id)
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.relative_path == "first.jpg")
        .unwrap();
    daemon
        .queue_file(file.id, file.image_hash.clone(), true)
        .await
        .unwrap();
    wait_for(&daemon, |status| {
        status.pending_images == 0 && status.extraction.completed_images == 3
    })
    .await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 2);
    assert_eq!(
        core.file(file.id).unwrap().text.as_deref(),
        Some("fresh transcription")
    );
    settings.extraction.prompt.push_str("\nUse reading order.");
    daemon.update_settings(settings).await.unwrap();
    fs::write(
        temporary.path().join("images/third.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    wait_for(&daemon, |status| {
        status.pending_images == 0 && status.total_images == 3
    })
    .await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 3);
    assert_eq!(
        trailer::inspect(&temporary.path().join("images/third.jpg"))
            .unwrap()
            .trailer
            .unwrap()
            .text,
        "new prompt result"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_reprocessing_preserves_text_and_the_forced_job_survives_a_restart() {
    let mut failure = Reply::text("truncated replacement");
    failure.body["choices"][0]["finish_reason"] = json!("length");
    let server = MockServer::start(vec![
        Reply::text("original receipt"),
        failure,
        Reply::text("recovered receipt"),
    ])
    .await;
    let (temporary, core, daemon, settings) = setup(server.settings());
    daemon.update_settings(settings).await.unwrap();
    wait_for(&daemon, |status| status.pending_images == 0).await;
    let file = core.snapshot(core.folders().unwrap()[0].id).unwrap().files[0].clone();
    daemon
        .queue_file(file.id, file.image_hash.clone(), true)
        .await
        .unwrap();
    wait_for(&daemon, |status| status.extraction.failed_images == 1).await;
    assert_eq!(
        core.file(file.id).unwrap().text.as_deref(),
        Some("original receipt")
    );
    assert_eq!(
        trailer::inspect(&temporary.path().join("images/first.jpg"))
            .unwrap()
            .trailer
            .unwrap()
            .text,
        "original receipt"
    );
    daemon.shutdown().await.unwrap();
    drop(daemon);
    let restarted = Daemon::load(
        core.clone(),
        temporary.path().join("settings.json"),
        Arc::new(|_| {}),
    )
    .unwrap();
    restarted.start().await.unwrap();
    wait_for(&restarted, |status| status.extraction.failed_images == 1).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 2);
    assert_eq!(restarted.status().unwrap().extraction.issues[0].attempts, 1);
    restarted.retry().await.unwrap();
    wait_for(&restarted, |status| status.pending_images == 0).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 3);
    assert_eq!(
        core.file(file.id).unwrap().text.as_deref(),
        Some("recovered receipt")
    );
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn retrying_one_file_does_not_reset_other_permanent_failures() {
    let mut failure = Reply::text("incomplete");
    failure.body["choices"][0]["finish_reason"] = json!("length");
    let server = MockServer::start(vec![
        failure.clone(),
        failure,
        Reply::text("targeted recovery"),
    ])
    .await;
    let (temporary, core, daemon, settings) = setup(server.settings());
    let mut different = include_bytes!("fixtures/pixel.jpg").to_vec();
    different.push(0);
    fs::write(temporary.path().join("images/second.jpg"), different).unwrap();
    daemon.update_settings(settings).await.unwrap();
    wait_for(&daemon, |status| status.extraction.failed_images == 2).await;
    let file = core
        .snapshot(core.folders().unwrap()[0].id)
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.relative_path == "first.jpg")
        .unwrap();
    daemon
        .queue_file(file.id, file.image_hash.clone(), false)
        .await
        .unwrap();
    wait_for(&daemon, |status| {
        status.pending_images == 1 && status.extraction.failed_images == 1
    })
    .await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 3);
    assert_eq!(
        core.file(file.id).unwrap().text.as_deref(),
        Some("targeted recovery")
    );
    assert_eq!(
        daemon.status().unwrap().extraction.issues[0].relative_path,
        "second.jpg"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn manual_edits_and_new_exclusions_reject_in_flight_reprocessing_results() {
    let first_gate = Arc::new(Semaphore::new(0));
    let second_gate = Arc::new(Semaphore::new(0));
    let mut delayed = Reply::text("stale replacement");
    delayed.gate = Some(first_gate.clone());
    let mut excluded = Reply::text("excluded replacement");
    excluded.gate = Some(second_gate.clone());
    let server = MockServer::start(vec![Reply::text("original receipt"), delayed, excluded]).await;
    let (temporary, core, daemon, mut settings) = setup(server.settings());
    daemon.update_settings(settings.clone()).await.unwrap();
    wait_for(&daemon, |status| status.pending_images == 0).await;
    let file = core.snapshot(core.folders().unwrap()[0].id).unwrap().files[0].clone();
    daemon
        .queue_file(file.id, file.image_hash.clone(), true)
        .await
        .unwrap();
    server.wait_for_requests(2).await;
    daemon
        .edit_file(
            file.id,
            file.image_hash.clone(),
            file.record_hash.clone(),
            "manual correction".into(),
        )
        .await
        .unwrap();
    first_gate.add_permits(1);
    wait_for(&daemon, |status| status.extraction.current_file.is_none()).await;
    assert_eq!(
        core.file(file.id).unwrap().text.as_deref(),
        Some("manual correction")
    );
    daemon
        .queue_file(file.id, file.image_hash.clone(), true)
        .await
        .unwrap();
    server.wait_for_requests(3).await;
    settings.folders[0].exclusions = vec!["first.jpg".into()];
    daemon.update_settings(settings).await.unwrap();
    second_gate.add_permits(1);
    wait_for(&daemon, |status| status.extraction.current_file.is_none()).await;
    assert_eq!(daemon.status().unwrap().total_images, 0);
    assert_eq!(
        trailer::inspect(&temporary.path().join("images/first.jpg"))
            .unwrap()
            .trailer
            .unwrap()
            .text,
        "manual correction"
    );
    daemon.shutdown().await.unwrap();
}

fn image() -> PreparedImage {
    let bytes = include_bytes!("fixtures/pixel.jpg").to_vec();
    PreparedImage {
        folder_id: 1,
        relative_path: "fixture.jpg".into(),
        image_hash: trailer::hash_bytes(&bytes),
        mime_type: "image/jpeg".into(),
        bytes,
    }
}

fn setup(settings: ExtractionSettings) -> (TempDir, Arc<Core>, Arc<Daemon>, Settings) {
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
    let settings = Settings {
        folders: vec![FolderSettings {
            path: images.to_str().unwrap().into(),
            enabled: true,
            ..lenscribe_core::settings::FolderSettings::default()
        }],
        extraction: settings,
        ..Settings::default()
    };
    (temporary, core, daemon, settings)
}

async fn wait_for(
    daemon: &Daemon,
    predicate: impl Fn(&lenscribe_core::daemon::DaemonStatus) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let status = daemon.status().unwrap();
        if predicate(&status) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "status did not converge: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn slow_writes_settle_before_extraction_and_preserve_the_completed_image() {
    let server = MockServer::start(vec![Reply::text("complete receipt")]).await;
    let (temporary, core, daemon, settings) = setup(server.settings());
    daemon.update_settings(settings).await.unwrap();
    let path = temporary.path().join("images/first.jpg");
    let mut original = include_bytes!("fixtures/pixel.jpg").to_vec();
    for chunk in 0..8 {
        original.extend_from_slice(format!("chunk {chunk}").as_bytes());
        fs::write(&path, &original).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(server.state.count.load(Ordering::SeqCst), 0);
    }
    let settled = Instant::now();
    server.wait_for_requests(1).await;
    assert!(
        server.state.times.lock().unwrap()[0].duration_since(settled) >= Duration::from_millis(750)
    );
    wait_for(&daemon, |status| status.pending_images == 0).await;
    let inspected = trailer::inspect(&path).unwrap();
    assert_eq!(inspected.image_hash, trailer::hash_bytes(&original));
    assert_eq!(&fs::read(&path).unwrap()[..original.len()], &original);
    assert_eq!(core.search("complete", None, 10).unwrap().len(), 1);
    daemon.shutdown().await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn a_lock_during_commit_defers_the_image_without_blocking_healthy_work() {
    use std::os::windows::fs::OpenOptionsExt;
    let gate = Arc::new(Semaphore::new(0));
    let mut first = Reply::text("first response");
    first.gate = Some(gate.clone());
    let server = MockServer::start(vec![
        first,
        Reply::text("healthy image"),
        Reply::text("retried image"),
    ])
    .await;
    let (temporary, core, daemon, settings) = setup(server.settings());
    daemon.update_settings(settings).await.unwrap();
    server.wait_for_requests(1).await;
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(temporary.path().join("images/first.jpg"))
        .unwrap();
    gate.add_permits(1);
    let mut healthy = include_bytes!("fixtures/pixel.jpg").to_vec();
    healthy.extend_from_slice(b"healthy original");
    fs::write(temporary.path().join("images/healthy.jpg"), healthy).unwrap();
    server.wait_for_requests(2).await;
    wait_for(&daemon, |status| {
        status.processed_images == 1 && status.pending_images == 1
    })
    .await;
    assert_eq!(daemon.status().unwrap().extraction.failed_images, 0);
    assert_eq!(core.search("healthy", None, 10).unwrap().len(), 1);
    drop(locked);
    wait_for(&daemon, |status| status.pending_images == 0).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 2);
    assert_eq!(daemon.status().unwrap().extraction.failed_images, 0);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn vision_request_targets_custom_endpoint_with_original_bytes_and_preserves_text() {
    let server = MockServer::start(vec![Reply::text("  Café\nTotal: 12.00\n")]).await;
    let client = VisionClient::new(server.settings()).unwrap();
    assert_eq!(
        client.extract(&image()).await.unwrap(),
        "  Café\nTotal: 12.00\n"
    );
    let requests = server.state.requests.lock().unwrap().clone();
    let (headers, body) = &requests[0];
    assert!(!headers.contains_key("authorization"));
    assert_eq!(body["model"], "custom/vision-model");
    assert_eq!(body["stream"], false);
    assert_eq!(body["max_tokens"], 8192);
    assert_eq!(body["messages"][0]["role"], "system");
    let data = body["messages"][1]["content"][1]["image_url"]["url"]
        .as_str()
        .unwrap();
    let encoded = data.strip_prefix("data:image/jpeg;base64,").unwrap();
    assert_eq!(STANDARD.decode(encoded).unwrap(), image().bytes);
    assert!(!body.to_string().contains("fixture.jpg"));
    assert!(client
        .processor()
        .starts_with("openai-compatible/custom/vision-model/transcription-v1/"));
}

#[tokio::test]
async fn provider_requests_use_the_selected_protocol_endpoint_auth_and_original_image() {
    for provider in [
        LlmProvider::Custom,
        LlmProvider::OpenAi,
        LlmProvider::Anthropic,
        LlmProvider::Gemini,
        LlmProvider::OpenRouter,
        LlmProvider::Ollama,
        LlmProvider::Groq,
        LlmProvider::Xai,
    ] {
        let server = MockServer::start(vec![Reply::transcription(
            provider,
            "  Café\nTotal: 12.00\n",
        )])
        .await;
        let mut settings = server.settings();
        settings.provider = provider;
        settings.model = if provider == LlmProvider::Gemini {
            "models/fixture-vision"
        } else {
            "fixture-vision"
        }
        .into();
        settings.api_key = if provider.requires_api_key() {
            "fixture-secret"
        } else {
            ""
        }
        .into();
        let prompt = settings.prompt.clone();
        let client = VisionClient::new(settings).unwrap();
        assert_eq!(
            client.extract(&image()).await.unwrap(),
            "  Café\nTotal: 12.00\n",
            "{provider:?}"
        );
        assert!(!client.processor().contains("fixture-secret"));
        let requests = server.state.requests.lock().unwrap();
        let (headers, body) = &requests[0];
        let paths = server.state.paths.lock().unwrap();
        let encoded = match provider {
            LlmProvider::Anthropic => {
                assert_eq!(paths[0], "/v1/messages");
                assert_eq!(headers["x-api-key"], "fixture-secret");
                assert_eq!(headers["anthropic-version"], "2023-06-01");
                assert!(!headers.contains_key("authorization"));
                assert_eq!(body["max_tokens"], 8192);
                assert!(body["system"].to_string().contains(&prompt));
                assert_eq!(
                    body["messages"][0]["content"][1]["source"]["media_type"],
                    "image/jpeg"
                );
                body["messages"][0]["content"][1]["source"]["data"]
                    .as_str()
                    .unwrap()
            }
            LlmProvider::Gemini => {
                assert_eq!(paths[0], "/v1/models/fixture-vision:generateContent");
                assert_eq!(headers["x-goog-api-key"], "fixture-secret");
                assert!(!headers.contains_key("authorization"));
                assert_eq!(body["generationConfig"]["maxOutputTokens"], 8192);
                assert_eq!(body["systemInstruction"]["parts"][0]["text"], prompt);
                assert_eq!(
                    body["contents"][0]["parts"][1]["inline_data"]["mime_type"],
                    "image/jpeg"
                );
                body["contents"][0]["parts"][1]["inline_data"]["data"]
                    .as_str()
                    .unwrap()
            }
            LlmProvider::Ollama => {
                assert_eq!(paths[0], "/v1/api/chat");
                assert!(!headers.contains_key("authorization"));
                assert_eq!(body["options"]["num_predict"], 8192);
                assert_eq!(body["stream"], false);
                assert_eq!(body["messages"][0]["content"], prompt);
                body["messages"][1]["images"][0].as_str().unwrap()
            }
            _ => {
                assert_eq!(paths[0], "/v1/chat/completions");
                if provider.requires_api_key() {
                    assert_eq!(headers["authorization"], "Bearer fixture-secret");
                } else {
                    assert!(!headers.contains_key("authorization"));
                }
                assert_eq!(body["messages"][0]["content"], prompt);
                body["messages"][1]["content"][1]["image_url"]["url"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("data:image/jpeg;base64,")
                    .unwrap()
            }
        };
        assert_eq!(STANDARD.decode(encoded).unwrap(), image().bytes);
        assert!(!body.to_string().contains("fixture.jpg"));
        assert!(!body.to_string().contains("LENSCRIBE-TEXT"));
    }
}

#[tokio::test]
async fn native_providers_validate_completion_and_do_not_save_reasoning_or_partial_results() {
    for provider in [
        LlmProvider::Anthropic,
        LlmProvider::Gemini,
        LlmProvider::Ollama,
    ] {
        let empty = Reply::transcription(provider, "");
        let mut reasoning = Reply::transcription(provider, "  Café\nPaid\n");
        let mut truncated = Reply::transcription(provider, "private partial text");
        let mut refused = Reply::transcription(provider, "private refusal details");
        let mut missing = Reply::transcription(provider, "unused");
        match provider {
            LlmProvider::Anthropic => {
                reasoning.body["content"] = json!([{"type":"thinking","thinking":"private reasoning"},{"type":"text","text":"  Café\n"},{"type":"text","text":"Paid\n"}]);
                truncated.body["stop_reason"] = json!("max_tokens");
                refused.body["stop_reason"] = json!("refusal");
                missing.body["content"] = json!([]);
            }
            LlmProvider::Gemini => {
                reasoning.body["candidates"][0]["content"]["parts"] = json!([{"thought":true,"text":"private reasoning"},{"text":"  Café\n","thoughtSignature":"private signature"},{"text":"Paid\n"}]);
                truncated.body["candidates"][0]["finishReason"] = json!("MAX_TOKENS");
                refused.body["candidates"][0]["finishReason"] = json!("SAFETY");
                missing.body["candidates"][0]["content"]["parts"] = json!([]);
            }
            LlmProvider::Ollama => {
                reasoning.body["message"]["thinking"] = json!("private reasoning");
                truncated.body["done_reason"] = json!("length");
                refused.body["done"] = json!(false);
                missing.body["message"]["content"] = Value::Null;
            }
            _ => unreachable!(),
        }
        let server = MockServer::start(vec![empty, reasoning, truncated, refused, missing]).await;
        let mut settings = server.settings();
        settings.provider = provider;
        settings.model = "fixture-vision".into();
        settings.api_key = "fixture-secret".into();
        let client = VisionClient::new(settings).unwrap();
        assert_eq!(client.extract(&image()).await.unwrap(), "", "{provider:?}");
        assert_eq!(
            client.extract(&image()).await.unwrap(),
            "  Café\nPaid\n",
            "{provider:?}"
        );
        for _ in 0..3 {
            let error = client.extract(&image()).await.unwrap_err();
            assert!(!error.retryable);
            assert!(!error.message.contains("private"));
            assert!(!error.message.contains("fixture-secret"));
        }
    }
}

#[tokio::test]
async fn webp_arrivals_are_watched_extracted_and_sent_without_the_text_trailer() {
    for provider in [
        LlmProvider::Custom,
        LlmProvider::Anthropic,
        LlmProvider::Gemini,
        LlmProvider::Ollama,
    ] {
        let server = MockServer::start(vec![
            Reply::transcription(provider, "Previous private text"),
            Reply::transcription(provider, "WebP coffee receipt"),
        ])
        .await;
        let mut extraction = server.settings();
        extraction.provider = provider;
        extraction.model = "fixture-vision".into();
        extraction.api_key = "fixture-secret".into();
        let (temporary, core, daemon, settings) = setup(extraction);
        let images = temporary.path().join("images");
        fs::remove_file(images.join("first.jpg")).unwrap();
        daemon.update_settings(settings).await.unwrap();
        wait_for(&daemon, |status| {
            status.folder_statuses.len() == 1 && status.folder_statuses[0].watching
        })
        .await;
        fs::create_dir(images.join("nested")).unwrap();
        let relative = "nested/receipt.WEBP";
        let original = include_bytes!("fixtures/pixel-lossless.webp");
        let path = images.join(relative);
        fs::write(&path, original).unwrap();
        let image_hash = trailer::hash_bytes(original);
        wait_for(&daemon, |status| {
            status.total_images == 1 && status.extraction.completed_images == 1
        })
        .await;
        assert_eq!(
            trailer::inspect(&path).unwrap().trailer.unwrap().text,
            "Previous private text"
        );
        let snapshot = core
            .snapshot(
                daemon.status().unwrap().folder_statuses[0]
                    .folder_id
                    .unwrap(),
            )
            .unwrap();
        let file = &snapshot.files[0];
        // Existing embedded text must stay local when a fresh extraction is requested.
        daemon
            .queue_file(file.id, file.image_hash.clone(), true)
            .await
            .unwrap();
        wait_for(&daemon, |status| {
            status.extraction.completed_images == 2 && status.extraction.current_file.is_none()
        })
        .await;
        assert_eq!(server.state.count.load(Ordering::SeqCst), 2);
        let saved = trailer::inspect(&path).unwrap();
        assert_eq!(saved.mime_type, "image/webp");
        assert_eq!(saved.trailer.unwrap().text, "WebP coffee receipt");
        assert_eq!(
            trailer::original_bytes(&path, &image_hash).unwrap(),
            original
        );
        assert_eq!(core.search("coffee", None, 10).unwrap().len(), 1);
        {
            let requests = server.state.requests.lock().unwrap();
            for (_, body) in requests.iter() {
                let encoded = match provider {
                    LlmProvider::Anthropic => {
                        assert_eq!(
                            body["messages"][0]["content"][1]["source"]["media_type"],
                            "image/webp"
                        );
                        body["messages"][0]["content"][1]["source"]["data"]
                            .as_str()
                            .unwrap()
                    }
                    LlmProvider::Gemini => {
                        assert_eq!(
                            body["contents"][0]["parts"][1]["inline_data"]["mime_type"],
                            "image/webp"
                        );
                        body["contents"][0]["parts"][1]["inline_data"]["data"]
                            .as_str()
                            .unwrap()
                    }
                    LlmProvider::Ollama => body["messages"][1]["images"][0].as_str().unwrap(),
                    _ => body["messages"][1]["content"][1]["image_url"]["url"]
                        .as_str()
                        .unwrap()
                        .strip_prefix("data:image/webp;base64,")
                        .unwrap(),
                };
                assert_eq!(STANDARD.decode(encoded).unwrap(), original);
                assert!(!body.to_string().contains("Previous private text"));
            }
        }
        daemon.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn native_provider_workers_append_only_transcription_to_images() {
    for provider in [
        LlmProvider::Anthropic,
        LlmProvider::Gemini,
        LlmProvider::Ollama,
    ] {
        let server =
            MockServer::start(vec![Reply::transcription(provider, "Café receipt 12.00")]).await;
        let mut extraction = server.settings();
        extraction.provider = provider;
        extraction.model = "fixture-vision".into();
        extraction.api_key = "fixture-secret".into();
        let (temporary, core, daemon, settings) = setup(extraction);
        daemon.update_settings(settings).await.unwrap();
        wait_for(&daemon, |status| {
            status.pending_images == 0 && status.total_images == 1
        })
        .await;
        assert_eq!(core.search("receipt", None, 10).unwrap().len(), 1);
        let saved = trailer::inspect(&temporary.path().join("images/first.jpg"))
            .unwrap()
            .trailer
            .unwrap();
        assert_eq!(saved.text, "Café receipt 12.00");
        assert!(!saved.processor.contains("fixture-secret"));
        daemon.shutdown().await.unwrap();
    }
}

#[test]
fn legacy_provider_settings_and_cache_identity_are_preserved_and_new_providers_are_separate() {
    let original = ExtractionSettings {
        enabled: true,
        base_url: "https://example.com/v1".into(),
        model: "fixture-vision".into(),
        api_key: "fixture-secret".into(),
        ..ExtractionSettings::default()
    };
    let mut old_json = serde_json::to_value(&original).unwrap();
    old_json.as_object_mut().unwrap().remove("provider");
    let restored: ExtractionSettings = serde_json::from_value(old_json).unwrap();
    assert_eq!(restored, original);
    let identity = json!({"format":"lenscribe-transcription-v1", "endpoint":original.normalized_base_url().unwrap(), "model":original.model, "prompt":original.prompt, "maxTokens":original.max_tokens});
    let expected = format!(
        "openai-compatible/fixture-vision/transcription-v1/{}",
        trailer::hash_bytes(identity.to_string().as_bytes())
    );
    assert_eq!(VisionClient::new(restored).unwrap().processor(), expected);
    let mut identities = std::collections::BTreeSet::from([expected]);
    for provider in [
        LlmProvider::OpenAi,
        LlmProvider::Anthropic,
        LlmProvider::Gemini,
        LlmProvider::OpenRouter,
        LlmProvider::Ollama,
        LlmProvider::Groq,
        LlmProvider::Xai,
    ] {
        let mut settings = original.clone();
        settings.provider = provider;
        assert!(identities.insert(
            VisionClient::new(settings.clone())
                .unwrap()
                .processor()
                .to_owned()
        ));
        settings.api_key = "changed-key".into();
        assert!(identities.contains(VisionClient::new(settings).unwrap().processor()));
    }
}

#[test]
fn provider_validation_requires_cloud_keys_and_rejects_generation_urls_and_model_paths() {
    for provider in [
        LlmProvider::OpenAi,
        LlmProvider::Anthropic,
        LlmProvider::Gemini,
        LlmProvider::OpenRouter,
        LlmProvider::Ollama,
        LlmProvider::Groq,
        LlmProvider::Xai,
        LlmProvider::Custom,
    ] {
        let mut settings = ExtractionSettings {
            enabled: true,
            provider,
            model: "fixture-vision".into(),
            ..ExtractionSettings::default()
        };
        assert_eq!(settings.validate().is_err(), provider.requires_api_key());
        settings.api_key = "fixture-key".into();
        settings.validate().unwrap();
        for suffix in ["/messages", "/api/chat", "/models/fixture:generateContent"] {
            settings.base_url = format!("https://example.com{suffix}");
            assert!(settings.validate().is_err());
        }
        settings.base_url = "https://example.com/v1".into();
        settings.api_key.clear();
        settings.enabled = false;
        settings.validate().unwrap();
    }
    for model in [
        "../other",
        "model?key=secret",
        "model#fragment",
        "models/",
        "..",
        "models/..",
        "gemini::fixture",
    ] {
        let settings = ExtractionSettings {
            provider: LlmProvider::Gemini,
            model: model.into(),
            ..ExtractionSettings::default()
        };
        assert!(settings.validate().is_err());
    }
}

#[tokio::test]
async fn api_key_round_trips_in_settings_and_gpt_models_still_use_chat_completions() {
    let server = MockServer::start(vec![]).await;
    let temporary = tempdir().unwrap();
    let mut settings = server.settings();
    settings.api_key = "fixture-secret-not-a-real-api-key".into();
    settings.model = "gpt-6-vision".into();
    let saved = Settings {
        extraction: settings.clone(),
        ..Settings::default()
    };
    let path = temporary.path().join("settings.json");
    saved.save(&path).unwrap();
    let restored = Settings::load(&path).unwrap();
    assert_eq!(restored.extraction.api_key, settings.api_key);
    assert!(!format!("{restored:?}").contains("fixture-secret"));
    let client = VisionClient::new(restored.extraction).unwrap();
    client.extract(&image()).await.unwrap();
    let requests = server.state.requests.lock().unwrap();
    assert_eq!(
        requests[0].0["authorization"],
        "Bearer fixture-secret-not-a-real-api-key"
    );
    assert_eq!(requests[0].1["model"], "gpt-6-vision");
    assert_eq!(requests[0].1["max_completion_tokens"], 8192);
    assert!(requests[0].1.get("max_tokens").is_none());
    assert!(serde_json::to_string(&settings)
        .unwrap()
        .contains("fixture-secret"));
    assert!(!client.processor().contains("fixture-secret"));
}

#[tokio::test]
async fn empty_transcription_is_valid_but_truncated_refused_and_missing_content_are_rejected() {
    let mut truncated = Reply::text("partial");
    truncated.body["choices"][0]["finish_reason"] = json!("length");
    let mut refused = Reply::text("refused");
    refused.body["choices"][0]["message"]["refusal"] = json!("refusal details");
    let mut missing = Reply::text("text");
    missing.body["choices"][0]["message"]["content"] = Value::Null;
    let mut no_choices = Reply::text("text");
    no_choices.body["choices"] = json!([]);
    let mut filtered = Reply::text("text");
    filtered.body["choices"][0]["finish_reason"] = json!("content_filter");
    let server = MockServer::start(vec![
        Reply::text(""),
        truncated,
        refused,
        missing,
        no_choices,
        filtered,
    ])
    .await;
    let client = VisionClient::new(server.settings()).unwrap();
    assert_eq!(client.extract(&image()).await.unwrap(), "");
    for _ in 0..5 {
        let error = client.extract(&image()).await.unwrap_err();
        assert!(!error.retryable);
        assert!(!error.to_string().contains("refusal details"));
    }
}

#[tokio::test]
async fn rate_limits_and_timeouts_are_retryable_without_exposing_endpoint_bodies() {
    let mut rate_limit = Reply::text("unused");
    rate_limit.status = StatusCode::TOO_MANY_REQUESTS;
    rate_limit.body = json!({"error":"private response image and API secret"});
    rate_limit.retry_after = Some("1");
    let mut timeout = Reply::text("late");
    timeout.delay = Duration::from_secs(2);
    let server = MockServer::start(vec![rate_limit, timeout]).await;
    let mut settings = server.settings();
    settings.timeout_seconds = 1;
    let client = VisionClient::new(settings).unwrap();
    let error = client.extract(&image()).await.unwrap_err();
    assert!(error.retryable);
    assert_eq!(error.retry_after_seconds, Some(1));
    assert!(!error.to_string().contains("private"));
    let error = client.extract(&image()).await.unwrap_err();
    assert!(error.retryable);
    assert_eq!(error.message, "LLM request timed out");
}

#[test]
fn settings_migrate_without_enabling_uploads_and_validate_url_model_and_api_key() {
    let settings: Settings = serde_json::from_value(
        json!({"version":1,"folders":[],"api":{"enabled":false,"port":47831}}),
    )
    .unwrap();
    assert!(!settings.extraction.enabled);
    settings.validate().unwrap();
    let mut extraction = ExtractionSettings {
        enabled: true,
        model: "vision-model".into(),
        ..ExtractionSettings::default()
    };
    for url in [
        "file:///tmp/images",
        "https://user:secret@example.com/v1",
        "https://example.com/v1?key=secret",
        "https://example.com/v1/chat/completions",
    ] {
        extraction.base_url = url.into();
        assert!(extraction.validate().is_err());
    }
    extraction.base_url = "https://example.com/api/v1/".into();
    extraction.validate().unwrap();
    assert_eq!(
        extraction.normalized_base_url().unwrap(),
        "https://example.com/api/v1/"
    );
    extraction.api_key = "invalid\nheader-value".into();
    assert!(extraction.validate().is_err());
    extraction.api_key.clear();
    extraction.model.clear();
    assert!(extraction.validate().is_err());
}

#[tokio::test]
async fn worker_appends_text_updates_search_and_processes_new_files_without_a_ui() {
    let server = MockServer::start(vec![]).await;
    let (temporary, core, daemon, settings) = setup(server.settings());
    settings
        .save(&temporary.path().join("settings.json"))
        .unwrap();
    drop(daemon);
    let daemon = Daemon::load(
        core.clone(),
        temporary.path().join("settings.json"),
        Arc::new(|_| {}),
    )
    .unwrap();
    daemon.start().await.unwrap();
    wait_for(&daemon, |status| {
        status.pending_images == 0 && status.extraction.completed_images == 1
    })
    .await;
    assert_eq!(core.search("coffee", None, 10).unwrap().len(), 1);
    let first = temporary.path().join("images/first.jpg");
    let inspected = trailer::inspect(&first).unwrap();
    assert_eq!(inspected.trailer.unwrap().text, "Coffee receipt 12.00");
    let snapshot = core.snapshot(core.folders().unwrap()[0].id).unwrap();
    let prepared = core.prepare_image(snapshot.folder.id, "first.jpg").unwrap();
    assert_eq!(prepared.bytes, include_bytes!("fixtures/pixel.jpg"));
    fs::write(
        temporary.path().join("images/second.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    wait_for(&daemon, |status| {
        status.pending_images == 0 && status.extraction.completed_images == 2
    })
    .await;
    assert_eq!(core.search("receipt", None, 10).unwrap().len(), 2);
    daemon.shutdown().await.unwrap();
    drop(daemon);
    let restarted = Daemon::load(
        core.clone(),
        temporary.path().join("settings.json"),
        Arc::new(|_| {}),
    )
    .unwrap();
    restarted.start().await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 1); // The identical second image reuses the first result.
    assert_eq!(restarted.status().unwrap().pending_images, 0);
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn pausing_cancels_in_flight_extraction_and_resuming_catches_up() {
    let gate = Arc::new(Semaphore::new(0));
    let mut blocked = Reply::text("old response");
    blocked.gate = Some(gate.clone());
    let server = MockServer::start(vec![blocked, Reply::text("fresh response")]).await;
    let (temporary, core, daemon, mut settings) = setup(server.settings());
    daemon.update_settings(settings.clone()).await.unwrap();
    server.wait_for_requests(1).await;
    settings.monitoring_paused = true;
    daemon.update_settings(settings.clone()).await.unwrap();
    gate.add_permits(1);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(trailer::inspect(&temporary.path().join("images/first.jpg"))
        .unwrap()
        .trailer
        .is_none());
    assert_eq!(
        daemon.status().unwrap().extraction.phase,
        lenscribe_core::extraction::ExtractionPhase::Paused
    );
    settings.monitoring_paused = false;
    daemon.update_settings(settings).await.unwrap();
    wait_for(&daemon, |status| status.pending_images == 0).await;
    assert_eq!(core.search("fresh response", None, 10).unwrap().len(), 1);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn changing_provider_configuration_cancels_old_responses_before_commit() {
    let gate = Arc::new(Semaphore::new(0));
    let mut blocked = Reply::text("old model text");
    blocked.gate = Some(gate.clone());
    let server = MockServer::start(vec![blocked, Reply::text("new model text")]).await;
    let (_temporary, core, daemon, mut settings) = setup(server.settings());
    daemon.update_settings(settings.clone()).await.unwrap();
    server.wait_for_requests(1).await;
    settings.extraction.model = "new/vision-model".into();
    daemon.update_settings(settings).await.unwrap();
    gate.add_permits(1);
    wait_for(&daemon, |status| status.pending_images == 0).await;
    let file = core
        .snapshot(core.folders().unwrap()[0].id)
        .unwrap()
        .files
        .remove(0);
    assert_eq!(core.file(file.id).unwrap().text.unwrap(), "new model text");
    assert!(file.processor.unwrap().contains("new/vision-model"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_responses_stay_pending_and_manual_retry_processes_them() {
    let mut truncated = Reply::text("incomplete receipt");
    truncated.body["choices"][0]["finish_reason"] = json!("length");
    let server = MockServer::start(vec![truncated, Reply::text("complete receipt")]).await;
    let (temporary, _core, daemon, settings) = setup(server.settings());
    daemon.update_settings(settings).await.unwrap();
    wait_for(&daemon, |status| status.extraction.failed_images == 1).await;
    assert_eq!(daemon.status().unwrap().pending_images, 1);
    assert!(!daemon.status().unwrap().extraction.issues[0].retrying);
    assert!(trailer::inspect(&temporary.path().join("images/first.jpg"))
        .unwrap()
        .trailer
        .is_none());
    daemon.retry().await.unwrap();
    wait_for(&daemon, |status| {
        status.pending_images == 0 && status.extraction.failed_images == 0
    })
    .await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn endpoint_authentication_errors_stop_the_queue_until_explicit_retry() {
    let mut unauthorized = Reply::text("unused");
    unauthorized.status = StatusCode::UNAUTHORIZED;
    unauthorized.body = json!({"error":"private secret from upstream"});
    let server = MockServer::start(vec![unauthorized]).await;
    let (temporary, _core, daemon, settings) = setup(server.settings());
    fs::write(
        temporary.path().join("images/second.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    daemon.update_settings(settings).await.unwrap();
    wait_for(&daemon, |status| {
        status.extraction.phase == lenscribe_core::extraction::ExtractionPhase::NeedsConfiguration
    })
    .await;
    daemon.reconcile().await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 1);
    assert_eq!(daemon.status().unwrap().pending_images, 2);
    assert!(!serde_json::to_string(&daemon.status().unwrap())
        .unwrap()
        .contains("private secret"));
    daemon.retry().await.unwrap();
    wait_for(&daemon, |status| status.pending_images == 0).await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn rate_limited_worker_honors_backoff_then_retries_successfully() {
    let mut limited = Reply::text("unused");
    limited.status = StatusCode::TOO_MANY_REQUESTS;
    limited.retry_after = Some("1");
    let server = MockServer::start(vec![limited, Reply::text("retried receipt")]).await;
    let (_temporary, _core, daemon, settings) = setup(server.settings());
    daemon.update_settings(settings).await.unwrap();
    wait_for(&daemon, |status| status.extraction.failed_images == 1).await;
    assert!(daemon.status().unwrap().extraction.issues[0].retrying);
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 1);
    wait_for(&daemon, |status| {
        status.pending_images == 0 && status.extraction.failed_images == 0
    })
    .await;
    assert_eq!(server.state.count.load(Ordering::SeqCst), 2);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_key_is_rejected_without_sending_any_images() {
    let server = MockServer::start(vec![]).await;
    let mut extraction = server.settings();
    extraction.api_key = "invalid\nheader-value".into();
    let (_temporary, _core, daemon, settings) = setup(extraction);
    assert!(daemon.update_settings(settings).await.is_err());
    assert_eq!(
        daemon.status().unwrap().extraction.phase,
        lenscribe_core::extraction::ExtractionPhase::Disabled
    );
    assert_eq!(server.state.count.load(Ordering::SeqCst), 0);
    daemon.shutdown().await.unwrap();
}

#[test]
fn legacy_variable_configuration_loads_without_env_support_and_saves_the_new_key_field() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("settings.json");
    fs::write(&path, serde_json::to_vec(&json!({
        "version": 1,
        "extraction": { "enabled": true, "baseUrl": "https://example.com/v1", "model": "vision-model", "apiKeyEnv": "LEGACY_KEY_VARIABLE" }
    })).unwrap()).unwrap();
    let mut settings = Settings::load(&path).unwrap();
    assert!(!settings.extraction.enabled);
    assert!(settings.extraction.api_key.is_empty());
    assert_eq!(settings.extraction.base_url, "https://example.com/v1");
    settings.extraction.api_key = "fixture-key".into();
    settings.extraction.enabled = true;
    settings.save(&path).unwrap();
    let saved = fs::read_to_string(path).unwrap();
    assert!(!saved.contains("apiKeyEnv"));
    assert!(saved.contains("\"apiKey\": \"fixture-key\""));
}

#[tokio::test]
async fn editing_the_api_key_restarts_extraction_without_restarting_the_app() {
    let gate = Arc::new(Semaphore::new(0));
    let mut blocked = Reply::text("old key response");
    blocked.gate = Some(gate.clone());
    let server = MockServer::start(vec![blocked, Reply::text("new key response")]).await;
    let mut extraction = server.settings();
    extraction.api_key = "old-fixture-key".into();
    let (_temporary, core, daemon, mut settings) = setup(extraction);
    daemon.update_settings(settings.clone()).await.unwrap();
    server.wait_for_requests(1).await;
    settings.extraction.api_key = "new-fixture-key".into();
    daemon.update_settings(settings).await.unwrap();
    gate.add_permits(1);
    wait_for(&daemon, |status| status.pending_images == 0).await;
    assert_eq!(core.search("new key response", None, 10).unwrap().len(), 1);
    let requests = server.state.requests.lock().unwrap().clone();
    assert_eq!(requests[0].0["authorization"], "Bearer old-fixture-key");
    assert_eq!(requests[1].0["authorization"], "Bearer new-fixture-key");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn removed_or_changed_images_cannot_receive_an_old_in_flight_response() {
    let gate = Arc::new(Semaphore::new(0));
    let mut blocked = Reply::text("stale text");
    blocked.gate = Some(gate.clone());
    let server = MockServer::start(vec![blocked]).await;
    let (temporary, core, daemon, settings) = setup(server.settings());
    daemon.update_settings(settings).await.unwrap();
    server.wait_for_requests(1).await;
    let first = temporary.path().join("images/first.jpg");
    let mut changed = include_bytes!("fixtures/pixel.jpg").to_vec();
    changed.extend_from_slice(b"changed original trailing bytes");
    fs::write(&first, &changed).unwrap();
    core.scan_folder(Path::new(&daemon.settings().unwrap().folders[0].path))
        .unwrap();
    gate.add_permits(1);
    wait_for(&daemon, |status| status.pending_images == 0).await;
    let inspected = trailer::inspect(&first).unwrap();
    assert_eq!(inspected.image_hash, trailer::hash_bytes(&changed));
    assert_ne!(inspected.trailer.unwrap().text, "stale text");
    daemon.shutdown().await.unwrap();
}
