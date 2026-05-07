# cekanje

Croatian *čekanje* — "waiting".

A tiny Rust daemon that tracks Claude Code sessions inside tmux, surfaces "needs attention" with a native macOS / Linux notification, and offers an fzf-style popup picker bound to a tmux key. Visiting the pane clears the notification automatically.

The binary installs as `cekanje`, with `cek` as a shorter symlink — both names are interchangeable everywhere below.

## Why

Multiple parallel Claude sessions across tmux windows are easy to lose track of. cekanje pairs each session to its tmux pane via Claude Code hooks, marks sessions Waiting on `Notification` / `Stop` events, and pops a desktop notification. From inside tmux, a key binding opens a picker; selection switches you straight to the pane, and the focus-in hook clears the Waiting flag.

## Architecture

- `cek serve` — axum HTTP daemon on `127.0.0.1:8731`. State lives in RAM. Auto-exits after configurable idle (default 30 min, zero sessions).
- Claude Code's HTTP hooks POST event JSON; the daemon reads `X-Tmux-Pane` and `X-Tmux-Socket` headers (forwarded from `$TMUX_PANE` / `$TMUX`) to bind the session to a pane.
- On a Notification or Stop, if the pane is *not* the currently focused pane on any attached client, mark Waiting + fire a native notification. If the pane is focused, treat as Working (no badge bump).
- `cek menu` queries `/list`, pipes through `fzf`, and on selection runs `tmux switch-client` + `select-pane`.
- `cek visit <pane>` clears Waiting; wired to the tmux `pane-focus-in` hook.

```
┌──────────────────┐  HTTP POST   ┌──────────────────────┐
│ Claude hooks     │─────────────▶│  cek serve           │
│ (HTTP, env-var   │  events +    │  axum on 127.0.0.1   │
│ headers)         │  pane meta   │  state in RAM        │
└──────────────────┘              │  notify-rust on event│
                                  └──────┬───────────────┘
┌──────────────────┐  HTTP        │      │
│ cek status       │◀─────────────┤      │ shells out
│ cek list         │              │      │ to tmux
│ cek visit        │              │      ▼
│ cek menu         │              │  switch-client / select-pane
└──────────────────┘              └────────────────────────────
```

## Subcommands

`cekanje` and `cek` accept the same arguments — pick whichever you like.

| | |
|---|---|
| `cek serve [--port 8731] [--ensure] [--idle-secs 1800]` | Run daemon. `--ensure` no-ops if already up, otherwise spawns detached. `--idle-secs 0` disables auto-shutdown. |
| `cek status` | Print `⏳N` if any session is Waiting; empty otherwise. (For users with a tmux status bar.) |
| `cek list` | Dump current state as JSON. |
| `cek visit <pane>` | Mark the pane's session as visited. |
| `cek menu` | fzf picker over sessions; on selection, jump to the pane. |

## Install

### Homebrew (macOS)

```bash
brew tap abosnjakovic/cekanje https://github.com/abosnjakovic/cekanje
brew install cekanje
```

Installs `cekanje` and `cek` (symlink) into `$(brew --prefix)/bin`.

### cargo

```bash
cargo install cekanje
```

The `cek` shortcut isn't created automatically by cargo; symlink it yourself:

```bash
ln -sf cekanje "$(dirname $(which cekanje))/cek"
```

### From source

```bash
cargo build --release
install -m 0755 target/release/cekanje ~/.local/bin/cekanje
ln -sf cekanje ~/.local/bin/cek
```

Requires: tmux (≥3.2 for `pane-focus-in`), `fzf`, Claude Code with HTTP-hook support.

## Configure Claude Code

Add to `~/.claude/settings.json` (top-level `hooks`):

```json
{
  "hooks": {
    "SessionStart":   [{ "matcher": "", "hooks": [{ "type": "http", "url": "http://127.0.0.1:8731/hooks/event", "headers": { "X-Tmux-Pane": "${TMUX_PANE}", "X-Tmux-Socket": "${TMUX}" }, "allowedEnvVars": ["TMUX_PANE","TMUX"] }]}],
    "Notification":   [{ "matcher": "", "hooks": [{ "type": "http", "url": "http://127.0.0.1:8731/hooks/event", "headers": { "X-Tmux-Pane": "${TMUX_PANE}", "X-Tmux-Socket": "${TMUX}" }, "allowedEnvVars": ["TMUX_PANE","TMUX"] }]}],
    "Stop":           [{ "matcher": "", "hooks": [{ "type": "http", "url": "http://127.0.0.1:8731/hooks/event", "headers": { "X-Tmux-Pane": "${TMUX_PANE}", "X-Tmux-Socket": "${TMUX}" }, "allowedEnvVars": ["TMUX_PANE","TMUX"] }]}],
    "SessionEnd":     [{ "matcher": "", "hooks": [{ "type": "http", "url": "http://127.0.0.1:8731/hooks/event", "headers": { "X-Tmux-Pane": "${TMUX_PANE}", "X-Tmux-Socket": "${TMUX}" }, "allowedEnvVars": ["TMUX_PANE","TMUX"] }]}]
  }
}
```

Daemon binds 127.0.0.1 only — no auth needed.

## Configure tmux

Append to `~/.config/tmux/tmux.conf`:

```tmux
set-hook -g session-created 'run-shell -b "cek serve --ensure"'
set-hook -g pane-focus-in   'run-shell -b "cek visit #{pane_id}"'
bind-key -n M-i run-shell 'tmux display-popup -E -w 80% -h 60% "cek menu"'
```

Reload: `tmux source-file ~/.config/tmux/tmux.conf`. Then bootstrap once with `cek serve --ensure` (the `session-created` hook only fires for newly-opened tmux sessions).

If you keep a tmux status bar, you can also add `set -ag status-right '#(cek status) '` plus `set -g status-interval 5`.

## State machine

| Event | New status | Notification |
|---|---|---|
| `SessionStart`, `UserPromptSubmit` | Working | — |
| `Notification`, `Stop` (pane focused) | Working | — *(auto-clear)* |
| `Notification`, `Stop` (pane not focused) | Waiting | macOS / Linux popup |
| `SessionEnd` | dropped | — |
| `cek visit <pane>` | Working | — |

Auto-clear (pane focused) means: if any attached tmux client's `client_pane` equals the session's pane, no badge bump and no popup. This avoids a flood of notifications for the pane you're already looking at.

## Files

- `src/main.rs` — clap dispatch
- `src/serve.rs` — axum app, event handlers, idle-shutdown task
- `src/state.rs` — Session / State types and transitions
- `src/tmux.rs` — `tmux` shell-out helpers (`is_pane_focused`, `switch_to_pane`)
- `src/menu.rs` — fzf picker
- `src/notify.rs` — `notify-rust` wrapper
- `src/client.rs` — minimal HTTP client for the CLI subcommands

## Development

```bash
make help         # all commands
make lint         # cargo fmt + clippy -D warnings (CI parity)
make test         # cargo test
make test-release # full local simulation of the release flow
```

## Verification

After install, walk through [docs/SMOKE.md](docs/SMOKE.md) — daemon up, single session, idle → notification, popup picker, focus-clear, idle shutdown.

Release flow runs as `.github/workflows/release.yml` on tag push (`v*`). It builds for aarch64-apple-darwin, x86_64-apple-darwin, and x86_64-unknown-linux-gnu, uploads tarballs to the release, regenerates `Formula/cekanje.rb` with fresh SHA256s, and publishes to crates.io. Required repo secrets: `CRATES_IO_TOKEN`, `HOMEBREW_TAP_TOKEN`.

## Limitations / TODO

- No persistence — daemon restart loses session state until each session's next hook event re-registers it.
- No web UI.
- macOS notification path tested; Linux works via `notify-rust` but autostart unit (systemd user) not yet provided.
- No rebuild from `~/.claude/projects/*` transcripts on cold start.
- Single tmux server tracked per session via the `X-Tmux-Socket` header. Multiple tmux servers run side by side fine, but cross-server menu jump only works when each client is on the right server.
