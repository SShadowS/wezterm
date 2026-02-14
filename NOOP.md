# Tmux Compat Server: No-Op Commands

Commands that are parsed and accepted (return success) but don't actually do anything yet.
These are candidates for real implementation later.

## set-option

**Handler**: `handle_set_option()` in `handlers.rs:2445`
**What it does now**: Logs the option name/value pair and returns empty success.
**What it should do**: Apply relevant options to WezTerm. Useful subset:

- `-p -t <pane> pane-border-style "fg=<color>"` — Could map to WezTerm pane border colors
- `-p -t <pane> pane-active-border-style "fg=<color>"` — Active pane border color
- `-p -t <pane> pane-border-format "<format>"` — Pane border label (could map to tab title format)
- `-w -t <window> pane-border-status top` — Enable pane labels (could toggle WezTerm tab bar)
- Global options like `mouse`, `status`, `escape-time` could map to WezTerm config equivalents

**Used by**: Claude Code team mode (pane styling), iTerm2 CC mode

## select-layout

**Handler**: Inline `Ok(String::new())` in `handlers.rs:693`
**What it does now**: Silently ignored.
**What it should do**: Rearrange pane layout within a tab. Common layouts:

- `main-vertical` — One large pane on left, rest stacked on right (Claude Code team default)
- `tiled` — All panes equal size (Claude Code external swarm mode)
- `even-horizontal` / `even-vertical` — Equal horizontal/vertical splits

**Used by**: Claude Code team mode (rebalancing after pane create/destroy)

## copy-mode

**Handler**: `handle_copy_mode()` in `handlers.rs:841`
**What it does now**: Returns empty success.
**What it should do**: `-q` could dismiss WezTerm's copy overlay if active. Entering copy mode could trigger WezTerm's copy overlay.
**Used by**: iTerm2 CC mode (sends `copy-mode -q` on connect as defensive cleanup)

## break-pane

**Handler**: `handle_break_pane()` in `handlers.rs:2455`
**What it does now**: Resolves source pane and target session, then returns empty success. Has scaffolding code for the real implementation.
**What it should do**: Move a pane out of its current split into a new tab (optionally in a different workspace/session). With `-d`, keep focus on the original window.
**Used by**: Claude Code team mode (hide pane by breaking to `claude-hidden` session)

## join-pane

**Handler**: Part of `handle_move_pane()` — join-pane is an alias for move-pane.
**Note**: move-pane IS implemented (moves pane between splits), so join-pane may already work. Verify that the `-h -s <pane> -t <window>` form used by Claude Code team mode (show hidden pane) works correctly.
**Used by**: Claude Code team mode (show hidden pane by joining back)

## select-pane -P (style)

**Handler**: `handle_select_pane()` in `handlers.rs:1290`
**What it does now**: The `-P "bg=default,fg=<color>"` style argument is parsed but ignored. Only `-T` (title) and focus are actually applied.
**What it should do**: Could set pane-specific foreground/background colors if WezTerm supports per-pane color overrides.
**Used by**: Claude Code team mode (set pane background/foreground color per agent)

## wait-for

**Handler**: Inline stub in `handlers.rs:707`
**What it does now**: Returns immediately (both `-S` signal and lock/unlock).
**What it should do**: Implement a channel-based wait/signal mechanism. `wait-for <channel>` blocks until another client sends `wait-for -S <channel>`. Could use async channels.
**Used by**: Advanced tmux scripting, synchronization between scripts

## pipe-pane

**Handler**: Inline stub in `handlers.rs:714`
**What it does now**: Returns empty success.
**What it should do**: Pipe pane output to a shell command (or toggle off if no command given). Our CC `%output` notifications partially cover this use case.
**Used by**: Logging/monitoring scripts

## display-popup / display-menu

**Handler**: Inline stub in `handlers.rs:718`
**What it does now**: Returns empty success.
**What it should do**: Show a popup/overlay inside the terminal. Would require WezTerm GUI overlay support.
**Used by**: tmux popup scripts, fzf-tmux

## run-shell (partial)

**Handler**: `handle_run_shell()` in `handlers.rs:847`
**What it does now**: Executes the command and returns stdout, but ignores `-b` (background), `-t` (target pane for output display), and `-d` (delay).
**What it should do**: Support background execution (`-b` flag spawns command without blocking). Support `-d` delay. Route output to target pane if `-t` specified.
**Used by**: tmux scripting, plugin systems

## kill-server

**Handler**: Inline in `handlers.rs:703`
**What it does now**: Sets `ctx.detach_requested = true`, which disconnects the CC client. Does NOT actually kill the WezTerm process.
**What it should do**: Arguably correct behavior — killing the WezTerm GUI from a tmux command would be unexpected. Could optionally close all tmux-managed panes.
**Used by**: Cleanup scripts

---

## Priority for Claude Code Team Mode

**High** (affects visible behavior):
1. `select-layout` — Layout rebalancing after pane creation
2. `break-pane` — Hide/show pane workflow

**Medium** (cosmetic):
3. `set-option` (pane border styles) — Agent color coding
4. `select-pane -P` — Per-pane colors

**Low** (not needed for team mode):
5. `copy-mode`, `wait-for`, `pipe-pane`, `display-popup`, `run-shell -b`, `kill-server`
