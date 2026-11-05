//! A small HTTP API so other apps can link to Rippy, search, and download.
//!
//! Every request needs `Authorization: Bearer <key>`. The key and address are in Settings.
//!
//! GET  /api/status                  name, version, and the installed yt-dlp, deno and ffmpeg
//! GET  /api/search?q=…              search results
//! POST /api/downloads               {"url": "…", "media": "audio" | "video"} starts a download
//! GET  /api/downloads/{id}          {"status": "downloading" | "done" | "failed", …}
//! GET  /api/downloads/{id}/file     the downloaded file

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rustypipe::client::RustyPipe;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::tools::{self, Tool};
use crate::youtube;
use crate::yourip::{Executor, Media};

pub const DEFAULT_ADDRESS: &str = "0.0.0.0:7479";

/// What Settings shows so people can link another app.
#[derive(Clone)]
pub struct Connection {
    pub address: Result<String, String>,
    pub key: String,
}

/// Starts the server on the Tokio runtime. A bind failure is reported, not fatal.
pub fn start(tokio: &tokio::runtime::Handle, youtube: RustyPipe, executor: Executor) -> Connection {
    let key = api_key();
    let address = std::env::var("RIPPY_API_ADDR").unwrap_or(DEFAULT_ADDRESS.to_string());
    let state = Arc::new(ApiState {
        key: key.clone(),
        youtube,
        executor,
        downloads: Mutex::new(HashMap::new()),
    });

    let listener = match std::net::TcpListener::bind(&address) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("API couldn't listen on {address}: {err}");
            return Connection {
                address: Err(format!("Couldn't listen on {address}. Is another Rippy running?")),
                key,
            };
        }
    };
    let local = listener.local_addr().ok();
    let _ = listener.set_nonblocking(true);
    let app = router(state);
    tokio.spawn(async move {
        match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => {
                if let Err(err) = axum::serve(listener, app).await {
                    eprintln!("API stopped: {err}");
                }
            }
            Err(err) => eprintln!("API couldn't start: {err}"),
        }
    });

    Connection {
        address: Ok(local.map(display_address).unwrap_or(address)),
        key,
    }
}

struct ApiState {
    key: String,
    youtube: RustyPipe,
    executor: Executor,
    downloads: Mutex<HashMap<String, Download>>,
}

#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
enum Download {
    Downloading,
    Done {
        file: String,
        #[serde(skip)]
        path: PathBuf,
    },
    Failed {
        error: String,
    },
}

fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/search", get(search))
        .route("/api/downloads", post(start_download))
        .route("/api/downloads/{id}", get(download_status))
        .route("/api/downloads/{id}/file", get(download_file))
        .layer(middleware::from_fn_with_state(state.clone(), require_key))
        .with_state(state)
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn require_key(State(state): State<Arc<ApiState>>, request: Request, next: Next) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match given {
        Some(given) if same(given.as_bytes(), state.key.as_bytes()) => next.run(request).await,
        _ => error(StatusCode::UNAUTHORIZED, "Missing or wrong API key"),
    }
}

/// Compares without stopping at the first difference, so timing doesn't reveal the key.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

async fn status() -> Json<serde_json::Value> {
    Json(json!({
        "name": "Rippy",
        "version": env!("CARGO_PKG_VERSION"),
        "tools": Tool::ALL
            .iter()
            .map(|tool| {
                let version = tool.installed_version();
                (tool.name(), version.as_deref().map(tools::display_version).map(String::from))
            })
            .collect::<HashMap<_, _>>(),
    }))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

async fn search(State(state): State<Arc<ApiState>>, Query(query): Query<SearchQuery>) -> Response {
    if query.q.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "Add a search with ?q=");
    }
    match youtube::search(&state.youtube, &query.q).await {
        Ok(hits) => Json(hits).into_response(),
        Err(err) => {
            eprintln!("API search failed: {err}");
            error(StatusCode::BAD_GATEWAY, "Couldn't search YouTube")
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum MediaParam {
    Audio,
    Video,
}

#[derive(Deserialize)]
struct DownloadRequest {
    url: String,
    media: MediaParam,
}

async fn start_download(
    State(state): State<Arc<ApiState>>,
    Json(request): Json<DownloadRequest>,
) -> Response {
    let is_web_link = reqwest::Url::parse(&request.url)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
    if !is_web_link {
        return error(StatusCode::BAD_REQUEST, "url must be an http or https link");
    }
    let media = match request.media {
        MediaParam::Audio => Media::Audio,
        MediaParam::Video => Media::Video,
    };

    let id = Uuid::new_v4().simple().to_string();
    state
        .downloads
        .lock()
        .unwrap()
        .insert(id.clone(), Download::Downloading);

    let job = state.clone();
    let job_id = id.clone();
    tokio::spawn(async move {
        let executor = job.executor.clone();
        let result =
            tokio::task::spawn_blocking(move || executor.download(media, request.url)).await;
        let download = match result {
            Ok(Ok(path)) => Download::Done {
                file: file_name(&path),
                path,
            },
            Ok(Err(err)) => Download::Failed {
                error: err.to_string(),
            },
            Err(err) => Download::Failed {
                error: err.to_string(),
            },
        };
        job.downloads.lock().unwrap().insert(job_id, download);
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({ "id": id, "status": "downloading" })),
    )
        .into_response()
}

async fn download_status(State(state): State<Arc<ApiState>>, Path(id): Path<String>) -> Response {
    match state.downloads.lock().unwrap().get(&id) {
        Some(download) => {
            let mut body = serde_json::to_value(download).unwrap_or_default();
            body["id"] = json!(id);
            Json(body).into_response()
        }
        None => error(StatusCode::NOT_FOUND, "No download with that id"),
    }
}

async fn download_file(State(state): State<Arc<ApiState>>, Path(id): Path<String>) -> Response {
    let path = match state.downloads.lock().unwrap().get(&id) {
        Some(Download::Done { path, .. }) => path.clone(),
        Some(_) => return error(StatusCode::CONFLICT, "That download isn't finished"),
        None => return error(StatusCode::NOT_FOUND, "No download with that id"),
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(_) => return error(StatusCode::GONE, "The file was moved or deleted"),
    };

    let mut response = Body::from_stream(tokio_util::io::ReaderStream::new(file)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    // `filename` for clients that only read plain ASCII names, `filename*` for the real one.
    let name = file_name(&path);
    let ascii = name
        .chars()
        .map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' })
        .collect::<String>();
    if let Ok(disposition) = HeaderValue::from_str(&format!(
        "attachment; filename=\"{ascii}\"; filename*=UTF-8''{}",
        percent_encode(&name)
    )) {
        headers.insert(header::CONTENT_DISPOSITION, disposition);
    }
    response
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The saved key, or a new one on first run. `RIPPY_API_KEY` overrides it.
fn api_key() -> String {
    if let Ok(key) = std::env::var("RIPPY_API_KEY") {
        return key;
    }
    let path = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Rippy")
        .join("api-key");
    if let Ok(key) = std::fs::read_to_string(&path) {
        if !key.trim().is_empty() {
            return key.trim().to_string();
        }
    }
    let key = Uuid::new_v4().simple().to_string();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&path, &key).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
    key
}

/// A listener on every interface is shown with this Mac's address on the local network.
fn display_address(local: SocketAddr) -> String {
    let host = if local.ip().is_unspecified() {
        lan_ip().unwrap_or_else(|| "localhost".to_string())
    } else {
        local.ip().to_string()
    };
    format!("http://{host}:{}", local.port())
}

fn lan_ip() -> Option<String> {
    // Connecting a UDP socket sends nothing; it only asks the OS which interface it would use.
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}
