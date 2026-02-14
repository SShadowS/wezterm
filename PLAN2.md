# PLAN2.md — Tmux CC Protocol Compatibility Roadmap

**Created**: 2026-02-14
**Status**: Active development — Phase 10 (format string expansion) complete

---

## Current State

### What Works (Phase 1–5 Complete, Phase 6-10 Complete)

- **CC protocol server** running on TCP localhost (Windows) / UDS (Unix)
- **Shim binary** (`tmux-compat-shim`) intercepts `tmux` commands, forwards to CC server
- **Config option** `enable_tmux_compat = true` in `.wezterm.lua`
- **Environment variables** `TMUX`, `WEZTERM_TMUX_CC`, `PATH` set in spawned panes
- **Manual line-buffered I/O** on both server and shim (BufReader breaks Windows sockets)
- **27 commands** implemented and working (16 Phase 1-5 + 7 Phase 7 + 4 Phase 8)
- **9 notifications** emitted (+ Phase 6 lifecycle + Phase 9 `%session-window-changed`, `%paste-buffer-changed`)
- **33 format variables** supported with conditional syntax `#{?cond,true,false}` (20 Phase 1-5 + 13 Phase 10)

### Implemented Commands (16)

| Command | Flags | Notes |
|---------|-------|-------|
| `list-commands` | — | Lists all supported commands |
| `has-session` | `-t` | Check session exists |
| `list-sessions` | `-F` | List workspaces |
| `list-windows` | `-a`, `-F`, `-t` | List tabs |
| `list-panes` | `-a`, `-s`, `-F`, `-t` | List panes |
| `display-message` | `-p`, format | Format string expansion |
| `capture-pane` | `-p`, `-t`, `-e`, `-C`, `-S`, `-E` | Capture pane content |
| `send-keys` | `-t`, `-l`, `-H` | Send keystrokes (named keys, hex, literal) |
| `select-pane` | `-t` | Focus pane |
| `select-window` | `-t` | Activate tab |
| `kill-pane` | `-t` | Close pane |
| `resize-pane` | `-t`, `-x`, `-y` | Resize individual pane |
| `resize-window` | `-t`, `-x`, `-y` | Resize all panes in tab |
| `refresh-client` | `-C`, `-f` | Client resize |
| `split-window` | `-h`, `-v`, `-t`, `-l` | Split pane |
| `new-window` | `-t`, `-n` | Create tab |

### Emitted Notifications (7 active + 2 defined but unused)

| Notification | Status | Source Event |
|-------------|--------|-------------|
| `%session-changed` | Active | Handshake |
| `%window-add` | Active | `TabAddedToWindow` |
| `%window-renamed` | Active | `TabTitleChanged` |
| `%window-pane-changed` | Active | `PaneFocused` |
| `%layout-change` | Active | `TabResized` |
| `%output` | Active | `PaneOutput` (separate path) |
| `%sessions-changed` | Defined | Not emitted |
| `%window-close` | **DEFINED BUT NEVER EMITTED (BUG)** | `WindowRemoved` returns `None` |
| `%exit` | Defined | Not emitted |

### Format Variables (33)

`#{pane_id}`, `#{pane_index}`, `#{pane_width}`, `#{pane_height}`, `#{pane_active}`, `#{pane_left}`, `#{pane_top}`, `#{pane_dead}`, `#{window_id}`, `#{window_index}`, `#{window_name}`, `#{window_active}`, `#{window_width}`, `#{window_height}`, `#{session_id}`, `#{session_name}`, `#{cursor_x}`, `#{cursor_y}`, `#{history_limit}`, `#{history_size}`, `#{version}`, `#{pid}`, `#{client_name}`, `#{socket_path}`, `#{pane_title}`, `#{pane_current_command}`, `#{pane_current_path}`, `#{pane_pid}`, `#{pane_mode}`, `#{window_flags}`, `#{window_panes}`, `#{session_windows}`, `#{session_attached}`

---

## Phase 6: Bug Fixes & Critical Notifications

**Priority**: CRITICAL — fix before adding new features
**Status**: [x] Complete

### Verification Notes (from tmux + iTerm2 source analysis)

**tmux source** (`control-notify.c`):
- `%window-close @<id>` — emitted by `control_notify_window_unlinked()`, also has `%unlinked-window-close @<id>` variant for windows not in the client's session
- `%session-renamed $<id> <name>` — emitted by `control_notify_session_renamed()`, sent to ALL control clients
- `%sessions-changed` — no args, emitted by both `control_notify_session_created()` and `control_notify_session_closed()`

**iTerm2** (`TmuxGateway.m`, `TmuxController.m`):
- Handles both `%window-close` and `%unlinked-window-close` via regex `^%(?:unlinked-)?window-close @([0-9]+)$`
- `%session-renamed` → updates local session name, fires NSNotification (no response to tmux)
- `%sessions-changed` → sends `list-sessions -F "#{session_id} #{session_name}"` after 1.5s debounce

### 6.1 — Emit `%window-close` notification

- [x] **Bug**: `window_close_notification()` exists in `response.rs` but `translate_notification()` returns `None` for `WindowRemoved`
- [x] Add mux_window→tabs tracking to `IdMap` (required — not optional): `track_tab_in_window()`, `tabs_in_mux_window()`, `mux_window_workspace()`
- [x] In `translate_notification()`, handle `MuxNotification::WindowRemoved(window_id)`:
  - Look up tabs tracked for that mux window via id_map
  - Emit `%window-close @<tmux_wid>` for each tab
  - Clean up id_map entries for removed tabs
  - Check if workspace still has windows; if not, also emit `%sessions-changed`
- [x] Track mux_window→workspace on `WindowCreated` and `TabAddedToWindow`
- **Note**: tmux also emits `%unlinked-window-close` for windows not in client's session; we only emit `%window-close` since our CC client is always in one session
- **Files**: `server.rs`, `id_map.rs`
- **Difficulty**: Medium

### 6.2 — Emit `%session-renamed` notification

- [x] Add `session_renamed_notification(session_id, new_name)` to `response.rs` (was completely missing)
- [x] In `translate_notification()`, handle `MuxNotification::WorkspaceRenamed { old_workspace, new_workspace }`
- [x] Re-key id_map session mapping: preserve tmux session ID, update workspace name
- [x] Format: `%session-renamed $<id> <new_name>` (matches tmux's `%%session-renamed $%u %s`)
- **Files**: `response.rs`, `server.rs`, `id_map.rs`
- **Difficulty**: Easy

### 6.3 — Emit `%sessions-changed` notification

- [x] Already defined in `response.rs` but never emitted
- [x] Emit on workspace creation: `WindowCreated` → check if window's workspace is new (not in id_map)
- [x] Emit on workspace destruction: `WindowRemoved` → check if workspace has no remaining windows
- [x] **Corrected trigger**: `ActiveWorkspaceChanged` is wrong signal (tracks focus, not list changes). Use `WindowCreated`/`WindowRemoved` instead.
- [x] **iTerm2 follow-up**: After receiving `%sessions-changed`, iTerm2 sends `list-sessions -F "#{session_id} #{session_name}"` — our existing `list-sessions` command handles this.
- **Files**: `server.rs`, `id_map.rs`
- **Difficulty**: Easy–Medium

---

## Phase 7: High-Priority Missing Commands

**Priority**: HIGH — needed for basic tmux workflow compatibility. iTerm2 uses ALL 7 commands.
**Status**: [x] Complete

### Verification Notes (from tmux + iTerm2 source analysis)

**tmux aliases**: `killw`=kill-window, `rename`=rename-session, `renamew`=rename-window, `new`=new-session, `show`=show-options, `showw`=show-window-options, `resizep`=resize-pane

**iTerm2 sends**: `kill-window -t @%d`, `kill-session -t "$%d"`, `new-session -s "%@"`, `show-option -g -v status`, `show-option -q -g -v focus-events`, `show-options -v -s default-terminal`, `show-window-options -g aggressive-resize`, `resize-pane -Z -t "%%%d"`, `rename-window -t @%d "%@"`, `rename-session -t "$%d" "%@"`

### 7.1 — `kill-window` command

- [x] Add `KillWindow { target: Option<String> }` to `TmuxCliCommand`
- [x] Parse `-t <target>` flag (alias: `killw`)
- [x] Handler: resolve target → `mux.kill_window(window_id)`
- [x] `%window-close` emitted automatically by Phase 6 `WindowRemoved` handler
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Easy

### 7.2 — `kill-session` command

- [x] Add `KillSession { target: Option<String> }` to `TmuxCliCommand`
- [x] Parse `-t <target>` flag
- [x] Handler: resolve target → workspace, iterate `iter_windows_in_workspace()`, kill each window
- [x] `%sessions-changed` emitted automatically when last window removed
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Medium

### 7.3 — `new-session` command

- [x] Add `NewSession { name: Option<String>, window_name: Option<String>, detached: bool }` to `TmuxCliCommand`
- [x] Parse `-s <name>`, `-n <window-name>`, `-d` (detached) flags (alias: `new`)
- [x] Handler: create new workspace via `mux.spawn_tab_or_window()` with new workspace name
- [x] `%sessions-changed` emitted automatically by Phase 6 `WindowCreated` handler
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Medium

### 7.4 — `show-options` / `show-window-options` commands

- [x] Add `ShowOptions` and `ShowWindowOptions` as separate `TmuxCliCommand` variants
- [x] Parse `-g`, `-v`, `-q`, `-s`, `-t`, option name (aliases: `show`, `show-option`; `showw`, `show-window-option`)
- [x] Handler: return hardcoded defaults for known options, empty string for unknown (with `-q`)
- [x] iTerm2 init queries handled: `status`→`off`, `focus-events`→`on`, `default-terminal`→`screen-256color`, `set-titles`→`on`, `aggressive-resize`→`off`, `pane-border-format`→empty
- [x] Output format: `name value` (normal) or just `value` (with `-v`)
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Medium

### 7.5 — `resize-pane -Z` (zoom toggle)

- [x] Add `zoom: bool` flag to existing `ResizePane` variant
- [x] Parse `-Z` flag; `-Z` causes early return (other resize flags ignored, matching tmux behavior)
- [x] Handler: resolve target → `tab.toggle_zoom()`
- [x] `%layout-change` emitted via `TabResized` notification
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Easy

### 7.6 — `rename-window` command

- [x] Add `RenameWindow { target: Option<String>, name: String }` to `TmuxCliCommand`
- [x] Parse `-t <target>` and positional name argument (alias: `renamew`)
- [x] Handler: `tab.set_title(name)`
- [x] `%window-renamed` emitted via `TabTitleChanged` notification
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Easy

### 7.7 — `rename-session` command

- [x] Add `RenameSession { target: Option<String>, name: String }` to `TmuxCliCommand`
- [x] Parse `-t <target>` and positional name argument (alias: `rename`)
- [x] Handler: `mux.rename_workspace(old, new)` with duplicate name validation
- [x] `%session-renamed` emitted via Phase 6 `WorkspaceRenamed` notification
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Easy

---

## Phase 8: Session Lifecycle & Client Management

**Priority**: HIGH — needed for multi-session workflows. iTerm2 uses attach-session, detach, and list-clients.
**Status**: [x] Complete

### Verification Notes (from tmux + iTerm2 source analysis)

**CORRECTION**: tmux has NO `detach-session` command. Only `detach-client` (alias: `detach`) exists.
**CORRECTION**: iTerm2 does NOT use `switch-client`; it uses `attach-session -t "$N"` for session switching instead.

**tmux aliases**: `attach`=attach-session, `detach`=detach-client, `switchc`=switch-client, `lsc`=list-clients

**iTerm2 sends**: `attach-session -t "$N"` (session switch), `detach` (disconnect), `list-clients -t '$N' -F '#{client_name}\t#{client_control_mode}'` (multi-client tracking)

**Notifications**: `attach-session` triggers `%session-changed` to self. `detach-client` closes the CC connection. Both trigger `%client-session-changed`/`%client-detached` to OTHER CC clients (not implemented yet — deferred to multi-client phase).

### 8.1 — `attach-session` command

- [x] Add `AttachSession { target: Option<String> }` to `TmuxCliCommand`
- [x] Parse `-t <target>` flag (alias: `attach`)
- [x] Handler: resolve target workspace → update `ctx.workspace`, update active session/window/pane IDs
- [x] Return `%session-changed` notification content (server sends it after command response)
- [x] iTerm2 sends: `attach-session -t "$N"` where N is session number
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Medium

### 8.2 — `detach-client` command

- [x] Add `DetachClient` to `TmuxCliCommand`
- [x] Parse as bare command (alias: `detach`; flags `-t`, `-s`, `-a`, `-P` accepted but ignored for single-client CC)
- [x] Handler: return special sentinel value that signals connection close
- [x] Server loop detects sentinel → sends `%exit` notification → closes connection
- [x] iTerm2 sends: bare `detach` command
- **Files**: `command_parser.rs`, `handlers.rs`, `server.rs`
- **Difficulty**: Medium

### 8.3 — `switch-client` command

- [x] Add `SwitchClient { target: Option<String> }` to `TmuxCliCommand`
- [x] Parse `-t <target>` flag (alias: `switchc`); `-n`, `-p`, `-l` flags accepted but not implemented
- [x] Handler: same as attach-session (switch workspace)
- [x] LOW priority (iTerm2 doesn't use it) but easy to implement alongside attach-session
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Easy (reuses attach-session logic)

### 8.4 — `list-clients` command

- [x] Add `ListClients { format: Option<String>, target: Option<String> }` to `TmuxCliCommand`
- [x] Parse `-F <format>`, `-t <target>` flags (alias: `lsc`)
- [x] Handler: return single CC client entry with hardcoded format variables
- [x] Default format: `#{client_name}: #{session_name} [#{client_width}x#{client_height} #{client_termname}]`
- [x] iTerm2 queries: `list-clients -t '$N' -F '#{client_name}\t#{client_control_mode}'`
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Medium

### 8.5 — Session change notification mechanism

- [x] `attach-session` handler queues `%session-changed` via `ctx.pending_notifications`
- [x] Server loop drains `pending_notifications` after writing command response
- [x] Server loop checks `detach_requested` → sends `%exit` → closes connection
- [x] `ActiveWorkspaceChanged` remains in ignore arm (not needed — handler queues directly)
- **Files**: `handlers.rs`, `server.rs`
- **Difficulty**: Easy

---

## Phase 9: Missing Notifications

**Priority**: MEDIUM — improve sync fidelity with CC clients
**Status**: [x] Complete

### Verification Notes (from tmux + iTerm2 source analysis)

**`%pane-mode-changed`**: Triggered by ANY pane mode enter/exit in tmux (copy-mode, view-mode, tree-mode, etc.). iTerm2 **explicitly ignores** it: `"New in tmux 2.5. Don't care."`. WezTerm has no pane mode infrastructure. **Deferred to Phase 12.**

**`%session-window-changed`**: Format `$<sid> @<wid>`. Emitted by tmux `session_set_current()`. iTerm2 actively handles it with a suppression counter (`_ignoreWindowChangeNotificationCount`) to prevent feedback loops.

**`%paste-buffer-changed`**: Format includes buffer name: `%paste-buffer-changed buffer0`. tmux also has `%paste-buffer-deleted` (separate). iTerm2 validates `buffer[0-9]+` for security, syncs clipboard if preference enabled.

**`%client-session-changed`**: Dual format in tmux — switching client gets `%session-changed` (already implemented Phase 8), OTHER clients get `%client-session-changed <name> $<sid> <name>`. Single-client CC = already done.

**`%client-detached`**: Sent to OTHER remaining clients only, NOT to the disconnecting client. Single-client CC = no recipients. Deferred to multi-client phase.

### 9.1 — `%pane-mode-changed` — DEFERRED

- [x] Deferred to Phase 12 (copy-mode commands)
- iTerm2 explicitly ignores this notification
- WezTerm has no Pane trait mode tracking — requires architecture changes
- Format: `%pane-mode-changed %<pane_id>`

### 9.2 — `%session-window-changed`

- [x] Add `session_window_changed_notification(session_id, window_id)` to `response.rs`
- [x] Track last-known active tab per mux window in `HandlerContext`
- [x] On `WindowInvalidated`: compare current active tab vs prior, emit if changed
- [x] Feedback-loop suppression: increment counter on `select-window`, skip notification while > 0
- Format: `%session-window-changed $<session_id> @<window_id>`
- **Files**: `response.rs`, `server.rs`, `handlers.rs`
- **Difficulty**: Medium

### 9.3 — `%paste-buffer-changed`

- [x] Add `paste_buffer_changed_notification(buffer_name)` to `response.rs`
- [x] Translate `MuxNotification::AssignClipboard` → `%paste-buffer-changed buffer0` in `server.rs`
- [x] Use synthetic buffer name `buffer0` (single paste buffer)
- Format: `%paste-buffer-changed <buffer_name>`
- **Files**: `response.rs`, `server.rs`
- **Difficulty**: Easy

### 9.4 — `%client-session-changed` — ALREADY DONE

- [x] Single-client CC: switching client already receives `%session-changed` via Phase 8 `pending_notifications`
- [x] Multi-client `%client-session-changed` deferred (no other clients to notify)
- Format: `%client-session-changed <client_name> $<session_id> <session_name>`

### 9.5 — `%client-detached` — DEFERRED

- [x] Deferred to multi-client phase
- Only sent to OTHER remaining clients, not to the disconnecting client
- Single-client CC has no other clients to notify
- Format: `%client-detached <client_name>`

---

## Phase 10: Format String Expansion

**Priority**: HIGH — iTerm2 queries `#{version}`, `#{pid}`, `#{client_name}`, `#{window_flags}`, `#{pane_title}`, `#{pane_current_command}`, `#{pane_current_path}`, `#{pane_pid}` during initialization and runtime
**Status**: [x] Complete

### Verification Notes (from tmux + iTerm2 source analysis)

**tmux source** (`format.c`): 165 format variables total. All return strings (numbers formatted as strings). Variables are scoped: global, client, session, window, pane.

**iTerm2 usage** (`TmuxController.m`, `iTermTmuxOptionMonitor.m`):
- `#{version}` — `display-message -p "#{version}"` for feature detection. **CRITICAL**.
- `#{pid}` — `display-message -p "#{pid}"` for version detection (tmux 2.1+). **CRITICAL**.
- `#{client_name}` — `display-message -p '#{client_name}'` for client identification. **REQUIRED**.
- `#{window_flags}` — in `list-windows -F "..."` for active/zoomed state. **CRITICAL**.
- `#{pane_title}` — monitored via `iTermTmuxOptionMonitor` for session titles. **REQUIRED**.
- `#{pane_current_command}` — monitored via option monitor for job name tracking. **REQUIRED**.
- `#{pane_current_path}` — used in `new-window -c '#{pane_current_path}'` for directory recycling. **REQUIRED**.
- `#{pane_pid}` — `display-message -t '<%id>' -p '#{pane_pid}'` for process tracking. **IMPORTANT**.
- `#{socket_path}` — version detection only (tmux 2.2+). Low priority.
- `#{pane_mode}`, `#{window_panes}`, `#{session_windows}`, `#{session_attached}` — **not used** by iTerm2. Low priority (script/libtmux compat).

**WezTerm APIs available**: All 13 variables implementable with existing Pane trait methods (`get_title()`, `get_current_working_dir()`, `get_foreground_process_name()`, `get_foreground_process_info()`), Tab methods (`count_panes()`), and Mux methods (`iter_windows_in_workspace()`).

### Missing Format Variables (priority-corrected)

| Variable | Value | Priority | iTerm2? | WezTerm Source |
|----------|-------|----------|---------|----------------|
| `#{version}` | `"3.3a"` | **Critical** | Yes — version detection | Hardcode |
| `#{pid}` | Process ID | **Critical** | Yes — version detection | `std::process::id()` |
| `#{client_name}` | Client identifier | **High** | Yes — client identification | Hardcode per connection |
| `#{window_flags}` | Flag chars (`*-Z#!~M`) | **High** | Yes — active/zoomed state | Combine: `*`=active, `-`=last, `Z`=zoomed |
| `#{pane_title}` | Pane title string | **High** | Yes — title monitoring | `pane.get_title()` |
| `#{pane_current_command}` | Running command name | **High** | Yes — job name tracking | `pane.get_foreground_process_name()` |
| `#{pane_current_path}` | Current directory | **High** | Yes — directory recycling | `pane.get_current_working_dir()` |
| `#{pane_pid}` | Pane shell process ID | **Medium** | Yes — process tracking | `pane.get_foreground_process_info()` |
| `#{socket_path}` | CC socket/address | **Low** | Version detection only | Thread through context |
| `#{pane_mode}` | `""` (no mode infra) | **Low** | No | Hardcode `""` |
| `#{window_panes}` | Number of panes | **Low** | No | `tab.count_panes()` |
| `#{session_windows}` | Number of windows | **Low** | No | `mux.iter_windows_in_workspace().len()` |
| `#{session_attached}` | Number of clients | **Low** | No | Hardcode `1` (single-client CC) |

### Implementation

- [x] 10.1 — Add trivial global variables: `version`, `pid`, `client_name`, `pane_mode`, `session_attached`
- [x] 10.2 — Add pane data variables: `pane_title`, `pane_current_command`, `pane_current_path`, `pane_pid`
- [x] 10.3 — Add computed variables: `window_flags`, `window_panes`, `session_windows`
- [x] 10.4 — Add `socket_path` (thread listen address through context)
- **Files**: `format.rs`, `handlers.rs`
- **Difficulty**: Easy per variable; all use existing APIs

---

## Phase 11: Clipboard / Buffer Commands

**Priority**: LOW — nice-to-have for full compatibility
**Status**: [ ] Not started

### Commands

- [ ] `list-buffers` — list clipboard buffers
- [ ] `show-buffer` / `show-buffer -b <name>` — show buffer content
- [ ] `set-buffer` — set buffer content
- [ ] `paste-buffer` — paste to pane
- [ ] `delete-buffer` — remove buffer

### Notes

- WezTerm uses system clipboard; may need a buffer abstraction layer
- iTerm2 uses these for clipboard integration between tmux and the GUI
- **Difficulty**: Hard (needs clipboard integration design)

---

## Phase 12: Advanced / Modern tmux (3.2+) Features

**Priority**: LOW — for future compatibility
**Status**: [ ] Not started

- [ ] `%pause` / `%continue` — flow control for output
- [ ] `%extended-output` — output with latency info
- [ ] `%subscription-changed` — subscription-based notifications
- [ ] `refresh-client -f pause-after=N,wait-exit` — pause mode flags
- [ ] `copy-mode` / `copy-mode -q` — enter/exit copy mode
- [ ] `move-pane` / `move-window` — reorganize layout
- [ ] `display-menu` / `display-popup` — UI overlays
- [ ] Persistent ID mapping across reconnects

---

## Architecture Notes

### File Map

| File | Purpose |
|------|---------|
| `mux/src/tmux_compat_server/mod.rs` | Module declarations |
| `mux/src/tmux_compat_server/server.rs` | TCP/UDS listener, connection loop, handshake, notification translation |
| `mux/src/tmux_compat_server/command_parser.rs` | `parse_command()` → `TmuxCliCommand` enum |
| `mux/src/tmux_compat_server/handlers.rs` | `dispatch_command()` → executes against Mux |
| `mux/src/tmux_compat_server/response.rs` | `ResponseWriter` + notification formatting functions |
| `mux/src/tmux_compat_server/format.rs` | `expand_format()` + `FormatContext` |
| `mux/src/tmux_compat_server/id_map.rs` | WezTerm ID ↔ tmux ID mapping |
| `mux/src/tmux_compat_server/layout.rs` | Layout string generation |
| `mux/src/tmux_compat_server/target.rs` | Target resolution (`-t %3`, `-t @1`, etc.) |
| `tmux-compat-shim/src/main.rs` | CLI shim binary |
| `config/src/config.rs` | `enable_tmux_compat` config field |
| `mux/src/domain.rs` | Env var setup (`TMUX`, `WEZTERM_TMUX_CC`, `PATH`) |
| `wezterm-gui/src/main.rs` | Server startup integration |

### Key Patterns

- **Command dispatch**: `parse_command()` → `dispatch_command()` (async, non-Send) → main thread via `spawn_into_main_thread` + `spawn` + channel
- **Handshake**: Built synchronously on connection thread using `Mux::try_get()` (global Arc, works from any thread)
- **I/O**: Raw `read()`/`write_all()` with manual line accumulation — no `BufReader` (breaks Windows sockets)
- **Transport**: TCP localhost on Windows (`127.0.0.1:0`), UDS on Unix
- **ID mapping**: Monotonically increasing counters per type (sessions $0.., windows @0.., panes %0..)

### Reference Code

- **tmux source**: `U:\Git\tmux\` — CC protocol, command behavior, format strings
- **iTerm2 source**: `U:\Git\iTerm2\` — `TmuxGateway.m` (CC client), `TmuxController.m` (command dispatch), tmux Python API

---

## Compatibility Target

We aim to be compatible with tools that use tmux CC mode, including:
1. **iTerm2** tmux integration (primary reference)
2. **Python tmux API** (libtmux, tmuxp)
3. **Shell scripts** that call `tmux list-sessions`, `tmux send-keys`, etc.
4. **IDE integrations** that query tmux for pane/session info

Claimed version: **tmux 3.3a** (via `tmux -V` output from shim)
