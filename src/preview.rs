//! Render an fzf preview block for a given session_id. Invoked by
//! `cek preview <sid>`, which is what `cek menu`'s `--preview` flag points
//! at. All output goes to stdout. Failures degrade gracefully — a missing
//! transcript or dead pane just elides that section.

use crate::client;
use crate::state::{Session, Status};
use crate::tmux;
use crate::transcript;
use anyhow::{Context, Result};
use std::path::PathBuf;

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

    print_metadata(&session);
    print_transcript_excerpt(&session);
    print_pane_capture(&session);
    Ok(())
}

fn rule(label: &str) {
    println!("{RULE_PREFIX}{label}{RULE_SUFFIX}");
}

fn print_metadata(s: &Session) {
    rule("session");
    println!("sid:     {DIM}{}{RESET}", s.session_id);
    if let Some(cwd) = &s.cwd {
        println!("cwd:     {}", cwd.display());
    }
    if let Some(t) = &s.tmux {
        match &t.socket {
            Some(sock) => println!("pane:    {}  {DIM}socket: {sock}{RESET}", t.pane),
            None => println!("pane:    {}", t.pane),
        }
    }
    let status = match s.status {
        Status::Waiting => match s.waiting_since_secs {
            Some(n) => format!("\x1b[33mwaiting\x1b[0m ({n}s)"),
            None => "\x1b[33mwaiting\x1b[0m".to_string(),
        },
        Status::Working => "working".to_string(),
    };
    println!("status:  {status}");
    println!("age:     {}s", s.age_secs);
    if let Some(msg) = &s.last_message {
        println!("message: {msg}");
    }
    println!();
}

fn print_transcript_excerpt(s: &Session) {
    let Some(cwd) = s.cwd.as_deref() else { return };
    let Some(home) = home_dir() else { return };
    let path = transcript::transcript_path(&home, cwd, &s.session_id);
    if !path.exists() {
        return;
    }
    let Ok(ex) = transcript::last_excerpt(&path, TRANSCRIPT_TAIL_BYTES) else {
        return;
    };

    if let Some(text) = ex.last_user {
        rule("you asked");
        for line in text.lines() {
            println!("{line}");
        }
        println!();
    }
    if let Some(text) = ex.last_assistant {
        rule("claude said");
        let lines: Vec<&str> = text.lines().collect();
        let take = lines.len().min(ASSISTANT_MAX_LINES);
        for line in &lines[..take] {
            println!("{line}");
        }
        if lines.len() > ASSISTANT_MAX_LINES {
            println!("{DIM}…({} more lines){RESET}", lines.len() - take);
        }
        println!();
    }
}

fn print_pane_capture(s: &Session) {
    let Some(loc) = &s.tmux else { return };
    rule("pane");
    match tmux::capture_pane(loc.socket.as_deref(), &loc.pane, PANE_SCROLLBACK_LINES) {
        Ok(out) => print!("{out}"),
        Err(e) => println!("{DIM}(capture failed: {e}){RESET}"),
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
