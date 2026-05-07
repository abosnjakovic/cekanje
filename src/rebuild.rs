//! Cold-start rebuild: scan `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl`
//! transcript files and pre-populate the in-memory state with sessions that
//! were active recently. The next hook event from each session attaches its
//! tmux pane.
//!
//! We never restore Waiting status here — the transcript signal for "Claude
//! is waiting on the user" is too noisy / format-dependent. Rebuilt sessions
//! always start as Working.

use crate::state::Shared;
#[cfg(test)]
use crate::state::State;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

const PROJECTS_SUBDIR: &str = ".claude/projects";

/// Rebuild state from `~/.claude/projects/`. `window` is the maximum age of a
/// session's transcript file (by mtime) for it to be restored.
pub fn rebuild(shared: &Shared, window: Duration) -> usize {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        warn!("HOME not set, skipping cold-start rebuild");
        return 0;
    };
    let projects = home.join(PROJECTS_SUBDIR);
    if !projects.is_dir() {
        debug!(path = %projects.display(), "projects dir missing, nothing to rebuild");
        return 0;
    }

    let cutoff = SystemTime::now().checked_sub(window);
    let mut restored = 0usize;

    let mut state = shared.write();
    for entry in walk_dirs(&projects) {
        let cwd = decode_project_dir(&entry);
        for transcript in transcripts_in(&entry) {
            if !is_recent(&transcript, cutoff) {
                continue;
            }
            let Some(session_id) = transcript
                .file_stem()
                .and_then(|s| s.to_str())
                .map(String::from)
            else {
                continue;
            };
            state.upsert_working(session_id, cwd.clone(), None);
            restored += 1;
        }
    }
    if restored > 0 {
        info!(restored, "cold-start rebuild populated sessions");
        state.touch();
    }
    restored
}

fn walk_dirs(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect()
}

fn transcripts_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .map(|e| e.path())
        .collect()
}

fn is_recent(path: &Path, cutoff: Option<SystemTime>) -> bool {
    let Some(cutoff) = cutoff else {
        return true; // window saturated; rebuild everything
    };
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.modified().map(|m| m >= cutoff).unwrap_or(false)
}

/// Claude Code encodes the project's absolute path into a single dirname by
/// replacing `/` with `-`. We invert naively. Paths containing literal dashes
/// won't round-trip exactly but won't crash either.
fn decode_project_dir(dir: &Path) -> Option<PathBuf> {
    let name = dir.file_name()?.to_str()?;
    if !name.starts_with('-') {
        return None;
    }
    Some(PathBuf::from(name.replace('-', "/")))
}

/// Test-only entry point that lets us hand a `State` rather than a `Shared`.
#[cfg(test)]
pub(crate) fn rebuild_into(state: &mut State, root: &Path, window: Duration) -> usize {
    let cutoff = SystemTime::now().checked_sub(window);
    let mut restored = 0usize;
    for entry in walk_dirs(root) {
        let cwd = decode_project_dir(&entry);
        for transcript in transcripts_in(&entry) {
            if !is_recent(&transcript, cutoff) {
                continue;
            }
            let Some(session_id) = transcript
                .file_stem()
                .and_then(|s| s.to_str())
                .map(String::from)
            else {
                continue;
            };
            state.upsert_working(session_id, cwd.clone(), None);
            restored += 1;
        }
    }
    restored
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }

    #[test]
    fn decode_project_dir_round_trips_typical_paths() {
        let dir = PathBuf::from("/tmp/-Users-adam-Repositories-cekanje");
        assert_eq!(
            decode_project_dir(&dir),
            Some(PathBuf::from("/Users/adam/Repositories/cekanje"))
        );
    }

    #[test]
    fn rebuild_loads_recent_transcripts_only() {
        let tmp = tempdir();
        let proj = tmp.join("-tmp-foo");
        let recent = proj.join("aaa.jsonl");
        let stale = proj.join("bbb.jsonl");
        touch(&recent);
        touch(&stale);
        // Backdate the stale file's mtime far into the past.
        let past = std::time::SystemTime::now() - Duration::from_secs(86_400);
        let f = fs::OpenOptions::new().write(true).open(&stale).unwrap();
        f.set_modified(past).unwrap();
        drop(f);

        let mut state = State::default();
        let n = rebuild_into(&mut state, &tmp, Duration::from_secs(60));
        assert_eq!(n, 1);
        assert!(state.sessions.contains_key("aaa"));
        assert!(!state.sessions.contains_key("bbb"));
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cekanje-rebuild-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
