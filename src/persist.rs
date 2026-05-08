//! On-disk state persistence. Atomic write via tempfile + rename. JSON with a
//! schema version so we can break compatibility cleanly later.
//!
//! Path resolution:
//! - `$CEKANJE_STATE_PATH` env override (full file path)
//! - else `$HOME/.cache/cekanje/state.json`

use crate::state::{Session, State};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

const FILENAME: &str = "state.json";
const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Persisted {
    version: u32,
    sessions: Vec<Session>,
}

pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CEKANJE_STATE_PATH") {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cache/cekanje").join(FILENAME))
}

pub fn load(path: &Path) -> Result<Vec<Session>> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read state file {}", path.display()))?;
    let p: Persisted = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse state file {}", path.display()))?;
    if p.version != SCHEMA_VERSION {
        anyhow::bail!(
            "state file schema {} != expected {}",
            p.version,
            SCHEMA_VERSION
        );
    }
    Ok(p.sessions)
}

pub fn save(path: &Path, state: &State) -> Result<()> {
    let sessions: Vec<Session> = state.sessions.values().cloned().collect();
    let p = Persisted {
        version: SCHEMA_VERSION,
        sessions,
    };
    let json = serde_json::to_vec_pretty(&p)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("mkdir -p {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{State, TmuxLocation};

    fn tempfile(suffix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cekanje-persist-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            suffix,
        ))
    }

    #[test]
    fn save_then_load_roundtrips_sessions() {
        let path = tempfile("roundtrip.json");
        let mut s = State::default();
        s.upsert_working(
            "S1".into(),
            Some("/tmp/a".into()),
            Some(TmuxLocation {
                pane: "%1".into(),
                socket: None,
            }),
        );
        s.mark_waiting("S2".into(), Some("/tmp/b".into()), None, Some("hi".into()));

        save(&path, &s).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_rejects_wrong_schema_version() {
        let path = tempfile("bad-version.json");
        std::fs::write(&path, r#"{"version": 99, "sessions": []}"#).unwrap();
        assert!(load(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
