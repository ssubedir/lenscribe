//! An optional read-only loopback API. No filesystem mutation or CORS access is exposed.

use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use tokio::{net::TcpListener, sync::oneshot};

use crate::{
    Core, Error, FileDetails, FolderRecord, FolderSnapshot, Result, SearchHit, SearchPage,
};

pub fn router(core: Arc<Core>) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async {
                Json(serde_json::json!({ "status": "ready", "version": env!("CARGO_PKG_VERSION") }))
            }),
        )
        .route("/folders", get(folders))
        .route("/folders/{id}", get(snapshot))
        .route("/files/{id}", get(file))
        .route("/files/{id}/text", get(file_text))
        .route("/search", get(search))
        .route("/search/page", get(search_page))
        .with_state(core)
}

pub struct ApiServer {
    address: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl ApiServer {
    /// Port zero asks the OS to allocate an available port.
    pub async fn start(core: Arc<Core>, port: u16) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
        let address = listener.local_addr()?;
        log::info!("Local API started at http://{}", address);
        let (shutdown, received) = oneshot::channel();
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router(core))
                .with_graceful_shutdown(async {
                    let _ = received.await;
                })
                .await
            {
                log::error!("Local API stopped unexpectedly: {error}");
            }
        });
        Ok(Self {
            address,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            log::info!("Local API stopping at http://{}", self.address);
            let _ = shutdown.send(());
        }
    }
}

struct ApiError(Error);

impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::InvalidInput(_) | Error::UnsupportedImage(_) | Error::InvalidTrailer(_) => {
                StatusCode::BAD_REQUEST
            }
            Error::ImageChanged => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(serde_json::json!({ "error": self.0.to_string() })),
        )
            .into_response()
    }
}

async fn query_core<T: Send + 'static>(
    core: Arc<Core>,
    operation: impl FnOnce(&Core) -> Result<T> + Send + 'static,
) -> std::result::Result<T, ApiError> {
    tokio::task::spawn_blocking(move || operation(&core))
        .await
        .map_err(|error| ApiError(Error::Io(std::io::Error::other(error.to_string()))))?
        .map_err(ApiError)
}

async fn folders(
    State(core): State<Arc<Core>>,
) -> std::result::Result<Json<Vec<FolderRecord>>, ApiError> {
    Ok(Json(query_core(core, Core::folders).await?))
}

async fn snapshot(
    State(core): State<Arc<Core>>,
    Path(id): Path<i64>,
) -> std::result::Result<Json<FolderSnapshot>, ApiError> {
    Ok(Json(query_core(core, move |core| core.snapshot(id)).await?))
}

async fn file(
    State(core): State<Arc<Core>>,
    Path(id): Path<i64>,
) -> std::result::Result<Json<FileDetails>, ApiError> {
    Ok(Json(query_core(core, move |core| core.file(id)).await?))
}

async fn file_text(
    State(core): State<Arc<Core>>,
    Path(id): Path<i64>,
) -> std::result::Result<String, ApiError> {
    query_core(core, move |core| {
        core.file(id)?
            .text
            .ok_or_else(|| Error::NotFound(format!("extracted text for file {id}")))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchQuery {
    q: String,
    folder_id: Option<i64>,
    limit: Option<usize>,
    offset: Option<usize>,
    fuzzy: Option<bool>,
}

async fn search(
    State(core): State<Arc<Core>>,
    Query(query): Query<SearchQuery>,
) -> std::result::Result<Json<Vec<SearchHit>>, ApiError> {
    Ok(Json(
        query_core(core, move |core| {
            Ok(core
                .search_page(
                    &query.q,
                    query.folder_id,
                    query.offset.unwrap_or(0),
                    query.limit.unwrap_or(20),
                    query.fuzzy.unwrap_or(true),
                )?
                .hits)
        })
        .await?,
    ))
}

async fn search_page(
    State(core): State<Arc<Core>>,
    Query(query): Query<SearchQuery>,
) -> std::result::Result<Json<SearchPage>, ApiError> {
    Ok(Json(
        query_core(core, move |core| {
            core.search_page(
                &query.q,
                query.folder_id,
                query.offset.unwrap_or(0),
                query.limit.unwrap_or(20),
                query.fuzzy.unwrap_or(true),
            )
        })
        .await?,
    ))
}
