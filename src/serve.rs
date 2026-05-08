use crate::persist;
use crate::restore;
use crate::state::{self, Shared, TmuxLocation};
use crate::tmux;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
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

    let app = Router::new()
        .route("/hooks/event", post(event))
        .route("/status", get(status))
        .route("/list", get(list))
        .route("/visit", post(visit))
        .with_state(shared);

    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(addr = %addr, idle_secs, "cekanje listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn event(
    State(shared): State<Shared>,
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

    info!(event_name, %session_id, ?tmux_loc, "event");

    // Auto-clear: if a Notification or Stop fires for a pane the user is currently
    // looking at, treat as Working — no badge bump, no popup.
    let is_attention_event = matches!(event_name, "Notification" | "Stop");
    let pane_focused = tmux_loc
        .as_ref()
        .map(|t| tmux::is_pane_focused(t.socket.as_deref(), &t.pane))
        .unwrap_or(false);

    let mut should_notify = None;
    let mut state_changed = false;
    {
        let mut s = shared.write();
        s.touch();
        match event_name {
            "SessionStart" | "UserPromptSubmit" => {
                s.upsert_working(session_id, cwd, tmux_loc);
                state_changed = true;
            }
            "Notification" | "Stop" if pane_focused => {
                info!(
                    pane = ?tmux_loc.as_ref().map(|t| &t.pane),
                    "auto-cleared (user is focused on pane)"
                );
                s.upsert_working(session_id, cwd, tmux_loc);
                state_changed = true;
            }
            "Notification" | "Stop" => {
                let cwd_str = cwd.as_ref().map(|p| p.display().to_string());
                should_notify = Some((session_id.clone(), cwd_str, message.clone()));
                s.mark_waiting(session_id, cwd, tmux_loc, message);
                state_changed = true;
            }
            "SessionEnd" => {
                s.drop_session(&session_id);
                state_changed = true;
            }
            other => {
                info!(event = other, "ignoring unhandled hook event");
            }
        }
        // suppress unused warning when event isn't an attention event
        let _ = is_attention_event;
    }
    if state_changed {
        persist_now(&shared);
    }
    if let Some((sid, cwd, msg)) = should_notify {
        crate::notify::waiting(&sid, cwd.as_deref(), msg.as_deref());
    }
    StatusCode::OK
}

async fn status(State(shared): State<Shared>) -> String {
    let n = shared.read().waiting_count();
    if n == 0 {
        String::new()
    } else {
        format!("⏳{n}")
    }
}

async fn list(State(shared): State<Shared>) -> impl IntoResponse {
    let snapshot = shared.read().snapshot();
    Json(snapshot)
}

#[derive(Deserialize)]
struct VisitBody {
    pane: String,
}

async fn visit(State(shared): State<Shared>, Json(b): Json<VisitBody>) -> StatusCode {
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
