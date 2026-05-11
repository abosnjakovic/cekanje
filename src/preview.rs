//! Render an fzf preview block for a given session_id. Invoked by
//! `cek preview <sid>`, which is what `cek menu`'s `--preview` flag points
//! at. All output goes to stdout. Failures degrade gracefully — a missing
//! transcript or dead pane just elides that section.

use crate::client;
use crate::state::{Session, Status};
use crate::tmux;
use crate::transcript;
use anyhow::{Context, Result};
use std::fmt::Write;
use std::path::{Path, PathBuf};

const ASSISTANT_MAX_LINES: usize = 30;
const PANE_SCROLLBACK_LINES: u32 = 100;
const TRANSCRIPT_TAIL_BYTES: u64 = 65_536;

const RULE_PREFIX: &str = "\x1b[1;36m─── ";
const RULE_SUFFIX: &str = " \x1b[0m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

pub async fn run(port: u16, sid: &str) -> Result<()> {
    let body = client::http_get(port, "/list")
        .await
        .context("fetch /list")?;
    let sessions: Vec<Session> = serde_json::from_str(&body).unwrap_or_default();
    let Some(session) = sessions.into_iter().find(|s| s.session_id == sid) else {
        println!("(session {sid} not found)");
        return Ok(());
    };

    print!("{}", metadata_block(&session));
    if let Some(home) = home_dir()
        && let Some(block) = transcript_block(&home, &session)
    {
        print!("{block}");
    }
    print_pane_capture(&session);
    Ok(())
}

fn rule_line(label: &str) -> String {
    format!("{RULE_PREFIX}{label}{RULE_SUFFIX}\n")
}

fn metadata_block(s: &Session) -> String {
    let mut out = String::new();
    out.push_str(&rule_line("session"));
    let _ = writeln!(out, "sid:     {DIM}{}{RESET}", s.session_id);
    if let Some(cwd) = &s.cwd {
        let _ = writeln!(out, "cwd:     {}", cwd.display());
    }
    if let Some(t) = &s.tmux {
        match &t.socket {
            Some(sock) => {
                let _ = writeln!(out, "pane:    {}  {DIM}socket: {sock}{RESET}", t.pane);
            }
            None => {
                let _ = writeln!(out, "pane:    {}", t.pane);
            }
        }
    }
    let status = match s.status {
        Status::Waiting => match s.waiting_since_secs {
            Some(n) => format!("\x1b[33mwaiting\x1b[0m ({n}s)"),
            None => "\x1b[33mwaiting\x1b[0m".to_string(),
        },
        Status::Working => "working".to_string(),
    };
    let _ = writeln!(out, "status:  {status}");
    let _ = writeln!(out, "age:     {}s", s.age_secs);
    if let Some(msg) = &s.last_message {
        let _ = writeln!(out, "message: {msg}");
    }
    out.push('\n');
    out
}

fn transcript_block(home: &Path, s: &Session) -> Option<String> {
    let cwd = s.cwd.as_deref()?;
    let path = transcript::transcript_path(home, cwd, &s.session_id);
    if !path.exists() {
        return None;
    }
    let ex = transcript::last_excerpt(&path, TRANSCRIPT_TAIL_BYTES).ok()?;

    let mut out = String::new();
    if let Some(text) = ex.last_user {
        out.push_str(&rule_line("you asked"));
        for line in text.lines() {
            let _ = writeln!(out, "{line}");
        }
        out.push('\n');
    }
    if let Some(text) = ex.last_assistant {
        out.push_str(&rule_line("claude said"));
        let lines: Vec<&str> = text.lines().collect();
        let take = lines.len().min(ASSISTANT_MAX_LINES);
        for line in &lines[..take] {
            let _ = writeln!(out, "{line}");
        }
        if lines.len() > ASSISTANT_MAX_LINES {
            let _ = writeln!(out, "{DIM}…({} more lines){RESET}", lines.len() - take);
        }
        out.push('\n');
    }
    if out.is_empty() { None } else { Some(out) }
}

fn print_pane_capture(s: &Session) {
    let Some(loc) = &s.tmux else { return };
    print!("{}", rule_line("pane"));
    match tmux::capture_pane(loc.socket.as_deref(), &loc.pane, PANE_SCROLLBACK_LINES) {
        Ok(out) => print!("{out}"),
        Err(e) => println!("{DIM}(capture failed: {e}){RESET}"),
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;

    fn session(json_overrides: serde_json::Value) -> Session {
        let mut base = serde_json::json!({
            "session_id": "abc-123",
            "cwd": "/tmp/proj",
            "tmux": { "pane": "%1", "socket": null },
            "status": "working",
            "last_message": null,
            "waiting_since_secs": null,
            "age_secs": 10u64,
        });
        let serde_json::Value::Object(ref mut map) = base else {
            unreachable!()
        };
        if let serde_json::Value::Object(over) = json_overrides {
            for (k, v) in over {
                map.insert(k, v);
            }
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn metadata_block_working_session_includes_core_fields() {
        let s = session(serde_json::json!({}));
        let out = metadata_block(&s);
        assert!(out.contains("session"));
        assert!(out.contains("sid:     "));
        assert!(out.contains("abc-123"));
        assert!(out.contains("cwd:     /tmp/proj"));
        assert!(out.contains("pane:    %1"));
        assert!(out.contains("status:  working"));
        assert!(out.contains("age:     10s"));
    }

    #[test]
    fn metadata_block_waiting_session_uses_yellow_and_elapsed_secs() {
        let s = session(serde_json::json!({
            "status": "waiting",
            "waiting_since_secs": 42u64,
        }));
        let out = metadata_block(&s);
        assert!(out.contains("\x1b[33mwaiting\x1b[0m"));
        assert!(out.contains("(42s)"));
    }

    #[test]
    fn metadata_block_omits_cwd_tmux_message_lines_when_none() {
        let s = session(serde_json::json!({
            "cwd": null,
            "tmux": null,
            "last_message": null,
        }));
        let out = metadata_block(&s);
        assert!(!out.contains("cwd:"));
        assert!(!out.contains("pane:"));
        assert!(!out.contains("message:"));
    }

    #[test]
    fn metadata_block_includes_socket_when_set() {
        let s = session(serde_json::json!({
            "tmux": { "pane": "%5", "socket": "/tmp/sock" },
        }));
        let out = metadata_block(&s);
        assert!(out.contains("pane:    %5"));
        assert!(out.contains("socket: /tmp/sock"));
    }

    #[test]
    fn metadata_block_includes_message_when_present() {
        let s = session(serde_json::json!({ "last_message": "permission?" }));
        let out = metadata_block(&s);
        assert!(out.contains("message: permission?"));
    }

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cekanje-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn transcript_block_returns_none_when_no_cwd() {
        let s = session(serde_json::json!({ "cwd": null }));
        let home = tmpdir();
        assert!(transcript_block(&home, &s).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn transcript_block_returns_none_when_transcript_missing() {
        let s = session(serde_json::json!({ "cwd": "/tmp/nonexistent-proj-xyz" }));
        let home = tmpdir();
        assert!(transcript_block(&home, &s).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn transcript_block_renders_user_and_assistant_sections() {
        let home = tmpdir();
        let s = session(serde_json::json!({ "cwd": "/tmp/projx" }));
        let project_dir = home.join(".claude/projects/-tmp-projx");
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join(format!("{}.jsonl", s.session_id));
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","message":{{"content":"hi claude"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"hello!"}}]}}}}"#
        )
        .unwrap();
        drop(f);

        let block = transcript_block(&home, &s).expect("transcript block");
        assert!(block.contains("you asked"));
        assert!(block.contains("hi claude"));
        assert!(block.contains("claude said"));
        assert!(block.contains("hello!"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn transcript_block_truncates_assistant_past_max_lines() {
        let home = tmpdir();
        let s = session(serde_json::json!({ "cwd": "/tmp/projy" }));
        let project_dir = home.join(".claude/projects/-tmp-projy");
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join(format!("{}.jsonl", s.session_id));
        let big = (0..ASSISTANT_MAX_LINES + 5)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\\n");
        let line = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{big}"}}]}}}}"#,
        );
        std::fs::write(&path, line + "\n").unwrap();

        let block = transcript_block(&home, &s).expect("transcript block");
        assert!(block.contains("more lines"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn rule_line_wraps_label_with_ansi_box() {
        let r = rule_line("session");
        assert!(r.starts_with("\x1b[1;36m─── session "));
        assert!(r.ends_with("\x1b[0m\n"));
    }
}
