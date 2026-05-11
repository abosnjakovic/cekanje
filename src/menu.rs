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
