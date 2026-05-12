use crate::persist;
use crate::reload;
use crate::restore;
use crate::state::{self, Shared, State, TmuxLocation};
use crate::tmux;
use axum::{
    Json, Router,
    extract::{Request, State as AxumState},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{info, warn};

static PERSIST_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Atomic write of the current state to disk if a persist path is configured.
/// Errors are logged but never propagated — a failed persist must not block
/// processing of the hook event.
fn persist_now(shared: &Shared) {
    let Some(Some(path)) = PERSIST_PATH.get() else {
        return;
    };
    let snap = shared.read();
    if let Err(e) = persist::save(path, &snap) {
        warn!(path = %path.display(), error = %e, "persist failed");
    }
}

pub async fn run(port: u16, idle_secs: u64, rebuild_window_secs: u64) -> anyhow::Result<()> {
    let shared = state::new_shared();
    let persist_path = persist::default_path();
    let _ = PERSIST_PATH.set(persist_path.clone());

    restore::restore(
        &shared,
        persist_path.as_deref(),
        Duration::from_secs(rebuild_window_secs),
    );

    let shutdown = Arc::new(Notify::new());
    if let Err(e) = init_reload(shutdown.clone()) {
        warn!(error = %e, "reload init failed; hot-reload disabled this session");
    }

    if idle_secs > 0 {
        let s = Arc::clone(&shared);
        let threshold = Duration::from_secs(idle_secs);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                if s.read().is_idle(threshold) {
                    info!(idle_secs, "idle timeout reached, exiting");
                    std::process::exit(0);
                }
            }
        });
    }

    let listener = bind_with_retry(port).await?;
    info!(addr = %listener.local_addr()?, idle_secs, "cekanje listening");

    let shutdown_signal = {
        let n = shutdown.clone();
        async move {
            n.notified().await;
        }
    };
    let serve_fut = axum::serve(listener, router(shared)).with_graceful_shutdown(shutdown_signal);

    // Drain ceiling: once shutdown is signalled, give in-flight handlers up
    // to 3 seconds to finish. If they don't, drop the serve future to force
    // termination so the swap (or normal exit) can proceed.
    let drain_deadline = {
        let n = shutdown.clone();
        async move {
            n.notified().await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    };

    tokio::select! {
        res = serve_fut => { res?; }
        _ = drain_deadline => {
            warn!("graceful drain exceeded 3s; forcing shutdown");
        }
    }

    if reload::was_swap_requested() {
        // Returns Infallible on success — control transfers into the new image.
        // If we reach here with an Err, fall through and exit with that error.
        reload::replace_process_image()?;
    }
    Ok(())
}

fn init_reload(shutdown: Arc<Notify>) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let argv_tail: Vec<String> = std::env::args().skip(1).collect();
    reload::init(exe, argv_tail, shutdown)
}

/// Bind 127.0.0.1:port with a brief retry loop. Covers the race between an
/// outgoing process image releasing the port and a fresh image rebinding it
/// after `execve(2)`.
async fn bind_with_retry(port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    let addr = format!("127.0.0.1:{port}");
    let mut last_err: Option<std::io::Error> = None;
    for attempt in 0..10 {
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => {
                if attempt > 0 {
                    info!(attempt, "bound after retry");
                }
                return Ok(l);
            }
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Err(last_err
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("bind failed")))
}

pub(crate) fn router(shared: Shared) -> Router {
    Router::new()
        .route("/hooks/event", post(event))
        .route_layer(middleware::from_fn(reload_check))
        .route("/status", get(status))
        .route("/list", get(list))
        .route("/visit", post(visit))
        .route("/admin/version", get(version))
        .with_state(shared)
}

/// Post-handler middleware on `/hooks/event`. Cheap fingerprint check; if
/// the binary has changed since startup, signals the server to drain and
/// re-image. Runs after the handler returns so the response path stays fast.
async fn reload_check(req: Request, next: Next) -> Response {
    let resp = next.run(req).await;
    reload::request_swap_if_stale();
    resp
}

async fn version() -> Json<Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "startup_fp": reload::ctx().map(|c| c.startup_fp),
        "current_fp": reload::current_fp(),
        "swap_in_flight": reload::was_swap_requested(),
    }))
}

/// Outcome of applying a hook event to in-memory state. The handler uses
/// `state_changed` to decide whether to persist, and `notify` to decide
/// whether to send a desktop notification.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct EventOutcome {
    pub state_changed: bool,
    pub notify: Option<NotifyPayload>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NotifyPayload {
    pub session_id: String,
    pub cwd: Option<String>,
    pub message: Option<String>,
}

/// Apply a single hook event to state. Pure — no I/O, no logging side effects.
pub(crate) fn apply_event(
    s: &mut State,
    event_name: &str,
    session_id: String,
    cwd: Option<PathBuf>,
    tmux_loc: Option<TmuxLocation>,
    message: Option<String>,
    pane_focused: bool,
) -> EventOutcome {
    s.touch();
    match event_name {
        "SessionStart" | "UserPromptSubmit" => {
            s.upsert_working(session_id, cwd, tmux_loc);
            EventOutcome {
                state_changed: true,
                notify: None,
            }
        }
        "Notification" | "Stop" if pane_focused => {
            s.upsert_working(session_id, cwd, tmux_loc);
            EventOutcome {
                state_changed: true,
                notify: None,
            }
        }
        "Notification" | "Stop" => {
            let cwd_str = cwd.as_ref().map(|p| p.display().to_string());
            let payload = NotifyPayload {
                session_id: session_id.clone(),
                cwd: cwd_str,
                message: message.clone(),
            };
            s.mark_waiting(session_id, cwd, tmux_loc, message);
            EventOutcome {
                state_changed: true,
                notify: Some(payload),
            }
        }
        "SessionEnd" => {
            s.drop_session(&session_id);
            EventOutcome {
                state_changed: true,
                notify: None,
            }
        }
        _ => EventOutcome::default(),
    }
}

async fn event(
    AxumState(shared): AxumState<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> StatusCode {
    let event_name = body
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Some(session_id) = body
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(String::from)
    else {
        warn!(event_name, "event missing session_id");
        return StatusCode::BAD_REQUEST;
    };

    let cwd = body.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from);
    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .map(String::from);

    let pane = header_value(&headers, "x-tmux-pane");
    let socket = header_value(&headers, "x-tmux-socket").map(|s| tmux::parse_socket(&s));
    let tmux_loc = pane.map(|pane| TmuxLocation {
        pane,
        socket: socket.clone(),
    });

    let pane_focused = tmux_loc
        .as_ref()
        .map(|t| tmux::is_pane_focused(t.socket.as_deref(), &t.pane))
        .unwrap_or(false);

    info!(event_name, %session_id, ?tmux_loc, pane_focused, "event");

    let outcome = {
        let mut s = shared.write();
        apply_event(
            &mut s,
            event_name,
            session_id,
            cwd,
            tmux_loc,
            message,
            pane_focused,
        )
    };
    if outcome.state_changed {
        persist_now(&shared);
    }
    if let Some(n) = outcome.notify {
        crate::notify::waiting(&n.session_id, n.cwd.as_deref(), n.message.as_deref());
    }
    StatusCode::OK
}

async fn status(AxumState(shared): AxumState<Shared>) -> String {
    let n = shared.read().waiting_count();
    if n == 0 {
        String::new()
    } else {
        format!("⏳{n}")
    }
}

async fn list(AxumState(shared): AxumState<Shared>) -> impl IntoResponse {
    let dropped = shared.write().prune_dead_panes();
    let snapshot = shared.read().snapshot();
    if dropped > 0 {
        persist_now(&shared);
    }
    Json(snapshot)
}

#[derive(Deserialize)]
struct VisitBody {
    pane: String,
}

async fn visit(AxumState(shared): AxumState<Shared>, Json(b): Json<VisitBody>) -> StatusCode {
    let cleared = {
        let mut s = shared.write();
        s.touch();
        s.visit_pane(&b.pane)
    };
    if cleared {
        info!(pane = %b.pane, "cleared");
        persist_now(&shared);
    }
    StatusCode::OK
}

fn header_value(h: &HeaderMap, name: &str) -> Option<String> {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client;

    fn pane(p: &str) -> Option<TmuxLocation> {
        Some(TmuxLocation {
            pane: p.into(),
            socket: None,
        })
    }

    // ── apply_event (pure) ──────────────────────────────────────────────

    #[test]
    fn apply_event_session_start_upserts_working() {
        let mut s = State::default();
        let out = apply_event(
            &mut s,
            "SessionStart",
            "S1".into(),
            Some("/tmp/a".into()),
            pane("%1"),
            None,
            false,
        );
        assert!(out.state_changed);
        assert!(out.notify.is_none());
        assert_eq!(s.sessions["S1"].status, crate::state::Status::Working);
        assert_eq!(s.by_pane["%1"], "S1");
    }

    #[test]
    fn apply_event_user_prompt_submit_upserts_working() {
        let mut s = State::default();
        let out = apply_event(
            &mut s,
            "UserPromptSubmit",
            "S1".into(),
            None,
            pane("%1"),
            None,
            false,
        );
        assert!(out.state_changed);
        assert!(out.notify.is_none());
        assert_eq!(s.sessions["S1"].status, crate::state::Status::Working);
    }

    #[test]
    fn apply_event_notification_when_focused_does_not_notify() {
        let mut s = State::default();
        let out = apply_event(
            &mut s,
            "Notification",
            "S1".into(),
            Some("/tmp/a".into()),
            pane("%1"),
            Some("hi".into()),
            true,
        );
        assert!(out.state_changed);
        assert!(out.notify.is_none());
        assert_eq!(s.sessions["S1"].status, crate::state::Status::Working);
    }

    #[test]
    fn apply_event_notification_unfocused_marks_waiting_and_notifies() {
        let mut s = State::default();
        let out = apply_event(
            &mut s,
            "Notification",
            "S1".into(),
            Some("/tmp/a".into()),
            pane("%1"),
            Some("permission?".into()),
            false,
        );
        assert!(out.state_changed);
        let n = out.notify.expect("notify payload");
        assert_eq!(n.session_id, "S1");
        assert_eq!(n.cwd.as_deref(), Some("/tmp/a"));
        assert_eq!(n.message.as_deref(), Some("permission?"));
        assert_eq!(s.sessions["S1"].status, crate::state::Status::Waiting);
    }

    #[test]
    fn apply_event_stop_unfocused_mirrors_notification() {
        let mut s = State::default();
        let out = apply_event(&mut s, "Stop", "S1".into(), None, pane("%1"), None, false);
        assert!(out.notify.is_some());
        assert_eq!(s.sessions["S1"].status, crate::state::Status::Waiting);
    }

    #[test]
    fn apply_event_session_end_drops_session() {
        let mut s = State::default();
        s.upsert_working("S1".into(), None, pane("%1"));
        let out = apply_event(&mut s, "SessionEnd", "S1".into(), None, None, None, false);
        assert!(out.state_changed);
        assert!(out.notify.is_none());
        assert!(s.sessions.is_empty());
        assert!(s.by_pane.is_empty());
    }

    #[test]
    fn apply_event_unknown_event_is_noop() {
        let mut s = State::default();
        let out = apply_event(
            &mut s,
            "PreToolUse",
            "S1".into(),
            None,
            pane("%1"),
            None,
            false,
        );
        assert!(!out.state_changed);
        assert!(out.notify.is_none());
        assert!(s.sessions.is_empty());
    }

    // ── HTTP integration via real TCP listener ──────────────────────────

    async fn spawn() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = state::new_shared();
        let app = router(shared);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        port
    }

    #[tokio::test]
    async fn status_endpoint_empty_when_no_waiting() {
        let port = spawn().await;
        let body = client::http_get(port, "/status").await.unwrap();
        assert_eq!(body, "");
    }

    #[tokio::test]
    async fn status_endpoint_returns_badge_when_waiting() {
        let port = spawn().await;
        let evt = serde_json::json!({
            "hook_event_name": "Notification",
            "session_id": "S1",
            "cwd": "/tmp/a",
            "message": "permission?",
        })
        .to_string();
        client::http_post_json(port, "/hooks/event", &evt)
            .await
            .unwrap();
        let body = client::http_get(port, "/status").await.unwrap();
        assert_eq!(body, "⏳1");
    }

    #[tokio::test]
    async fn list_endpoint_returns_json_snapshot() {
        let port = spawn().await;
        let evt = serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "S1",
            "cwd": "/tmp/a",
        })
        .to_string();
        client::http_post_json(port, "/hooks/event", &evt)
            .await
            .unwrap();
        let body = client::http_get(port, "/list").await.unwrap();
        let arr: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["session_id"], "S1");
        assert_eq!(arr[0]["status"], "working");
    }

    #[tokio::test]
    async fn visit_endpoint_clears_waiting_for_pane() {
        let port = spawn().await;
        // Seed a waiting session bound to pane %42.
        // (We can't easily set headers via our minimalist client, so we use
        // the SessionStart path to register pane via headers — instead we
        // just register without a pane and visit with no pane; we want to
        // test the round-trip, so send a Notification with a header.)
        // Simpler: use a raw TCP request with the x-tmux-pane header.
        let evt_body = serde_json::json!({
            "hook_event_name": "Notification",
            "session_id": "S1",
            "cwd": "/tmp/a",
        })
        .to_string();
        send_with_pane_header(port, "/hooks/event", &evt_body, "%42")
            .await
            .unwrap();
        // Confirm waiting.
        assert_eq!(client::http_get(port, "/status").await.unwrap(), "⏳1");

        // Visit clears.
        let visit = serde_json::json!({ "pane": "%42" }).to_string();
        client::http_post_json(port, "/visit", &visit)
            .await
            .unwrap();
        assert_eq!(client::http_get(port, "/status").await.unwrap(), "");
    }

    #[tokio::test]
    async fn event_endpoint_400_when_session_id_missing() {
        let port = spawn().await;
        let evt = serde_json::json!({
            "hook_event_name": "SessionStart",
            "cwd": "/tmp/a",
        })
        .to_string();
        let raw = raw_post(port, "/hooks/event", &evt, &[]).await.unwrap();
        assert!(
            raw.starts_with("HTTP/1.1 400"),
            "expected 400, got: {}",
            raw.lines().next().unwrap_or("")
        );
    }

    /// Minimal raw POST helper that lets us attach extra headers (the
    /// production `client::http_post_json` doesn't).
    async fn raw_post(
        port: u16,
        path: &str,
        body: &str,
        extra_headers: &[(&str, &str)],
    ) -> std::io::Result<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        let mut req = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (k, v) in extra_headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        req.push_str(body);
        stream.write_all(req.as_bytes()).await?;
        let mut buf = String::new();
        stream.read_to_string(&mut buf).await?;
        Ok(buf)
    }

    async fn send_with_pane_header(
        port: u16,
        path: &str,
        body: &str,
        pane: &str,
    ) -> std::io::Result<String> {
        raw_post(port, path, body, &[("x-tmux-pane", pane)]).await
    }
}
