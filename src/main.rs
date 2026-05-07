use clap::{Parser, Subcommand};

mod client;
mod menu;
mod notify;
mod serve;
mod state;
mod tmux;

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
        } => {
            if ensure {
                if client::http_get(port, "/status").await.is_ok() {
                    return Ok(());
                }
                spawn_detached(port, idle_secs)?;
                return Ok(());
            }
            serve::run(port, idle_secs).await
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
    }
}

fn spawn_detached(port: u16, idle_secs: u64) -> anyhow::Result<()> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe()?;
    Command::new(exe)
        .args(["serve", "--port"])
        .arg(port.to_string())
        .arg("--idle-secs")
        .arg(idle_secs.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}
