//! One-shot vision transcription with explicit provider routing and validated completions.

use std::time::Duration;

mod connection;
pub use connection::{discover_models, ModelCatalog, VisionModel};

use base64::{engine::general_purpose::STANDARD, Engine};
use genai::{
    adapter::AdapterKind,
    chat::{ChatMessage, ChatOptions, ChatRequest, ContentPart},
    resolver::{AuthData, Endpoint},
    Client, Headers, ModelIden, ServiceTarget,
};

use crate::{
    settings::{ExtractionSettings, LlmProvider},
    trailer::hash_bytes,
    PreparedImage,
};

pub use crate::ports::vision::ExtractionError;
use crate::ports::vision::{ExtractionFuture, VisionFactory, VisionProvider};

pub struct VisionClient {
    client: Client,
    target: ServiceTarget,
    settings: ExtractionSettings,
    processor: String,
}

impl VisionClient {
    pub fn new(settings: ExtractionSettings) -> Result<Self, ExtractionError> {
        settings
            .validate()
            .map_err(|error| ExtractionError::permanent(error.to_string()))?;
        let endpoint = settings
            .normalized_base_url()
            .map_err(|error| ExtractionError::permanent(error.to_string()))?;
        let mut header_values = vec![];
        if settings.provider == LlmProvider::Anthropic {
            header_values.push(("anthropic-version".to_owned(), "2023-06-01".to_owned()));
        }
        if !settings.api_key.trim().is_empty() {
            let (name, header) = match settings.provider {
                LlmProvider::Anthropic => ("x-api-key", settings.api_key.trim().to_owned()),
                LlmProvider::Gemini => ("x-goog-api-key", settings.api_key.trim().to_owned()),
                _ => (
                    "Authorization",
                    format!("Bearer {}", settings.api_key.trim()),
                ),
            };
            reqwest::header::HeaderValue::from_str(&header).map_err(|_| {
                ExtractionError::permanent("API key contains invalid header characters")
            })?;
            header_values.push((name.to_owned(), header));
        }
        let transport = reqwest::Client::builder()
            .timeout(Duration::from_secs(settings.timeout_seconds))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ExtractionError::permanent("Cannot initialize the LLM HTTP client"))?;
        // A direct target avoids model-name inference and accidental routing to another provider.
        // RequestOverride also supports endpoints that genuinely require no Authorization header.
        let model = if settings.provider == LlmProvider::Gemini {
            settings
                .model
                .trim()
                .strip_prefix("models/")
                .unwrap_or(settings.model.trim())
        } else {
            settings.model.trim()
        };
        let (adapter, route, processor_name) = match settings.provider {
            LlmProvider::Custom => (
                AdapterKind::OpenAI,
                "chat/completions".into(),
                "openai-compatible",
            ),
            LlmProvider::OpenAi => (AdapterKind::OpenAI, "chat/completions".into(), "openai"),
            LlmProvider::OpenRouter => (
                AdapterKind::OpenRouter,
                "chat/completions".into(),
                "openrouter",
            ),
            LlmProvider::Groq => (AdapterKind::Groq, "chat/completions".into(), "groq"),
            LlmProvider::Xai => (AdapterKind::Xai, "chat/completions".into(), "xai"),
            LlmProvider::Anthropic => (AdapterKind::Anthropic, "messages".into(), "anthropic"),
            LlmProvider::Gemini => (
                AdapterKind::Gemini,
                format!("models/{model}:generateContent"),
                "gemini",
            ),
            LlmProvider::Ollama => (AdapterKind::Ollama, "api/chat".into(), "ollama"),
        };
        let target = ServiceTarget {
            endpoint: Endpoint::from_owned(endpoint.clone()),
            model: ModelIden::new(adapter, model),
            auth: AuthData::RequestOverride {
                url: format!("{endpoint}{route}"),
                headers: Headers::from(header_values),
            },
        };
        let mut identity = serde_json::json!({
            "format": "lenscribe-transcription-v1", "endpoint": endpoint,
            "model": settings.model.trim(), "prompt": settings.prompt, "maxTokens": settings.max_tokens,
        });
        // Preserve existing custom-endpoint cache keys when old settings gain the
        // default provider field. Other providers have separate cache identities.
        if settings.provider != LlmProvider::Custom {
            identity["provider"] = serde_json::json!(settings.provider);
        }
        let processor = format!(
            "{processor_name}/{}/transcription-v1/{}",
            settings.model.trim(),
            hash_bytes(identity.to_string().as_bytes())
        );
        Ok(Self {
            client: Client::builder().with_reqwest(transport).build(),
            target,
            settings,
            processor,
        })
    }

    pub fn processor(&self) -> &str {
        &self.processor
    }

    pub async fn extract(&self, image: &PreparedImage) -> Result<String, ExtractionError> {
        if image.bytes.len() > 20 * 1024 * 1024 {
            return Err(ExtractionError::permanent(
                "Image exceeds the 20 MiB extraction limit",
            ));
        }
        let request = ChatRequest::new(vec![
            ChatMessage::system(self.settings.prompt.clone()),
            ChatMessage::user(vec![
                ContentPart::from_text("Transcribe this image."),
                ContentPart::from_binary_base64(
                    &image.mime_type,
                    STANDARD.encode(&image.bytes),
                    None,
                ),
            ]),
        ]);
        let options = ChatOptions::default().with_capture_raw_body(true);
        // genai 0.6 handles GPT-5/o-series token naming; extend that mapping for GPT-6.
        let options = if matches!(
            self.settings.provider,
            LlmProvider::Custom | LlmProvider::OpenAi
        ) && self.settings.model.trim().starts_with("gpt-6")
        {
            options.with_extra_body(
                serde_json::json!({"max_completion_tokens": self.settings.max_tokens}),
            )
        } else {
            options.with_max_tokens(self.settings.max_tokens)
        };
        let response = self
            .client
            .exec_chat(self.target.clone(), request, Some(&options))
            .await
            .map_err(safe_error)?;
        let body = response.captured_raw_body.ok_or_else(|| {
            ExtractionError::permanent("Endpoint did not return a readable response")
        })?;
        let text = match self.settings.provider {
            LlmProvider::Anthropic => anthropic_text(&body),
            LlmProvider::Gemini => gemini_text(&body),
            LlmProvider::Ollama => ollama_text(&body),
            _ => openai_text(&body),
        }?;
        if text.len() > 16 * 1024 * 1024 {
            return Err(ExtractionError::permanent(
                "Transcription exceeds the 16 MiB text limit",
            ));
        }
        Ok(text)
    }
}

fn openai_text(body: &serde_json::Value) -> Result<String, ExtractionError> {
    let choice = body
        .pointer("/choices/0")
        .ok_or_else(|| ExtractionError::permanent("Endpoint returned no completion choices"))?;
    match choice
        .get("finish_reason")
        .and_then(serde_json::Value::as_str)
    {
        Some("stop") => (),
        Some("length") => {
            return Err(ExtractionError::permanent(
                "Transcription was truncated. Increase the output token limit and retry",
            ))
        }
        Some("content_filter") => {
            return Err(ExtractionError::permanent(
                "Endpoint filtered this image; no text was saved",
            ))
        }
        _ => {
            return Err(ExtractionError::permanent(
                "Endpoint did not report a completed text response",
            ))
        }
    }
    if choice
        .pointer("/message/refusal")
        .is_some_and(|refusal| !refusal.is_null() && refusal.as_str() != Some(""))
    {
        return Err(ExtractionError::permanent(
            "Model refused transcription; no text was saved",
        ));
    }
    // Read the original content to preserve whitespace and distinguish valid empty text from
    // missing content, which some SDKs normalize to the same result.
    let text = choice
        .pointer("/message/content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ExtractionError::permanent("Endpoint returned missing or non-text content")
        })?;
    Ok(text.to_owned())
}

fn incomplete() -> ExtractionError {
    ExtractionError::permanent("Endpoint did not report a completed text response")
}

fn truncated() -> ExtractionError {
    ExtractionError::permanent(
        "Transcription was truncated. Increase the output token limit and retry",
    )
}

fn invalid_content() -> ExtractionError {
    ExtractionError::permanent("Endpoint returned missing or non-text content")
}

fn anthropic_text(body: &serde_json::Value) -> Result<String, ExtractionError> {
    match body.get("stop_reason").and_then(serde_json::Value::as_str) {
        Some("end_turn") => (),
        Some("max_tokens") => return Err(truncated()),
        Some("refusal") => {
            return Err(ExtractionError::permanent(
                "Model refused transcription; no text was saved",
            ))
        }
        _ => return Err(incomplete()),
    }
    let parts = body
        .get("content")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(invalid_content)?;
    let mut text = String::new();
    let mut has_text = false;
    for part in parts {
        match part.get("type").and_then(serde_json::Value::as_str) {
            Some("text") => {
                text.push_str(
                    part.get("text")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(invalid_content)?,
                );
                has_text = true;
            }
            Some("thinking" | "redacted_thinking") => (),
            _ => return Err(invalid_content()),
        }
    }
    if !has_text {
        return Err(invalid_content());
    }
    Ok(text)
}

fn gemini_text(body: &serde_json::Value) -> Result<String, ExtractionError> {
    if body
        .pointer("/promptFeedback/blockReason")
        .is_some_and(|reason| !reason.is_null())
    {
        return Err(ExtractionError::permanent(
            "Endpoint filtered this image; no text was saved",
        ));
    }
    let candidate = body.pointer("/candidates/0").ok_or_else(incomplete)?;
    match candidate
        .get("finishReason")
        .and_then(serde_json::Value::as_str)
    {
        Some("STOP") => (),
        Some("MAX_TOKENS") => return Err(truncated()),
        Some(
            "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY",
        ) => {
            return Err(ExtractionError::permanent(
                "Endpoint filtered this image; no text was saved",
            ))
        }
        _ => return Err(incomplete()),
    }
    let parts = candidate
        .pointer("/content/parts")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(invalid_content)?;
    let mut text = String::new();
    let mut has_text = false;
    for part in parts {
        if part.get("thought").and_then(serde_json::Value::as_bool) == Some(true) {
            continue;
        }
        // A function call or generated media is not a transcription. Thought
        // signatures alongside normal text are metadata, not text to save.
        if part.get("functionCall").is_some() || part.get("inlineData").is_some() {
            return Err(invalid_content());
        }
        text.push_str(
            part.get("text")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(invalid_content)?,
        );
        has_text = true;
    }
    if !has_text {
        return Err(invalid_content());
    }
    Ok(text)
}

fn ollama_text(body: &serde_json::Value) -> Result<String, ExtractionError> {
    if body.get("done").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(incomplete());
    }
    match body.get("done_reason").and_then(serde_json::Value::as_str) {
        Some("stop") => (),
        Some("length") => return Err(truncated()),
        _ => return Err(incomplete()),
    }
    if body
        .pointer("/message/tool_calls")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|calls| !calls.is_empty())
    {
        return Err(invalid_content());
    }
    body.pointer("/message/content")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(invalid_content)
}

// Never display the SDK error directly: it can contain the complete image/request, response
// body, or endpoint-supplied secrets. Keep status and transport diagnostics only.
fn safe_error(error: genai::Error) -> ExtractionError {
    let web = match error {
        genai::Error::WebModelCall { webc_error, .. }
        | genai::Error::WebAdapterCall { webc_error, .. } => webc_error,
        _ => {
            return ExtractionError::permanent(
                "Endpoint returned an incompatible completion response",
            )
        }
    };
    match web {
        genai::webc::Error::ResponseFailedStatus {
            status, headers, ..
        } => {
            let retryable =
                status.as_u16() == 429 || status.as_u16() == 408 || status.is_server_error();
            let message = match status.as_u16() {
                401 | 403 => "Endpoint rejected the API key or access permission".to_owned(),
                404 => "Endpoint or model was not found; check the base URL and model".to_owned(),
                400 | 422 => "Endpoint rejected the vision request; check model image support and output token limit".to_owned(),
                _ => format!("LLM endpoint returned HTTP {}", status.as_u16()),
            };
            ExtractionError {
                message,
                retryable,
                blocks_queue: matches!(status.as_u16(), 401 | 403 | 404),
                retry_after_seconds: headers
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .map(|seconds: u64| seconds.min(900)),
            }
        }
        genai::webc::Error::Reqwest(error) if error.is_timeout() => {
            ExtractionError::temporary("LLM request timed out")
        }
        genai::webc::Error::Reqwest(_) => {
            ExtractionError::temporary("Cannot reach the LLM endpoint")
        }
        _ => ExtractionError::permanent("Endpoint returned invalid JSON or a non-JSON response"),
    }
}

pub struct GenaiVisionFactory;
impl VisionFactory for GenaiVisionFactory {
    fn create(
        &self,
        settings: ExtractionSettings,
    ) -> Result<std::sync::Arc<dyn VisionProvider>, ExtractionError> {
        Ok(std::sync::Arc::new(VisionClient::new(settings)?))
    }
}
impl VisionProvider for VisionClient {
    fn processor(&self) -> &str {
        VisionClient::processor(self)
    }
    fn extract<'a>(&'a self, image: &'a PreparedImage) -> ExtractionFuture<'a> {
        Box::pin(VisionClient::extract(self, image))
    }
}
