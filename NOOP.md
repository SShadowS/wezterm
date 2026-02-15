# Tmux Compat Server: No-Op Commands

Commands that are parsed and accepted (return success) but don't actually do anything yet.
These are candidates for real implementation later.

## Remaining No-Ops

### copy-mode

**Handler**: `handle_copy_mode()` in `handlers.rs`
**What it does now**: Returns empty success. All valid tmux flags (`-d`, `-e`, `-H`, `-M`, `-q`, `-S`, `-s`, `-t`, `-u`) are now accepted by the parser.
**What it should do**: `-q` could dismiss WezTerm's copy overlay if active. Entering copy mode could trigger WezTerm's copy overlay.
**Used by**: iTerm2 CC mode (sends `copy-mode -q` on connect as defensive cleanup)

### display-popup / display-menu

**Handler**: Inline no-op in `handlers.rs`
**What it does now**: Returns empty success. Parser accepts all valid tmux flags for both `display-popup` and `display-menu` (including boolean flags `-B`, `-C`, `-k`, `-M`, `-N`, `-O` and value flags `-b`, `-c`, `-d`, `-e`, `-h`, `-H`, `-s`, `-S`, `-T`, `-w`, `-x`, `-y`). `display-menu` appears in `list-commands`.
**What it should do**: Show a popup/overlay inside the terminal. Would require WezTerm GUI overlay support. CC protocol has no popup mechanism; iTerm2 also forbids these.
**Used by**: tmux popup scripts, fzf-tmux

---

## Implemented (no longer no-ops)

### pipe-pane ✓

Full implementation via `handle_pipe_pane()` in `handlers.rs`. Spawns a shell
command and connects pane I/O: `-O` (default) taps pane output to child stdin
via `register_output_tap()`, `-I` pipes child stdout to `pane.writer()`.
Supports `-o` (toggle — no-op if pipe already exists). No command closes
existing pipe. Pipes are automatically cleaned up on pane removal.

### kill-server ✓

Kills all sessions (workspaces) by removing all windows, tabs, and panes,
cleaning up pipe-pane handles and ID mappings, then detaches the CC client
with `%exit server killed`. Does NOT kill the WezTerm GUI process.

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
