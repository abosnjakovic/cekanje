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

fn base_cmd(socket: Option<&str>) -> Command {
    let mut cmd = Command::new("tmux");
    if let Some(s) = socket {
        cmd.arg("-S").arg(s);
    }
    cmd
}

fn build_list_clients(socket: Option<&str>) -> Command {
    let mut cmd = base_cmd(socket);
    cmd.args(["list-clients", "-F", "#{client_pane}"]);
    cmd
}

fn build_display_message(socket: Option<&str>, pane: &str, fmt: &str) -> Command {
    let mut cmd = base_cmd(socket);
    cmd.args(["display-message", "-p", "-t", pane, fmt]);
    cmd
}

fn build_capture_pane(socket: Option<&str>, pane: &str, scrollback_lines: u32) -> Command {
    let mut cmd = base_cmd(socket);
    let scrollback = format!("-{scrollback_lines}");
    cmd.args(["capture-pane", "-p", "-t", pane, "-S", &scrollback]);
    cmd
}

fn build_list_panes(socket: Option<&str>) -> Command {
    let mut cmd = base_cmd(socket);
    cmd.args([
        "list-panes",
        "-a",
        "-F",
        "#{pane_id}\t#{pane_current_path}\t#{pane_current_command}",
    ]);
    cmd
}

/// Return active panes (one per attached client) on a given tmux server.
pub fn active_panes(socket: Option<&str>) -> Vec<String> {
    match build_list_clients(socket).output() {
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
    let out = build_display_message(socket, pane, fmt)
        .output()
        .context("tmux display-message")?;
    if !out.status.success() {
        bail!(
            "tmux display-message failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Capture the visible pane plus `scrollback_lines` of history. Returns the
/// raw text. Errors are surfaced; callers may render an empty preview if the
/// pane has gone away.
pub fn capture_pane(socket: Option<&str>, pane: &str, scrollback_lines: u32) -> Result<String> {
    let out = build_capture_pane(socket, pane, scrollback_lines)
        .output()
        .context("tmux capture-pane")?;
    if !out.status.success() {
        bail!(
            "tmux capture-pane failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
    let Ok(out) = build_list_panes(socket).output() else {
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
    let mut cmd = base_cmd(socket);
    cmd.args(args);
    let st = cmd.status().context("run tmux")?;
    if !st.success() {
        bail!("tmux exited with {st}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn parse_socket_extracts_socket_from_full_tmux_env() {
        assert_eq!(
            parse_socket("/private/tmp/tmux-501/default,12345,0"),
            "/private/tmp/tmux-501/default"
        );
    }

    #[test]
    fn parse_socket_returns_input_when_no_comma() {
        assert_eq!(parse_socket("/tmp/sock"), "/tmp/sock");
    }

    #[test]
    fn parse_socket_handles_empty_input() {
        assert_eq!(parse_socket(""), "");
    }

    #[test]
    fn base_cmd_targets_tmux_binary() {
        let cmd = base_cmd(None);
        assert_eq!(cmd.get_program(), "tmux");
        assert!(args_of(&cmd).is_empty());
    }

    #[test]
    fn base_cmd_prepends_dash_s_when_socket_given() {
        let cmd = base_cmd(Some("/tmp/sock"));
        assert_eq!(args_of(&cmd), vec!["-S", "/tmp/sock"]);
    }

    #[test]
    fn build_list_clients_no_socket() {
        let cmd = build_list_clients(None);
        assert_eq!(args_of(&cmd), vec!["list-clients", "-F", "#{client_pane}"]);
    }

    #[test]
    fn build_list_clients_with_socket() {
        let cmd = build_list_clients(Some("/tmp/s"));
        assert_eq!(
            args_of(&cmd),
            vec!["-S", "/tmp/s", "list-clients", "-F", "#{client_pane}"]
        );
    }

    #[test]
    fn build_display_message_passes_pane_and_format() {
        let cmd = build_display_message(None, "%7", "#{pane_id}");
        assert_eq!(
            args_of(&cmd),
            vec!["display-message", "-p", "-t", "%7", "#{pane_id}"]
        );
    }

    #[test]
    fn build_capture_pane_includes_negative_scrollback() {
        let cmd = build_capture_pane(None, "%2", 100);
        assert_eq!(
            args_of(&cmd),
            vec!["capture-pane", "-p", "-t", "%2", "-S", "-100"]
        );
    }

    #[test]
    fn build_list_panes_uses_tab_delimited_format() {
        let cmd = build_list_panes(None);
        let a = args_of(&cmd);
        assert_eq!(a[0], "list-panes");
        assert_eq!(a[1], "-a");
        assert_eq!(a[2], "-F");
        assert_eq!(
            a[3],
            "#{pane_id}\t#{pane_current_path}\t#{pane_current_command}"
        );
    }
}
