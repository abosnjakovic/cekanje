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
use crate::state::{Shared, State, TmuxLocation};
use crate::tmux;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    bind_unbound_with_panes(&mut shared.write(), &panes_by_cwd)
}

/// Pure binding pass: given current state and a snapshot of tmux panes running
/// `claude` grouped by cwd, attach exactly one unbound session per cwd when
/// exactly one matching free pane exists. Returns the number of sessions bound.
pub(crate) fn bind_unbound_with_panes(
    s: &mut State,
    panes_by_cwd: &HashMap<PathBuf, Vec<TmuxLocation>>,
) -> usize {
    let mut bound_panes: std::collections::HashSet<String> = s.by_pane.keys().cloned().collect();
    let mut bound = 0;

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

    let mut bindings: Vec<(String, TmuxLocation)> = Vec::new();
    for (cwd, sids) in &unbound_by_cwd {
        let Some(panes) = panes_by_cwd.get(*cwd) else {
            continue;
        };
        let unbound_panes: Vec<&TmuxLocation> = panes
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist;

    fn tempfile(suffix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cekanje-restore-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            suffix,
        ))
    }

    fn pane(p: &str) -> TmuxLocation {
        TmuxLocation {
            pane: p.into(),
            socket: None,
        }
    }

    #[test]
    fn load_persisted_returns_zero_when_path_is_none() {
        let shared = crate::state::new_shared();
        assert_eq!(load_persisted(&shared, None), 0);
        assert!(shared.read().sessions.is_empty());
    }

    #[test]
    fn load_persisted_returns_zero_when_file_missing() {
        let shared = crate::state::new_shared();
        let path = tempfile("missing.json");
        assert!(!path.exists());
        assert_eq!(load_persisted(&shared, Some(&path)), 0);
    }

    #[test]
    fn load_persisted_populates_sessions_from_valid_file() {
        let shared = crate::state::new_shared();
        let path = tempfile("valid.json");
        {
            let mut seed = State::default();
            seed.upsert_working("S1".into(), Some("/tmp/a".into()), Some(pane("%1")));
            seed.upsert_working("S2".into(), Some("/tmp/b".into()), Some(pane("%2")));
            persist::save(&path, &seed).unwrap();
        }
        let n = load_persisted(&shared, Some(&path));
        assert_eq!(n, 2);
        let s = shared.read();
        assert_eq!(s.sessions.len(), 2);
        // by_pane index rebuilt
        assert_eq!(s.by_pane["%1"], "S1");
        assert_eq!(s.by_pane["%2"], "S2");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_persisted_returns_zero_on_bad_schema() {
        let shared = crate::state::new_shared();
        let path = tempfile("badschema.json");
        std::fs::write(&path, r#"{"version": 99, "sessions": []}"#).unwrap();
        assert_eq!(load_persisted(&shared, Some(&path)), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bind_unbound_attaches_unique_cwd_to_unique_pane() {
        let mut s = State::default();
        s.upsert_working("S1".into(), Some("/tmp/a".into()), None);
        let mut panes = HashMap::new();
        panes.insert(PathBuf::from("/tmp/a"), vec![pane("%9")]);

        let n = bind_unbound_with_panes(&mut s, &panes);
        assert_eq!(n, 1);
        assert_eq!(s.sessions["S1"].tmux.as_ref().unwrap().pane, "%9");
        assert_eq!(s.by_pane["%9"], "S1");
    }

    #[test]
    fn bind_unbound_skips_ambiguous_cwd_with_multiple_sessions() {
        let mut s = State::default();
        s.upsert_working("S1".into(), Some("/tmp/a".into()), None);
        s.upsert_working("S2".into(), Some("/tmp/a".into()), None);
        let mut panes = HashMap::new();
        panes.insert(PathBuf::from("/tmp/a"), vec![pane("%9")]);

        let n = bind_unbound_with_panes(&mut s, &panes);
        assert_eq!(n, 0);
        assert!(s.sessions["S1"].tmux.is_none());
        assert!(s.sessions["S2"].tmux.is_none());
    }

    #[test]
    fn bind_unbound_skips_when_no_matching_cwd_in_panes_map() {
        let mut s = State::default();
        s.upsert_working("S1".into(), Some("/tmp/a".into()), None);
        let mut panes = HashMap::new();
        panes.insert(PathBuf::from("/tmp/other"), vec![pane("%9")]);

        let n = bind_unbound_with_panes(&mut s, &panes);
        assert_eq!(n, 0);
        assert!(s.sessions["S1"].tmux.is_none());
    }

    #[test]
    fn bind_unbound_does_not_steal_pane_already_bound_to_another_session() {
        let mut s = State::default();
        // S0 already owns %9.
        s.upsert_working("S0".into(), Some("/tmp/a".into()), Some(pane("%9")));
        // S1 is unbound in the same cwd.
        s.upsert_working("S1".into(), Some("/tmp/a".into()), None);
        let mut panes = HashMap::new();
        panes.insert(PathBuf::from("/tmp/a"), vec![pane("%9")]);

        let n = bind_unbound_with_panes(&mut s, &panes);
        assert_eq!(n, 0);
        assert!(s.sessions["S1"].tmux.is_none());
    }

    #[test]
    fn bind_unbound_skips_session_without_cwd() {
        let mut s = State::default();
        s.upsert_working("S1".into(), None, None);
        let mut panes = HashMap::new();
        panes.insert(PathBuf::from("/tmp/a"), vec![pane("%9")]);

        let n = bind_unbound_with_panes(&mut s, &panes);
        assert_eq!(n, 0);
    }

    #[test]
    fn bind_unbound_returns_zero_when_panes_map_empty() {
        let mut s = State::default();
        s.upsert_working("S1".into(), Some("/tmp/a".into()), None);
        let panes = HashMap::new();
        assert_eq!(bind_unbound_with_panes(&mut s, &panes), 0);
    }
}
