# Smoke test

End-to-end verification of a working install. Run after `brew install cekanje` (or `cargo install cekanje` plus the `cek` symlink).

Expects: tmux ≥ 3.2, Claude Code with HTTP-hook support, `~/.claude/settings.json` configured per [README — Configure Claude Code](../README.md#configure-claude-code), tmux config wired per [README — Configure tmux](../README.md#configure-tmux).

## 1. Daemon up

```bash
cek serve --ensure
cek list             # expect: []
cek status           # expect: empty (no badge)
```

If `cek list` errors with a connection failure, the daemon didn't start. Run `cek serve` in the foreground to see logs.

## 2. Single session

In a tmux pane:

```bash
cd /tmp/scratch && claude
```

In a sibling pane:

```bash
cek list             # expect: one entry, status "working", tmux.pane set
```

## 3. Idle → notification

Inside the Claude pane: send a prompt, let Claude finish responding (it ends in a `Stop` event).

Expected:

- macOS / Linux native notification pops with title "Claude is waiting"
- `cek status` prints `⏳1`
- `cek list` shows the session in `"status": "waiting"` with `waiting_since_secs` increasing

## 4. Popup picker

From any tmux pane, press `M-i` (the binding from your tmux.conf — adjust if different). An fzf-style popup lists the waiting session. Hit Enter; tmux jumps to its pane.

The `pane-focus-in` hook fires automatically on switch:

```bash
cek status           # expect: empty (auto-cleared on focus)
cek list             # expect: status "working"
```

## 5. Two concurrent sessions

Open a second Claude session in another tmux pane. Idle both.

Expected:

- `cek status` prints `⏳2`
- `cek list` shows both sessions, both Waiting; the older `waiting_since_secs` sorted first
- Visiting one pane drops the badge to `⏳1`

## 6. Idle daemon shutdown

Drop all Claude sessions (`SessionEnd` fires when each exits). Wait 30 minutes (or run `cek serve --idle-secs 60` for a faster check).

Expected:

```bash
pgrep -lf cekanje    # daemon process gone
```

Next tmux `session-created` re-spawns it via `cek serve --ensure`.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| `cek list` works but no notifications fire | macOS notification permission for "cekanje" — check System Settings → Notifications, allow. |
| Hook fires but session doesn't appear | `~/.claude/settings.json` `hooks` block missing or daemon not on `127.0.0.1:8731` — verify port. |
| `M-i` does nothing | tmux config not reloaded; `tmux source-file ~/.config/tmux/tmux.conf`. |
| Picker shows session but selecting hangs | tmux `display-popup` lacks `-E`; selection runs in a sub-shell that exits cleanly only with `-E`. |
| `cek status` always empty even with idle session | Pane is the active pane on at least one attached tmux client → auto-clear path. Detach/attach to verify. |
