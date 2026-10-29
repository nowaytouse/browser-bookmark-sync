use anyhow::Result;
use axum::{
    extract::State,
    http::header,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use tracing::warn;

// ─── State ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub tx: broadcast::Sender<String>,
    pub log_lines: Arc<Mutex<Vec<String>>>,
}

impl AppState {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self {
            tx,
            log_lines: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn log(&self, msg: &str) {
        if let Ok(mut v) = self.log_lines.lock() {
            v.push(msg.to_string());
            if v.len() > 1000 {
                v.remove(0);
            }
        }
        let _ = self.tx.send(msg.to_string());
    }
}

// ─── API types ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct BrowserInfo {
    name: String,
    available: bool,
    count: usize,
    icon: String,
}

#[derive(Deserialize)]
struct ExportRequest {
    output: String,
    all_profiles: bool,
    deduplicate: bool,
    flat: bool,
    merge: bool,
}

#[derive(Deserialize)]
struct MergeRequest {
    dir: String,
    output: String,
}

#[derive(Serialize)]
struct ApiResult {
    ok: bool,
    message: String,
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

async fn index_html() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("gui/index.html"),
    )
}

async fn api_list(State(state): State<AppState>) -> Json<Vec<BrowserInfo>> {
    state.log("🔍 Detecting browsers...");
    let adapters = crate::browsers::get_all_adapters();
    let mut infos = Vec::new();
    for adapter in &adapters {
        let name = adapter.browser_type().name().to_string();
        let icon = browser_icon(&name);
        match adapter.read_bookmarks() {
            Ok(bms) => {
                let count = count_deep(&bms);
                state.log(&format!("  ✅ {name}: {count} bookmarks"));
                infos.push(BrowserInfo {
                    name,
                    available: true,
                    count,
                    icon,
                });
            }
            Err(_) => {
                infos.push(BrowserInfo {
                    name,
                    available: false,
                    count: 0,
                    icon,
                });
            }
        }
    }
    Json(infos)
}

async fn api_export(
    State(state): State<AppState>,
    Json(req): Json<ExportRequest>,
) -> Json<ApiResult> {
    let output = crate::expand_path(&req.output);
    state.log(&format!("📤 Starting export → {output}"));

    let cfg = crate::sync::ExportConfig {
        merge: req.merge,
        deduplicate: req.deduplicate,
        clean_empty: true,
        verbose: true,
        folder_filter: None,
        flat: req.flat,
        wrap_folder: Some("📁镜像文件夹".to_string()),
        no_wrap: false,
        all_profiles: req.all_profiles,
        exclude_onion: true,
    };

    match crate::sync::SyncEngine::new() {
        Ok(engine) => {
            match engine
                .export_to_html_with_extra(Some("all"), &output, &cfg, vec![])
                .await
            {
                Ok(count) => {
                    let msg = format!("✅ Exported {count} bookmarks → {output}");
                    state.log(&msg);
                    Json(ApiResult {
                        ok: true,
                        message: msg,
                    })
                }
                Err(e) => {
                    let msg = format!("❌ Export failed: {e}");
                    state.log(&msg);
                    Json(ApiResult {
                        ok: false,
                        message: msg,
                    })
                }
            }
        }
        Err(e) => {
            let msg = format!("❌ Engine init failed: {e}");
            state.log(&msg);
            Json(ApiResult {
                ok: false,
                message: msg,
            })
        }
    }
}

async fn api_merge(
    State(state): State<AppState>,
    Json(req): Json<MergeRequest>,
) -> Json<ApiResult> {
    let dir = crate::expand_path(&req.dir);
    let output = crate::expand_path(&req.output);
    state.log(&format!("📦 Merging check results from: {dir}"));
    match crate::sync::merge_check_result_files(&dir, &output) {
        Ok(count) => {
            let msg = format!("✅ Merged {count} bookmarks → {output}");
            state.log(&msg);
            Json(ApiResult {
                ok: true,
                message: msg,
            })
        }
        Err(e) => {
            let msg = format!("❌ Merge failed: {e}");
            state.log(&msg);
            Json(ApiResult {
                ok: false,
                message: msg,
            })
        }
    }
}

async fn api_logs(State(state): State<AppState>) -> Json<Vec<String>> {
    let logs = match state.log_lines.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => {
            warn!("互斥锁 state.log_lines 已中毒，正在显式恢复内部日志数据");
            poisoned.into_inner().clone()
        }
    };
    Json(logs)
}

async fn api_events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(data) => Some(Ok(Event::default().data(data))),
        Err(e) => {
            tracing::warn!("GUI SSE 事件广播通道出现消息延迟或溢出丢弃: {}", e);
            None
        }
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn count_deep(bms: &[crate::browsers::Bookmark]) -> usize {
    bms.iter()
        .map(|b| usize::from(b.url.is_some()) + count_deep(&b.children))
        .sum()
}

fn browser_icon(name: &str) -> String {
    match name {
        n if n.contains("Safari") => "🧭",
        n if n.contains("Chrome") => "🌐",
        n if n.contains("Brave Nightly") => "🦁",
        n if n.contains("Brave") => "🦁",
        n if n.contains("Waterfox") => "🦊",
        n if n.contains("Firefox") => "🔥",
        n if n.contains("LibreWolf") => "🐺",
        _ => "🌍",
    }
    .to_string()
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub async fn start_gui(port: u16) -> Result<()> {
    let state = AppState::new();
    state.log("🚀 bsync GUI server starting...");

    let app = Router::new()
        .route("/", get(index_html))
        .route("/api/list", get(api_list))
        .route("/api/export", post(api_export))
        .route("/api/merge-results", post(api_merge))
        .route("/api/logs", get(api_logs))
        .route("/api/events", get(api_events))
        .with_state(state);

    let addr = format!("127.0.0.1:{port}");
    let url = format!("http://{addr}");

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🌐 bsync GUI  →  {url}");
    println!("   Ctrl+C to stop");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Open browser automatically
    let url_clone = url.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let _ = open::that(&url_clone);
    });

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
