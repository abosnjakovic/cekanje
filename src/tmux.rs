use crate::state::TmuxLocation;
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

/// $TMUX is `<socket-path>,<pid>,<session>`. We want only the socket path.
pub fn parse_socket(tmux_env: &str) -> String {
    tmux_env.split(',').next().unwrap_or(tmux_env).to_string()
}

/// Return active panes (one per attached client) on a given tmux server.
pub fn active_panes(socket: Option<&str>) -> Vec<String> {
    let mut cmd = Command::new("tmux");
    if let Some(s) = socket {
        cmd.arg("-S").arg(s);
    }
    cmd.args(["list-clients", "-F", "#{client_pane}"]);
    match cmd.output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Is a given pane currently the active pane for any attached client?
pub fn is_pane_focused(socket: Option<&str>, pane: &str) -> bool {
    active_panes(socket).iter().any(|p| p == pane)
}

/// Switch the current tmux client to the window containing `pane`, then select that pane.
pub fn switch_to_pane(socket: Option<&str>, pane: &str) -> Result<()> {
    let target = display_message(socket, pane, "#{session_id}:#{window_id}")?;
    run(socket, ["switch-client", "-t", &target])?;
    run(socket, ["select-pane", "-t", pane])?;
    Ok(())
}

pub fn display_message(socket: Option<&str>, pane: &str, fmt: &str) -> Result<String> {
    let mut cmd = Command::new("tmux");
    if let Some(s) = socket {
        cmd.arg("-S").arg(s);
    }
    cmd.args(["display-message", "-p", "-t", pane, fmt]);
    let out = cmd.output().context("tmux display-message")?;
    if !out.status.success() {
        bail!(
            "tmux display-message failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Verify a pane id still resolves on the given tmux server.
pub fn pane_alive(socket: Option<&str>, pane: &str) -> bool {
    display_message(socket, pane, "#{pane_id}").is_ok()
}

/// Discover panes currently running `claude`, grouped by their `pane_current_path`.
/// Used by the cold-start heuristic to recover bindings when persisted state is missing
/// a tmux pane (e.g. a session that started before this daemon ever ran).
///
/// Only the pane's reported current command is matched; we cannot retrieve the
/// session_id from the claude process itself, so disambiguation is the caller's job.
pub fn list_claude_panes_by_cwd(socket: Option<&str>) -> HashMap<PathBuf, Vec<TmuxLocation>> {
    let mut cmd = Command::new("tmux");
    if let Some(s) = socket {
        cmd.arg("-S").arg(s);
    }
    cmd.args([
        "list-panes",
        "-a",
        "-F",
        "#{pane_id}\t#{pane_current_path}\t#{pane_current_command}",
    ]);
    let Ok(out) = cmd.output() else {
        return HashMap::new();
    };
    if !out.status.success() {
        return HashMap::new();
    }

    let socket_owned = socket.map(String::from);
    let mut by_cwd: HashMap<PathBuf, Vec<TmuxLocation>> = HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.split('\t');
        let (Some(pane), Some(path), Some(cmd_name)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if cmd_name != "claude" {
            continue;
        }
        by_cwd
            .entry(PathBuf::from(path))
            .or_default()
            .push(TmuxLocation {
                pane: pane.to_string(),
                socket: socket_owned.clone(),
            });
    }
    by_cwd
}

pub fn run<I, S>(socket: Option<&str>, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new("tmux");
    if let Some(s) = socket {
        cmd.arg("-S").arg(s);
    }
    cmd.args(args);
    let st = cmd.status().context("run tmux")?;
    if !st.success() {
        bail!("tmux exited with {st}");
    }
    Ok(())
}
