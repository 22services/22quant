//! Local supervision dashboard (axum). Binds to 127.0.0.1 by default; every `/api` route needs
//! the session token (`Authorization: Bearer …` or `?token=` for the SSE stream).

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::runner::{Control, Shared};

const INDEX: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const STYLE: &str = include_str!("../ui/style.css");
const CHARTS: &str = include_str!("../ui/vendor/lightweight-charts.standalone.production.js");

#[derive(Deserialize)]
struct TokenQ {
    token: Option<String>,
}

fn authorized(s: &Shared, headers: &HeaderMap, q: &TokenQ) -> bool {
    let bearer = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(|v| v.to_string());
    let given = bearer.or_else(|| q.token.clone());
    given.as_deref() == Some(s.token.as_str())
}

fn deny() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": "missing or wrong token — open the URL printed in the terminal"}))).into_response()
}

async fn index() -> Html<&'static str> {
    Html(INDEX)
}

fn asset(ct: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}

async fn state(State(s): State<Arc<Shared>>, headers: HeaderMap, Query(q): Query<TokenQ>) -> Response {
    if !authorized(&s, &headers, &q) {
        return deny();
    }
    let mut v = s.snapshot.read().await.clone();
    v["meta"] = s.meta.read().await.clone();
    Json(v).into_response()
}

async fn stream(State(s): State<Arc<Shared>>, headers: HeaderMap, Query(q): Query<TokenQ>) -> Response {
    if !authorized(&s, &headers, &q) {
        return deny();
    }
    let st: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = Box::pin(futures::stream::unfold(s, |s| async move {
        tokio::time::sleep(StdDuration::from_millis(1000)).await;
        let mut v = s.snapshot.read().await.clone();
        v["meta"] = s.meta.read().await.clone();
        let ev = Event::default().data(v.to_string());
        Some((Ok(ev), s))
    }));
    Sse::new(st).keep_alive(KeepAlive::default()).into_response()
}

#[derive(Deserialize)]
struct ControlReq {
    action: String,
    id: Option<u64>,
}

async fn control(State(s): State<Arc<Shared>>, headers: HeaderMap, Query(q): Query<TokenQ>, Json(req): Json<ControlReq>) -> Response {
    if !authorized(&s, &headers, &q) {
        return deny();
    }
    let c = match (req.action.as_str(), req.id) {
        ("pause", _) => Control::Pause,
        ("resume", _) => Control::Resume,
        ("flatten", _) => Control::Flatten,
        ("kill", _) => Control::Kill,
        ("heartbeat", _) => Control::Heartbeat,
        ("approve", Some(id)) => Control::Approve(id),
        ("reject", Some(id)) => Control::Reject(id),
        _ => return (StatusCode::BAD_REQUEST, Json(json!({"error": "unknown action"}))).into_response(),
    };
    let ok = s.ctrl.send(c).is_ok();
    Json(json!({"ok": ok})).into_response()
}

async fn reports(State(s): State<Arc<Shared>>, headers: HeaderMap, Query(q): Query<TokenQ>) -> Response {
    if !authorized(&s, &headers, &q) {
        return deny();
    }
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(&s.reports_dir) {
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            if name.ends_with(".json") && !name.ends_with("__compare.json") {
                let modified = e.metadata().ok().and_then(|m| m.modified().ok()).map(chrono::DateTime::<chrono::Utc>::from);
                out.push(json!({"name": name, "modified": modified, "bytes": e.metadata().map(|m| m.len()).unwrap_or(0)}));
            }
        }
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Json(Value::Array(out)).into_response()
}

async fn report(State(s): State<Arc<Shared>>, headers: HeaderMap, Query(q): Query<TokenQ>, Path(name): Path<String>) -> Response {
    if !authorized(&s, &headers, &q) {
        return deny();
    }
    if name.contains('/') || name.contains("..") || !name.ends_with(".json") {
        return (StatusCode::BAD_REQUEST, "bad name").into_response();
    }
    match std::fs::read(s.reports_dir.join(&name)) {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/json")], bytes).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "no such report").into_response(),
    }
}

pub fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(|| async { asset("text/javascript; charset=utf-8", APP_JS) }))
        .route("/style.css", get(|| async { asset("text/css; charset=utf-8", STYLE) }))
        .route("/vendor/lightweight-charts.js", get(|| async { asset("text/javascript; charset=utf-8", CHARTS) }))
        .route("/api/state", get(state))
        .route("/api/stream", get(stream))
        .route("/api/control", post(control))
        .route("/api/reports", get(reports))
        .route("/api/report/{name}", get(report))
        .with_state(shared)
}

pub async fn serve(shared: Arc<Shared>, bind: &str) -> anyhow::Result<()> {
    if !bind.starts_with("127.") && !bind.starts_with("localhost") {
        tracing::warn!("dashboard bound to {bind}: it is reachable from other machines — keep the token secret");
    }
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("\n  dashboard → http://{}/?token={}\n", bind, shared.token);
    axum::serve(listener, router(shared)).await?;
    Ok(())
}
