# Tmux Compat Server: No-Op Commands

Commands that are parsed and accepted (return success) but don't actually do anything yet.
These are candidates for real implementation later.

## Remaining No-Ops

### copy-mode

**Handler**: `handle_copy_mode()` in `handlers.rs`
**What it does now**: Returns empty success.
**What it should do**: `-q` could dismiss WezTerm's copy overlay if active. Entering copy mode could trigger WezTerm's copy overlay.
**Used by**: iTerm2 CC mode (sends `copy-mode -q` on connect as defensive cleanup)

### pipe-pane

**Handler**: Inline stub in `handlers.rs`
**What it does now**: Returns empty success.
**What it should do**: Pipe pane output to a shell command (or toggle off if no command given). Our CC `%output` notifications partially cover this use case.
**Used by**: Logging/monitoring scripts

### display-popup / display-menu

**Handler**: Inline stub in `handlers.rs`
**What it does now**: Returns empty success.
**What it should do**: Show a popup/overlay inside the terminal. Would require WezTerm GUI overlay support. CC protocol has no popup mechanism; iTerm2 also forbids these.
**Used by**: tmux popup scripts, fzf-tmux

### kill-server

**Handler**: Inline in `handlers.rs`
**What it does now**: Sets `ctx.detach_requested = true`, which disconnects the CC client. Does NOT actually kill the WezTerm process.
**Why it stays**: Arguably correct behavior — killing the WezTerm GUI from a tmux command would be unexpected. Detach-only is the right semantics.
**Used by**: Cleanup scripts

---

## Implemented (no longer no-ops)

### select-layout ✓

Rearranges panes in a tab according to named layout presets. Implemented via
`Tab::apply_layout()` in `tab.rs` with support for: `even-horizontal`,
`even-vertical`, `main-vertical`, `main-horizontal`, `tiled`.

### break-pane ✓

Moves a pane out of its current split into a new tab (optionally in a different
workspace/session). Calls `mux.move_pane_to_new_tab()` and registers the new
tab/session in the ID map.

### select-pane -P (style) ✓

Parses tmux `-P "bg=X,fg=Y"` style strings and applies fg/bg colors via OSC
`ChangeDynamicColors`/`ResetDynamicColor` sequences through `pane.perform_actions()`.
Supports named colors, `#rrggbb`, `colour0`–`colour255`, and `default` (reset).

### set-option (partial) ✓

Handles `pane-border-format` (sets pane header text via `pane.set_header()`) and
`pane-border-status` (enables/disables header display). All other options remain
soft no-ops with debug logging.

### run-shell (complete) ✓

Now supports all flags: `-b` (background execution via `promise::spawn`),
`-d <secs>` (delay via `smol::Timer`), `-t <pane>` (route output to target pane
via `pane.writer()`). Foreground mode blocks until completion; background mode
returns immediately.

### wait-for ✓

Channel-based wait/signal synchronization. `wait-for <channel>` blocks until
another client sends `wait-for -S <channel>`. Uses a global `WAIT_CHANNELS` store
with `async-channel` senders/receivers. `-L`/`-U` (lock/unlock) return immediately.

### join-pane ✓

Already working — `join-pane` is an alias for `move-pane`, which is fully
implemented.

### env.exe shim ✓

New `env-shim` crate producing `env.exe` for Windows. Emulates Unix `env` command
(`KEY=VAL command args...`, `-i`, `-u VAR`). Deployed to the tmux-compat PATH
directory alongside `tmux.exe`.
