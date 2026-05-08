//! Read excerpts from Claude transcript files.
//!
//! Transcripts live at `~/.claude/projects/<encoded-cwd>/<sid>.jsonl`. Each
//! line is a JSON record with `type` (`user`/`assistant`/...), `timestamp`,
//! and a `message` payload. We tail-read the file and walk backwards to find
//! the most recent user prompt and assistant text without parsing the whole
//! file.
//!
//! The encoding is asymmetric — `/Users/a` → `-Users-a` — so we don't try to
//! reverse it. We only encode forward from a known cwd, which round-trips.

use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct Excerpt {
    pub last_user: Option<String>,
    pub last_assistant: Option<String>,
}

pub fn encode_cwd(cwd: &Path) -> String {
    let s = cwd.to_string_lossy();
    s.replace('/', "-")
}

pub fn transcript_path(home: &Path, cwd: &Path, sid: &str) -> PathBuf {
    home.join(".claude/projects")
        .join(encode_cwd(cwd))
        .join(format!("{sid}.jsonl"))
}

/// Tail-read a transcript file and return the most recent user prompt and
/// assistant message. `max_bytes` caps the tail window so long sessions don't
/// blow up. Returns an empty `Excerpt` if the file is missing or unreadable.
pub fn last_excerpt(path: &Path, max_bytes: u64) -> Result<Excerpt> {
    let mut f =
        std::fs::File::open(path).with_context(|| format!("open transcript {}", path.display()))?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf)?;

    // If we started mid-line, drop the partial first line.
    let text = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = if start == 0 {
        text.lines().collect()
    } else {
        text.lines().skip(1).collect()
    };

    let mut out = Excerpt::default();
    for line in lines.iter().rev() {
        if out.last_user.is_some() && out.last_assistant.is_some() {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("user") if out.last_user.is_none() => {
                if let Some(text) = extract_user_text(&v) {
                    out.last_user = Some(text);
                }
            }
            Some("assistant") if out.last_assistant.is_none() => {
                if let Some(text) = extract_assistant_text(&v) {
                    out.last_assistant = Some(text);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// User entries can carry plain-text prompts or `tool_result` payloads.
/// We only want the prompts.
fn extract_user_text(v: &Value) -> Option<String> {
    let content = v.get("message")?.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    let parts = content.as_array()?;
    let mut out = String::new();
    for p in parts {
        let ty = p.get("type").and_then(Value::as_str).unwrap_or("");
        if ty == "tool_result" {
            return None;
        }
        if let Some(t) = p.get("text").and_then(Value::as_str) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(t);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Assistant entries hold a list of content parts; we concatenate `text`
/// parts and surface a hint if the trailing event is a `tool_use` (which
/// usually means Claude is paused awaiting permission).
fn extract_assistant_text(v: &Value) -> Option<String> {
    let parts = v.get("message")?.get("content")?.as_array()?;
    let mut out = String::new();
    let mut pending_tool: Option<String> = None;
    for p in parts {
        let ty = p.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "text" => {
                if let Some(t) = p.get("text").and_then(Value::as_str) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
            }
            "tool_use" => {
                let name = p.get("name").and_then(Value::as_str).unwrap_or("?");
                pending_tool = Some(format!("[pending tool_use: {name}]"));
            }
            _ => {}
        }
    }
    if let Some(hint) = pending_tool {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&hint);
    }
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn encode_cwd_replaces_slashes_with_dashes() {
        assert_eq!(
            encode_cwd(Path::new("/Users/adam/Repositories/cekanje")),
            "-Users-adam-Repositories-cekanje"
        );
    }

    #[test]
    fn transcript_path_joins_home_projects_encoded_sid() {
        let p = transcript_path(Path::new("/home/me"), Path::new("/x/y"), "abc-123");
        assert_eq!(
            p,
            PathBuf::from("/home/me/.claude/projects/-x-y/abc-123.jsonl")
        );
    }

    fn write_fixture(lines: &[&str]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cekanje-transcript-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        path
    }

    #[test]
    fn last_excerpt_prefers_most_recent_text_user_and_assistant() {
        let lines = [
            r#"{"type":"user","message":{"content":"old prompt"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"old answer"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#,
            r#"{"type":"user","message":{"content":"latest prompt"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"latest answer"},{"type":"tool_use","name":"Bash"}]}}"#,
        ];
        let path = write_fixture(&lines);
        let ex = last_excerpt(&path, 65_536).unwrap();
        assert_eq!(ex.last_user.as_deref(), Some("latest prompt"));
        let assistant = ex.last_assistant.unwrap();
        assert!(assistant.contains("latest answer"));
        assert!(assistant.contains("[pending tool_use: Bash]"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn last_excerpt_skips_tool_result_user_entries() {
        let lines = [
            r#"{"type":"user","message":{"content":"real prompt"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"output"}]}}"#,
        ];
        let path = write_fixture(&lines);
        let ex = last_excerpt(&path, 65_536).unwrap();
        assert_eq!(ex.last_user.as_deref(), Some("real prompt"));
        let _ = std::fs::remove_file(&path);
    }
}
