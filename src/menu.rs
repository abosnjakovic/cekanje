use crate::client;
use crate::state::{Session, Status};
use crate::tmux;
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const PROJECT_W: usize = 16;
const WAITING_W: usize = 8;
const MESSAGE_W: usize = 80;

const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Set on the child process when we re-launch ourselves inside a tmux popup.
/// Presence means "render the picker in-line"; absence means "wrap me in a
/// fullscreen popup first". The single env var keeps the bind config small —
/// users put `cek menu` on their key and the binary handles the popup.
const FULLSCREEN_ENV: &str = "CEK_FULLSCREEN_HOST";

pub async fn run(port: u16) -> Result<()> {
    if std::env::var_os(FULLSCREEN_ENV).is_none() {
        return relaunch_in_popup(port);
    }

    let body = client::http_get(port, "/list")
        .await
        .context("fetch /list from cekanje daemon")?;
    let sessions: Vec<Session> = serde_json::from_str(&body).unwrap_or_default();

    let renderable: Vec<&Session> = sessions.iter().filter(|s| s.tmux.is_some()).collect();

    if renderable.is_empty() {
        eprintln!("(no claude sessions)");
        return Ok(());
    }

    let mut lines = String::new();
    lines.push_str(&header_row());
    lines.push('\n');
    for s in &renderable {
        lines.push_str(&format_row(s));
        lines.push('\n');
    }

    let preview_cmd = format!(
        "{} preview --port {} {{5}}",
        std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_else(|| "cek".to_string()),
        port,
    );

    let mut child = Command::new("fzf")
        .args([
            "--ansi",
            "--delimiter=\t",
            "--with-nth=1,2,3,4",
            "--header-lines=1",
            "--height=100%",
            "--no-info",
            "--prompt=claude> ",
            "--preview",
            &preview_cmd,
            "--preview-window=right:55%:wrap",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("spawn fzf — is it installed?")?;

    child
        .stdin
        .as_mut()
        .expect("fzf stdin")
        .write_all(lines.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Ok(());
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let sid = line.split('\t').nth(4).unwrap_or("").trim();
    if sid.is_empty() {
        return Ok(());
    }
    let Some(target) = sessions.iter().find(|s| s.session_id == sid) else {
        return Ok(());
    };
    let Some(loc) = &target.tmux else {
        bail!("no tmux location recorded for {sid}");
    };
    tmux::switch_to_pane(loc.socket.as_deref(), &loc.pane)?;
    Ok(())
}

fn header_row() -> String {
    format!(
        "{BOLD}{:<PROJECT_W$}{RESET}\t{BOLD}{:<WAITING_W$}{RESET}\t{BOLD}{}{RESET}\t \t",
        "PROJECT", "WAITING", "LAST MESSAGE",
    )
}

fn format_row(s: &Session) -> String {
    let icon = match s.status {
        Status::Waiting => "⏳",
        Status::Working => "  ",
    };
    let project = s
        .cwd
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .map(String::from)
        .or_else(|| s.tmux.as_ref().map(|t| t.pane.clone()))
        .unwrap_or_else(|| "?".to_string());
    let project = truncate_pad(&project, PROJECT_W);

    let waiting = s
        .waiting_since_secs
        .map(|n| format!("{n}s"))
        .unwrap_or_else(|| "—".to_string());
    let waiting = truncate_pad(&waiting, WAITING_W);

    let raw = s.last_message.as_deref().unwrap_or("");
    let first = raw.lines().next().unwrap_or("");
    let message = if first.chars().count() > MESSAGE_W {
        format!("{}…", first.chars().take(MESSAGE_W - 1).collect::<String>())
    } else {
        first.to_string()
    };

    let colour = match s.status {
        Status::Waiting => YELLOW,
        Status::Working => DIM,
    };

    format!(
        "{colour}{icon} {project}{RESET}\t{colour}{waiting}{RESET}\t{colour}{message}{RESET}\t \t{sid}",
        sid = s.session_id,
    )
}

/// Re-launch `cek menu` inside a borderless fullscreen tmux popup. `-E` makes
/// the popup auto-close when the inner command exits; `-B` removes the popup
/// border; `100%` width and height fill the terminal. The child sees
/// `CEK_FULLSCREEN_HOST=1` and skips this branch.
fn relaunch_in_popup(port: u16) -> Result<()> {
    let exe = std::env::current_exe().context("locate current binary")?;
    let exe_str = exe
        .to_str()
        .context("current binary path is not valid UTF-8")?;
    let inner = format!("{} menu --port {}", shell_quote(exe_str), port);

    let status = Command::new("tmux")
        .args([
            "display-popup",
            "-E",
            "-B",
            "-w",
            "100%",
            "-h",
            "100%",
            "-e",
            &format!("{FULLSCREEN_ENV}=1"),
        ])
        .arg(&inner)
        .status()
        .context(
            "spawn tmux display-popup — is tmux installed and are you inside a tmux session?",
        )?;
    if !status.success() {
        bail!("tmux display-popup exited with {status}");
    }
    Ok(())
}

/// Single-quote a shell argument. Adequate for filesystem paths — we replace
/// any embedded single quote with the standard `'\''` close-reopen idiom.
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    out.push_str(&s.replace('\'', "'\\''"));
    out.push('\'');
    out
}

/// Truncate to `width` chars (Unicode-aware) and right-pad with spaces. Avoids
/// byte-based slicing that could split a multi-byte codepoint.
fn truncate_pad(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count >= width {
        s.chars().take(width).collect()
    } else {
        let mut out = String::with_capacity(width);
        out.push_str(s);
        for _ in count..width {
            out.push(' ');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Session, Status};

    fn session(status: Status) -> Session {
        let json = serde_json::json!({
            "session_id": "S1",
            "cwd": "/tmp/proj",
            "tmux": { "pane": "%1", "socket": null },
            "status": match status {
                Status::Working => "working",
                Status::Waiting => "waiting",
            },
            "last_message": "hello world",
            "waiting_since_secs": 42u64,
            "age_secs": 100u64,
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn truncate_pad_pads_short_ascii() {
        assert_eq!(truncate_pad("hi", 5), "hi   ");
    }

    #[test]
    fn truncate_pad_truncates_long_ascii() {
        assert_eq!(truncate_pad("abcdef", 3), "abc");
    }

    #[test]
    fn truncate_pad_handles_exact_width() {
        assert_eq!(truncate_pad("abc", 3), "abc");
    }

    #[test]
    fn truncate_pad_counts_unicode_codepoints_not_bytes() {
        // ⏳ is 1 codepoint, multiple bytes.
        let out = truncate_pad("⏳hi", 4);
        assert_eq!(out.chars().count(), 4);
        assert_eq!(out, "⏳hi ");
    }

    #[test]
    fn shell_quote_wraps_plain_string() {
        assert_eq!(shell_quote("/tmp/cek"), "'/tmp/cek'");
    }

    #[test]
    fn shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("O'Brien"), r"'O'\''Brien'");
    }

    #[test]
    fn header_row_contains_column_titles() {
        let h = header_row();
        assert!(h.contains("PROJECT"));
        assert!(h.contains("WAITING"));
        assert!(h.contains("LAST MESSAGE"));
    }

    #[test]
    fn format_row_waiting_uses_yellow_and_hourglass() {
        let s = session(Status::Waiting);
        let row = format_row(&s);
        assert!(row.contains(YELLOW));
        assert!(row.contains("⏳"));
        assert!(row.contains("42s"));
    }

    #[test]
    fn format_row_working_uses_dim_and_dash() {
        let mut s = session(Status::Working);
        s.waiting_since_secs = None;
        let row = format_row(&s);
        assert!(row.contains(DIM));
        assert!(row.contains("—"));
    }

    #[test]
    fn format_row_uses_project_name_from_cwd() {
        let s = session(Status::Working);
        let row = format_row(&s);
        // PROJECT_W=16, "proj" padded.
        assert!(row.contains("proj"));
    }

    #[test]
    fn format_row_falls_back_to_pane_when_cwd_missing() {
        let mut s = session(Status::Working);
        s.cwd = None;
        let row = format_row(&s);
        assert!(row.contains("%1"));
    }

    #[test]
    fn format_row_truncates_long_message_with_ellipsis() {
        let mut s = session(Status::Working);
        s.last_message = Some("x".repeat(200));
        let row = format_row(&s);
        assert!(row.contains('…'));
    }

    #[test]
    fn format_row_takes_only_first_line_of_message() {
        let mut s = session(Status::Working);
        s.last_message = Some("first line\nsecond line".into());
        let row = format_row(&s);
        assert!(row.contains("first line"));
        assert!(!row.contains("second line"));
    }

    #[test]
    fn format_row_ends_with_session_id_as_last_field() {
        let s = session(Status::Working);
        let row = format_row(&s);
        let last = row.split('\t').next_back().unwrap();
        assert_eq!(last, "S1");
    }
}
