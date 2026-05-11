use clap::{Parser, Subcommand};

mod client;
mod menu;
mod notify;
mod persist;
mod preview;
mod rebuild;
mod restore;
mod serve;
mod state;
mod tmux;
mod transcript;

#[derive(Parser)]
#[command(
    name = "cekanje",
    about = "tmux notifier daemon for Claude sessions",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the HTTP daemon that receives Claude hook events
    Serve {
        #[arg(long, default_value_t = 8731)]
        port: u16,
        /// If a daemon is already running, exit successfully without starting another.
        /// If not, spawn a detached daemon and exit.
        #[arg(long)]
        ensure: bool,
        /// Self-exit after this many seconds with zero registered sessions. 0 disables.
        #[arg(long, default_value_t = 1800)]
        idle_secs: u64,
        /// On startup, scan ~/.claude/projects/ for transcripts modified within
        /// this many seconds and pre-populate sessions as Working. 0 disables.
        #[arg(long, default_value_t = 300)]
        rebuild_window_secs: u64,
    },
    /// Print the tmux status-bar badge text (empty when no sessions are waiting)
    Status {
        #[arg(long, default_value_t = 8731)]
        port: u16,
    },
    /// Print the current state as JSON
    List {
        #[arg(long, default_value_t = 8731)]
        port: u16,
    },
    /// Mark the session bound to the given pane as visited (clears Waiting)
    Visit {
        pane: String,
        #[arg(long, default_value_t = 8731)]
        port: u16,
    },
    /// Open an fzf picker over current sessions; on selection, switch tmux to that pane
    Menu {
        #[arg(long, default_value_t = 8731)]
        port: u16,
    },
    /// Render an fzf preview block for the given session_id (used by `cek menu` internally)
    #[command(hide = true)]
    Preview {
        session_id: String,
        #[arg(long, default_value_t = 8731)]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cekanje=info".into()),
        )
        .init();

    match Cli::parse().command {
        Cmd::Serve {
            port,
            ensure,
            idle_secs,
            rebuild_window_secs,
        } => {
            if ensure {
                if client::http_get(port, "/status").await.is_ok() {
                    return Ok(());
                }
                spawn_detached(port, idle_secs, rebuild_window_secs)?;
                return Ok(());
            }
            serve::run(port, idle_secs, rebuild_window_secs).await
        }
        Cmd::Status { port } => {
            let body = client::http_get(port, "/status").await.unwrap_or_default();
            print!("{body}");
            Ok(())
        }
        Cmd::List { port } => {
            let body = client::http_get(port, "/list").await?;
            println!("{body}");
            Ok(())
        }
        Cmd::Visit { pane, port } => {
            let body = serde_json::json!({ "pane": pane }).to_string();
            client::http_post_json(port, "/visit", &body).await?;
            Ok(())
        }
        Cmd::Menu { port } => menu::run(port).await,
        Cmd::Preview { session_id, port } => preview::run(port, &session_id).await,
    }
}

fn spawn_detached(port: u16, idle_secs: u64, rebuild_window_secs: u64) -> anyhow::Result<()> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe()?;
    Command::new(exe)
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .arg("--idle-secs")
        .arg(idle_secs.to_string())
        .arg("--rebuild-window-secs")
        .arg(rebuild_window_secs.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("parse")
    }

    #[test]
    fn serve_defaults() {
        let cli = parse(&["cek", "serve"]);
        match cli.command {
            Cmd::Serve {
                port,
                ensure,
                idle_secs,
                rebuild_window_secs,
            } => {
                assert_eq!(port, 8731);
                assert!(!ensure);
                assert_eq!(idle_secs, 1800);
                assert_eq!(rebuild_window_secs, 300);
            }
            _ => panic!("expected Serve"),
        }
    }

    #[test]
    fn serve_with_all_flags() {
        let cli = parse(&[
            "cek",
            "serve",
            "--port",
            "9000",
            "--ensure",
            "--idle-secs",
            "0",
            "--rebuild-window-secs",
            "0",
        ]);
        match cli.command {
            Cmd::Serve {
                port,
                ensure,
                idle_secs,
                rebuild_window_secs,
            } => {
                assert_eq!(port, 9000);
                assert!(ensure);
                assert_eq!(idle_secs, 0);
                assert_eq!(rebuild_window_secs, 0);
            }
            _ => panic!("expected Serve"),
        }
    }

    #[test]
    fn status_subcommand_with_custom_port() {
        let cli = parse(&["cek", "status", "--port", "9000"]);
        match cli.command {
            Cmd::Status { port } => assert_eq!(port, 9000),
            _ => panic!("expected Status"),
        }
    }

    #[test]
    fn list_subcommand_uses_default_port() {
        let cli = parse(&["cek", "list"]);
        match cli.command {
            Cmd::List { port } => assert_eq!(port, 8731),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn visit_requires_pane_argument() {
        let cli = parse(&["cek", "visit", "%42"]);
        match cli.command {
            Cmd::Visit { pane, port } => {
                assert_eq!(pane, "%42");
                assert_eq!(port, 8731);
            }
            _ => panic!("expected Visit"),
        }
    }

    #[test]
    fn menu_subcommand_parses() {
        let cli = parse(&["cek", "menu"]);
        assert!(matches!(cli.command, Cmd::Menu { port: 8731 }));
    }

    #[test]
    fn preview_takes_session_id() {
        let cli = parse(&["cek", "preview", "abc-123"]);
        match cli.command {
            Cmd::Preview { session_id, port } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(port, 8731);
            }
            _ => panic!("expected Preview"),
        }
    }

    #[test]
    fn missing_subcommand_errors() {
        assert!(Cli::try_parse_from(["cek"]).is_err());
    }

    #[test]
    fn visit_without_pane_errors() {
        assert!(Cli::try_parse_from(["cek", "visit"]).is_err());
    }

    #[test]
    fn preview_without_session_id_errors() {
        assert!(Cli::try_parse_from(["cek", "preview"]).is_err());
    }

    #[test]
    fn unknown_subcommand_errors() {
        assert!(Cli::try_parse_from(["cek", "bogus"]).is_err());
    }
}
