//! Startup state recovery. Runs once before the HTTP server starts accepting
//! events. Pipeline:
//!
//! 1. Load persisted state from disk (if any).
//! 2. Validate every persisted `tmux` binding by querying the tmux server;
//!    drop bindings whose pane no longer exists.
//! 3. For sessions still without a binding, attempt heuristic recovery: if
//!    exactly one tmux pane is running `claude` in the session's cwd and that
//!    pane is unbound, attach it.
//! 4. Scan `~/.claude/projects/` transcripts for sessions we don't yet know
//!    about and add them as Working with no tmux binding.
//!
//! After this returns, `by_pane` is reconciled with the final state of
//! `sessions`.

use crate::persist;
use crate::rebuild;
use crate::state::Shared;
use crate::tmux;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, info, warn};

pub fn restore(shared: &Shared, persist_path: Option<&Path>, transcript_window: Duration) {
    let loaded = load_persisted(shared, persist_path);
    let dropped = validate_panes(shared);
    let bound = bind_unbound_via_heuristic(shared);
    let from_transcripts = if transcript_window.is_zero() {
        0
    } else {
        rebuild::rebuild(shared, transcript_window)
    };

    if loaded + dropped + bound + from_transcripts > 0 {
        info!(
            loaded,
            dropped_dead_panes = dropped,
            bound_via_heuristic = bound,
            from_transcripts,
            "state restore complete"
        );
    }
}

fn load_persisted(shared: &Shared, persist_path: Option<&Path>) -> usize {
    let Some(path) = persist_path else {
        return 0;
    };
    if !path.exists() {
        debug!(path = %path.display(), "no persisted state file");
        return 0;
    }
    match persist::load(path) {
        Ok(sessions) => {
            let count = sessions.len();
            let mut s = shared.write();
            for sess in sessions {
                s.insert_persisted(sess);
            }
            s.rebuild_pane_index();
            count
        }
        Err(e) => {
            warn!(path = %path.display(), error = %e, "could not load persisted state");
            0
        }
    }
}

fn validate_panes(shared: &Shared) -> usize {
    let mut dropped = 0;
    let mut s = shared.write();
    for sess in s.sessions.values_mut() {
        let Some(loc) = &sess.tmux else { continue };
        if !tmux::pane_alive(loc.socket.as_deref(), &loc.pane) {
            sess.tmux = None;
            dropped += 1;
        }
    }
    if dropped > 0 {
        s.rebuild_pane_index();
    }
    dropped
}

fn bind_unbound_via_heuristic(shared: &Shared) -> usize {
    // Scan tmux panes once. We only consult the default tmux server here; sessions
    // that lived on a non-default socket can't be heuristically rebound.
    let panes_by_cwd = tmux::list_claude_panes_by_cwd(None);
    if panes_by_cwd.is_empty() {
        return 0;
    }

    let mut s = shared.write();
    let mut bound_panes: std::collections::HashSet<String> = s.by_pane.keys().cloned().collect();
    let mut bound = 0;

    // Group unbound sessions by cwd so we can spot ambiguity at the cwd level.
    let mut unbound_by_cwd: HashMap<&Path, Vec<&str>> = HashMap::new();
    for sess in s.sessions.values() {
        if sess.tmux.is_some() {
            continue;
        }
        if let Some(cwd) = sess.cwd.as_deref() {
            unbound_by_cwd
                .entry(cwd)
                .or_default()
                .push(&sess.session_id);
        }
    }

    let mut bindings: Vec<(String, crate::state::TmuxLocation)> = Vec::new();
    for (cwd, sids) in &unbound_by_cwd {
        let Some(panes) = panes_by_cwd.get(*cwd) else {
            continue;
        };
        let unbound_panes: Vec<&crate::state::TmuxLocation> = panes
            .iter()
            .filter(|p| !bound_panes.contains(&p.pane))
            .collect();

        // Only act when there's exactly one session and one free pane in this cwd.
        // Anything else is ambiguous and we leave it for the next hook event.
        if sids.len() == 1 && unbound_panes.len() == 1 {
            let sid = sids[0].to_string();
            let loc = unbound_panes[0].clone();
            bound_panes.insert(loc.pane.clone());
            bindings.push((sid, loc));
        }
    }

    for (sid, loc) in bindings {
        if let Some(sess) = s.sessions.get_mut(&sid) {
            sess.tmux = Some(loc);
            bound += 1;
        }
    }
    if bound > 0 {
        s.rebuild_pane_index();
    }
    bound
}
