use crate::client;
use crate::state::{Session, Status};
use crate::tmux;
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub async fn run(port: u16) -> Result<()> {
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
    for s in &renderable {
        let icon = match s.status {
            Status::Waiting => "⏳",
            Status::Working => "  ",
        };
        let label = s
            .cwd
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .map(String::from)
            .or_else(|| s.tmux.as_ref().map(|t| t.pane.clone()))
            .unwrap_or_else(|| "?".to_string());
        let raw = s.last_message.as_deref().unwrap_or("");
        let first = raw.lines().next().unwrap_or("");
        let msg = if first.chars().count() > 80 {
            format!("{}…", first.chars().take(79).collect::<String>())
        } else {
            first.to_string()
        };
        let age = s
            .waiting_since_secs
            .map(|n| format!("{n}s"))
            .unwrap_or_default();
        lines.push_str(&format!(
            "{icon} {label:<10} {age:<5} {msg}\t{sid}\n",
            sid = s.session_id
        ));
    }

    let mut child = Command::new("fzf")
        .args([
            "--with-nth=1",
            "--delimiter=\t",
            "--height=100%",
            "--no-info",
            "--prompt=claude> ",
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
    let sid = line.split('\t').nth(1).unwrap_or("").trim();
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
