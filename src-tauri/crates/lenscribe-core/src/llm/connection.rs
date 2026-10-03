//! Bounded provider model discovery without exposing response bodies.
use std::time::Duration;

use reqwest::{header::HeaderMap, Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ExtractionError;
use crate::settings::{ExtractionSettings, LlmProvider};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VisionModel {
    pub id: String,
    pub name: String,
    pub vision: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub models: Vec<VisionModel>,
    pub truncated: bool,
}

pub async fn discover_models(
    mut settings: ExtractionSettings,
) -> Result<ModelCatalog, ExtractionError> {
    settings.enabled = false;
    settings.model.clear();
    settings
        .validate()
        .map_err(|e| ExtractionError::permanent(e.to_string()))?;
    if settings.provider.requires_api_key() && settings.api_key.trim().is_empty() {
        return Err(ExtractionError::permanent(
            "Enter an API key before fetching models",
        ));
    }
    let base = settings
        .normalized_base_url()
        .map_err(|e| ExtractionError::permanent(e.to_string()))?;
    let mut headers = HeaderMap::new();
    if settings.provider == LlmProvider::Anthropic {
        headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
    }
    if !settings.api_key.trim().is_empty() {
        let (name, value) = match settings.provider {
            LlmProvider::Anthropic => ("x-api-key", settings.api_key.trim().to_owned()),
            LlmProvider::Gemini => ("x-goog-api-key", settings.api_key.trim().to_owned()),
            _ => (
                "authorization",
                format!("Bearer {}", settings.api_key.trim()),
            ),
        };
        headers.insert(
            name,
            value.parse().map_err(|_| {
                ExtractionError::permanent("API key contains invalid header characters")
            })?,
        );
    }
    let client = Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ExtractionError::permanent("Cannot initialize model discovery"))?;
    let route = if settings.provider == LlmProvider::Ollama {
        "api/tags"
    } else {
        "models"
    };
    let mut url = reqwest::Url::parse(&format!("{base}{route}"))
        .map_err(|_| ExtractionError::permanent("Invalid model catalog URL"))?;
    let mut models = Vec::new();
    let mut truncated = false;
    for page in 0..10 {
        let body = read_json(client.get(url.clone()).send().await.map_err(|_| {
            ExtractionError::permanent(
                "Cannot reach the model catalog. Check the connection and try again.",
            )
        })?)
        .await?;
        let key = if matches!(settings.provider, LlmProvider::Gemini | LlmProvider::Ollama) {
            "models"
        } else {
            "data"
        };
        let entries = body.get(key).and_then(Value::as_array).ok_or_else(|| ExtractionError::permanent("This endpoint does not provide a compatible model catalog. Enter the model ID manually."))?;
        for entry in entries {
            if entry.get("active").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            if settings.provider == LlmProvider::Gemini
                && !entry
                    .get("supportedGenerationMethods")
                    .and_then(Value::as_array)
                    .is_some_and(|methods| {
                        methods
                            .iter()
                            .any(|v| v.as_str() == Some("generateContent"))
                    })
            {
                continue;
            }
            let id = entry
                .get(
                    if matches!(settings.provider, LlmProvider::Gemini | LlmProvider::Ollama) {
                        "name"
                    } else {
                        "id"
                    },
                )
                .and_then(Value::as_str);
            let Some(id) = id.filter(|id| {
                !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
            }) else {
                continue;
            };
            let modalities = entry
                .pointer("/architecture/input_modalities")
                .or_else(|| entry.get("input_modalities"))
                .and_then(Value::as_array);
            let vision = modalities
                .map(|values| values.iter().any(|v| v.as_str() == Some("image")))
                .or_else(|| {
                    entry
                        .pointer("/capabilities/image_input/supported")
                        .and_then(Value::as_bool)
                });
            if vision == Some(false) {
                continue;
            }
            let name = entry
                .get("display_name")
                .or_else(|| entry.get("displayName"))
                .or_else(|| entry.get("name"))
                .and_then(Value::as_str)
                .filter(|v| v.len() <= 512 && !v.chars().any(char::is_control))
                .unwrap_or(id);
            models.push(VisionModel {
                id: id.into(),
                name: name.into(),
                vision,
            });
            if models.len() >= 500 {
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
        let next = match settings.provider {
            LlmProvider::Gemini => body
                .get("nextPageToken")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(|v| ("pageToken", v)),
            LlmProvider::Anthropic
                if body.get("has_more").and_then(Value::as_bool) == Some(true) =>
            {
                body.get("last_id")
                    .and_then(Value::as_str)
                    .map(|v| ("after_id", v))
            }
            _ => None,
        };
        let Some((name, token)) = next else {
            break;
        };
        if token.len() > 4096 {
            return Err(ExtractionError::permanent(
                "Model catalog returned an invalid pagination token",
            ));
        }
        url.set_query(None);
        url.query_pairs_mut().append_pair(name, token);
        if page == 9 {
            truncated = true;
        }
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(ModelCatalog { models, truncated })
}

async fn read_json(mut response: reqwest::Response) -> Result<Value, ExtractionError> {
    if !response.status().is_success() {
        let message = match response.status().as_u16() {
            401 | 403 => "Model catalog rejected the API key or access permission",
            404 | 405 => {
                "Model discovery is unavailable on this endpoint. Enter the model ID manually."
            }
            429 => "Model catalog is rate limited. Try again later.",
            _ => "Model catalog request failed. Check the connection and try again.",
        };
        return Err(ExtractionError::permanent(message));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ExtractionError::permanent("Could not read the model catalog"))?
    {
        if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err(ExtractionError::permanent(
                "Model catalog exceeds the 4 MiB limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| ExtractionError::permanent("Model catalog returned an invalid response"))
}
