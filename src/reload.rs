//! Hot-reload of `cek serve` when the daemon's on-disk binary changes
//! (brew upgrade, cargo install, manual tarball swap).
//!
//! Lazy detection: every hook event triggers a cheap `stat` of the binary
//! and, if its fingerprint differs from startup, signals the running server
//! to drain and replace its own process image in place via `execve(2)`.
//! PID is preserved so tmux's child relationship survives.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use serde::Serialize;
use tokio::sync::Notify;
use tracing::{info, warn};

/// Inode-level identity of the binary on disk. `fs::metadata` follows
/// symlinks intentionally — brew installs the daemon as a symlink into
/// `Cellar/cekanje/<ver>/bin/cek`, and an upgrade retargets that symlink.
/// Following it is exactly how we notice the upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BinFp {
    pub dev: u64,
    pub ino: u64,
    pub mtime: i64,
    pub size: u64,
}

impl BinFp {
    pub fn snapshot(path: &Path) -> std::io::Result<Self> {
        let m = fs::metadata(path)?;
        Ok(Self {
            dev: m.dev(),
            ino: m.ino(),
            mtime: m.mtime(),
            size: m.size(),
        })
    }
}

pub struct ReloadCtx {
    pub exe_path: PathBuf,
    pub startup_fp: BinFp,
    pub argv_tail: Vec<String>,
    swap_in_flight: AtomicBool,
    shutdown: std::sync::Arc<Notify>,
}

static CTX: OnceLock<ReloadCtx> = OnceLock::new();

/// Capture the binary's identity at startup.
pub fn init(
    exe_path: PathBuf,
    argv_tail: Vec<String>,
    shutdown: std::sync::Arc<Notify>,
) -> anyhow::Result<()> {
    let startup_fp = BinFp::snapshot(&exe_path)
        .with_context(|| format!("snapshot fingerprint for {}", exe_path.display()))?;
    let _ = CTX.set(ReloadCtx {
        exe_path,
        startup_fp,
        argv_tail,
        swap_in_flight: AtomicBool::new(false),
        shutdown,
    });
    Ok(())
}

pub fn ctx() -> Option<&'static ReloadCtx> {
    CTX.get()
}

pub fn current_fp() -> Option<BinFp> {
    let c = ctx()?;
    BinFp::snapshot(&c.exe_path).ok()
}

/// True when the on-disk fingerprint differs from the one captured at boot.
/// Errors stat'ing the binary are treated as "not stale" — a transient FS
/// hiccup must not euthanize a healthy daemon.
pub fn is_stale() -> bool {
    let Some(c) = ctx() else {
        return false;
    };
    match BinFp::snapshot(&c.exe_path) {
        Ok(cur) => cur != c.startup_fp,
        Err(e) => {
            warn!(error = %e, "fingerprint stat failed");
            false
        }
    }
}

/// Returns true if a swap has been requested and is in flight (used by
/// `serve::run` to decide whether to re-image after graceful shutdown).
pub fn was_swap_requested() -> bool {
    ctx()
        .map(|c| c.swap_in_flight.load(Ordering::SeqCst))
        .unwrap_or(false)
}

/// Called from the post-handler middleware. Checks staleness, claims the
/// single-flight slot, and notifies the server to begin graceful shutdown.
pub fn request_swap_if_stale() {
    let Some(c) = ctx() else {
        return;
    };
    if !is_stale() {
        return;
    }
    if c.swap_in_flight.swap(true, Ordering::SeqCst) {
        return;
    }
    info!(exe = %c.exe_path.display(), "binary changed on disk; initiating swap");
    // notify_waiters (not notify_one) so both the axum graceful-shutdown
    // future and the drain-deadline timer in serve::run wake together.
    c.shutdown.notify_waiters();
}

/// Replace the current process image with a fresh invocation of the new
/// binary. Does not return on success — control transfers into the new image.
pub fn replace_process_image() -> anyhow::Result<std::convert::Infallible> {
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    let c = ctx().context("reload ctx not initialised")?;
    info!(
        exe = %c.exe_path.display(),
        argv = ?c.argv_tail,
        "replacing process image"
    );
    let mut cmd = Command::new(&c.exe_path);
    cmd.args(&c.argv_tail);
    // Use the trait fn as a function pointer to avoid a method-call form.
    let replace: fn(&mut Command) -> std::io::Error = CommandExt::exec;
    let err = replace(&mut cmd);
    Err(anyhow::anyhow!("process replacement failed: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn snapshot_then_modify_changes_fingerprint() {
        let mut f = tempfile_shim::NamedTemp::new("alpha");
        let p = f.path().to_owned();
        let a = BinFp::snapshot(&p).unwrap();

        // Sleep just past 1-second mtime resolution on some filesystems.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        f.as_file_mut().write_all(b"-beta").unwrap();
        f.as_file_mut().sync_all().unwrap();

        let b = BinFp::snapshot(&p).unwrap();
        assert_ne!(a, b, "fingerprint must change after file mutation");
    }

    #[test]
    fn snapshot_stable_for_unchanged_file() {
        let f = tempfile_shim::NamedTemp::new("steady");
        let p = f.path().to_owned();
        let a = BinFp::snapshot(&p).unwrap();
        let b = BinFp::snapshot(&p).unwrap();
        assert_eq!(a, b);
    }

    /// Minimal stand-in so we don't pull in the `tempfile` crate just for
    /// these two tests.
    mod tempfile_shim {
        use std::fs::{File, OpenOptions};
        use std::io::Write;
        use std::path::{Path, PathBuf};

        pub struct NamedTemp {
            path: PathBuf,
            file: File,
        }

        impl NamedTemp {
            pub fn new(contents: &str) -> Self {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let path = std::env::temp_dir().join(format!("cek-reload-test-{nanos}"));
                let mut file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .unwrap();
                file.write_all(contents.as_bytes()).unwrap();
                file.sync_all().unwrap();
                Self { path, file }
            }
            pub fn path(&self) -> &Path {
                &self.path
            }
            pub fn as_file_mut(&mut self) -> &mut File {
                &mut self.file
            }
        }

        impl Drop for NamedTemp {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}
