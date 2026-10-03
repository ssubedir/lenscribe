use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use lenscribe_core::{
    llm::discover_models,
    settings::{ExtractionSettings, LlmProvider, SavedConnection, Settings},
};
use serde_json::{json, Value};
use tempfile::tempdir;
use tokio::task::JoinHandle;

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: String,
    location: Option<String>,
}

impl Reply {
    fn json(body: Value) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.to_string(),
            location: None,
        }
    }
}

#[derive(Clone)]
struct StateData {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<(String, HeaderMap)>>>,
}

struct Server {
    url: String,
    state: StateData,
    task: JoinHandle<()>,
}

impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let state = StateData {
            replies: Arc::new(Mutex::new(replies.into())),
            requests: Arc::new(Mutex::new(vec![])),
        };
        async fn catalog(State(state): State<StateData>, uri: Uri, headers: HeaderMap) -> Response {
            state
                .requests
                .lock()
                .unwrap()
                .push((uri.to_string(), headers));
            let reply = state
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Reply::json(json!({"data":[]})));
            let mut response = (reply.status, reply.body).into_response();
            if let Some(location) = reply.location {
                response
                    .headers_mut()
                    .insert("location", location.parse().unwrap());
            }
            response
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/v1/models", get(catalog))
            .route("/v1/api/tags", get(catalog))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, state, task }
    }

    fn settings(&self, provider: LlmProvider) -> ExtractionSettings {
        ExtractionSettings {
            provider,
            base_url: self.url.clone(),
            model: String::new(),
            api_key: "fixture-key".into(),
            ..ExtractionSettings::default()
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn catalogs_use_each_providers_endpoint_and_header_without_a_selected_model() {
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
        let body = match provider {
            LlmProvider::Gemini => {
                json!({"models":[{"name":"models/vision", "displayName":"Vision model", "supportedGenerationMethods":["generateContent"]}]})
            }
            LlmProvider::Ollama => json!({"models":[{"name":"vision:latest"}]}),
            _ => json!({"data":[{"id":"vision-model"}]}),
        };
        let server = Server::start(vec![Reply::json(body)]).await;
        let result = discover_models(server.settings(provider)).await.unwrap();
        assert_eq!(result.models.len(), 1);
        assert_eq!(result.models[0].vision, None);
        let requests = server.state.requests.lock().unwrap();
        let (path, headers) = &requests[0];
        assert_eq!(
            path,
            if provider == LlmProvider::Ollama {
                "/v1/api/tags"
            } else {
                "/v1/models"
            }
        );
        match provider {
            LlmProvider::Anthropic => {
                assert_eq!(headers["x-api-key"], "fixture-key");
                assert_eq!(headers["anthropic-version"], "2023-06-01");
                assert!(!headers.contains_key("authorization"));
            }
            LlmProvider::Gemini => {
                assert_eq!(headers["x-goog-api-key"], "fixture-key");
                assert!(!headers.contains_key("authorization"));
            }
            _ => assert_eq!(headers["authorization"], "Bearer fixture-key"),
        }
    }
}

#[tokio::test]
async fn discovery_filters_text_only_inactive_and_invalid_ids_but_keeps_unknown_models() {
    let server = Server::start(vec![Reply::json(json!({"data":[
        {"id":"unknown"}, {"id":"image", "name":"Images", "architecture":{"input_modalities":["text","image"]}},
        {"id":"text-only", "input_modalities":["text"]}, {"id":"inactive", "active":false},
        {"id":"bad\nid"}, {"id":""}, {"id":"x".repeat(257)},
        {"id":"unknown"}, {"id":"native", "capabilities":{"image_input":{"supported":true}}}
    ]}))]).await;
    let result = discover_models(server.settings(LlmProvider::OpenRouter))
        .await
        .unwrap();
    assert_eq!(
        result
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["image", "native", "unknown"]
    );
    assert_eq!(result.models[0].vision, Some(true));
    assert_eq!(result.models[0].name, "Images");
    assert_eq!(result.models[2].vision, None);
    assert!(!result.truncated);
}

#[tokio::test]
async fn native_catalogs_follow_encoded_pagination_and_filter_embedding_models() {
    for provider in [LlmProvider::Anthropic, LlmProvider::Gemini] {
        let pages = match provider {
            LlmProvider::Anthropic => vec![
                json!({"data":[{"id":"first"}], "has_more":true, "last_id":"page &/=2"}),
                json!({"data":[{"id":"second"}], "has_more":false}),
            ],
            _ => vec![
                json!({"models":[{"name":"models/first", "supportedGenerationMethods":["generateContent"]}, {"name":"models/embedding", "supportedGenerationMethods":["embedContent"]}], "nextPageToken":"page &/=2"}),
                json!({"models":[{"name":"models/second", "supportedGenerationMethods":["generateContent"]}]}),
            ],
        };
        let server = Server::start(pages.into_iter().map(Reply::json).collect()).await;
        let result = discover_models(server.settings(provider)).await.unwrap();
        assert_eq!(result.models.len(), 2);
        let requests = server.state.requests.lock().unwrap();
        let url = reqwest::Url::parse(&format!("http://localhost{}", requests[1].0)).unwrap();
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>()[0],
            (
                if provider == LlmProvider::Gemini {
                    "pageToken".into()
                } else {
                    "after_id".into()
                },
                "page &/=2".into()
            )
        );
    }
}

#[tokio::test]
async fn catalogs_are_bounded_and_errors_never_include_provider_response_bodies() {
    let data: Vec<_> = (0..550)
        .map(|n| json!({"id":format!("model-{n}")}))
        .collect();
    let server = Server::start(vec![Reply::json(json!({"data":data}))]).await;
    let result = discover_models(server.settings(LlmProvider::Custom))
        .await
        .unwrap();
    assert_eq!(result.models.len(), 500);
    assert!(result.truncated);
    for (status, body, expected) in [
        (
            StatusCode::NOT_FOUND,
            "secret-key response".into(),
            "manually",
        ),
        (
            StatusCode::UNAUTHORIZED,
            "secret-key response".into(),
            "API key",
        ),
        (
            StatusCode::OK,
            "secret-key invalid-json".into(),
            "invalid response",
        ),
        (StatusCode::OK, "x".repeat(4 * 1024 * 1024 + 1), "4 MiB"),
    ] {
        let server = Server::start(vec![Reply {
            status,
            body,
            location: None,
        }])
        .await;
        let error = discover_models(server.settings(LlmProvider::Custom))
            .await
            .unwrap_err();
        assert!(error.message.contains(expected), "{}", error.message);
        assert!(!error.message.contains("secret-key"));
    }
}

#[tokio::test]
async fn discovery_does_not_forward_credentials_on_redirects_and_requires_cloud_keys() {
    let destination = Server::start(vec![]).await;
    let server = Server::start(vec![Reply {
        status: StatusCode::FOUND,
        body: String::new(),
        location: Some(format!("{}/models", destination.url)),
    }])
    .await;
    assert!(discover_models(server.settings(LlmProvider::Custom))
        .await
        .is_err());
    assert!(destination.state.requests.lock().unwrap().is_empty());
    let mut settings = server.settings(LlmProvider::OpenAi);
    settings.api_key.clear();
    assert!(discover_models(settings)
        .await
        .unwrap_err()
        .message
        .contains("Enter an API key"));
    assert_eq!(server.state.requests.lock().unwrap().len(), 1);
}

#[test]
fn old_settings_load_safe_processing_defaults_and_saved_connections_round_trip() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("settings.json");
    std::fs::write(
        &path,
        "{\"version\":1,\"extraction\":{\"model\":\"old-model\"}}",
    )
    .unwrap();
    let mut settings = Settings::load(&path).unwrap();
    assert_eq!(settings.extraction.concurrency, 1);
    assert_eq!(settings.extraction.requests_per_minute, 0);
    assert!(settings.connections.is_empty());
    settings.connections.push(SavedConnection {
        provider: LlmProvider::Anthropic,
        base_url: "https://api.anthropic.com/v1".into(),
        model: "saved-model".into(),
        api_key: "private-fixture-key".into(),
    });
    settings.save(&path).unwrap();
    assert_eq!(Settings::load(&path).unwrap(), settings);
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("private-fixture-key"));
    assert!(!format!("{settings:?}").contains("private-fixture-key"));
    settings.connections.push(settings.connections[0].clone());
    assert!(settings.validate().is_err());
    settings.connections.clear();
    settings.extraction.concurrency = 9;
    assert!(settings.validate().is_err());
    settings.extraction.concurrency = 1;
    settings.extraction.requests_per_minute = 601;
    assert!(settings.validate().is_err());
}
