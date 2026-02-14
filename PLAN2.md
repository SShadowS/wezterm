# PLAN2.md — Tmux CC Protocol Compatibility Roadmap

**Created**: 2026-02-14
**Status**: Active development — Phases 1-12.5 complete (all Phase 12 sub-phases done)

---

## Current State

### What Works (Phase 1–5 Complete, Phase 6-11 Complete)

- **CC protocol server** running on TCP localhost (Windows) / UDS (Unix)
- **Shim binary** (`tmux-compat-shim`) intercepts `tmux` commands, forwards to CC server
- **Config option** `enable_tmux_compat = true` in `.wezterm.lua`
- **Environment variables** `TMUX`, `WEZTERM_TMUX_CC`, `PATH` set in spawned panes
- **Manual line-buffered I/O** on both server and shim (BufReader breaks Windows sockets)
- **36 commands** implemented and working (16 Phase 1-5 + 7 Phase 7 + 4 Phase 8 + 5 Phase 11 + 3 Phase 12.3 + 1 Phase 12.4)
- **10 notifications** emitted (+ Phase 6 lifecycle + Phase 9 `%session-window-changed`, `%paste-buffer-changed` + Phase 11 `%paste-buffer-deleted`)
- **36 format variables** supported with conditional syntax `#{?cond,true,false}` (20 Phase 1-5 + 13 Phase 10 + 3 Phase 11)

### Implemented Commands (32)

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
| `attach-session` | `-t` | Attach to session (Phase 7) |
| `detach-client` | — | Detach client (Phase 7) |
| `kill-session` | `-t` | Kill session (Phase 7) |
| `kill-window` | `-t` | Kill window (Phase 7) |
| `list-clients` | `-F` | List clients (Phase 7) |
| `new-session` | `-s`, `-n`, `-x`, `-y` | Create session (Phase 7) |
| `switch-client` | `-t` | Switch session (Phase 7) |
| `rename-session` | `-t`, name | Rename session (Phase 8) |
| `rename-window` | `-t`, name | Rename window (Phase 8) |
| `show-options` | `-g`, `-s`, name | Show options (Phase 8) |
| `show-window-options` | `-g`, name | Show window options (Phase 8) |
| `show-buffer` | `-b` | Show buffer content (Phase 11) |
| `set-buffer` | `-b`, `-a`, data | Set/append buffer (Phase 11) |
| `delete-buffer` | `-b` | Delete buffer (Phase 11) |
| `list-buffers` | `-F` | List buffers (Phase 11) |
| `paste-buffer` | `-b`, `-t`, `-d`, `-p` | Paste buffer to pane (Phase 11) |
| `move-pane` | `-s`, `-t`, `-h`, `-v`, `-b` | Move pane between split trees (Phase 12.3) |
| `join-pane` | `-s`, `-t`, `-h`, `-v`, `-b` | Alias for move-pane (Phase 12.3) |
| `move-window` | `-s`, `-t` | Move tab between windows (Phase 12.3) |
| `copy-mode` | `-q`, `-t` | Exit/enter copy mode — no-op (Phase 12.4) |

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

### Format Variables (36)

`#{pane_id}`, `#{pane_index}`, `#{pane_width}`, `#{pane_height}`, `#{pane_active}`, `#{pane_left}`, `#{pane_top}`, `#{pane_dead}`, `#{window_id}`, `#{window_index}`, `#{window_name}`, `#{window_active}`, `#{window_width}`, `#{window_height}`, `#{session_id}`, `#{session_name}`, `#{cursor_x}`, `#{cursor_y}`, `#{history_limit}`, `#{history_size}`, `#{version}`, `#{pid}`, `#{client_name}`, `#{socket_path}`, `#{pane_title}`, `#{pane_current_command}`, `#{pane_current_path}`, `#{pane_pid}`, `#{pane_mode}`, `#{window_flags}`, `#{window_panes}`, `#{session_windows}`, `#{session_attached}`, `#{buffer_name}`, `#{buffer_size}`, `#{buffer_sample}`

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

**Priority**: MEDIUM — `show-buffer` is required for iTerm2 clipboard sync
**Status**: [x] Complete

### Verification Notes (from tmux + iTerm2 source analysis)

**tmux buffer model** (`paste.c`, `paste.h`):
- Named paste buffer stack: up to 50 auto-named (`buffer0`..`buffer49`) + unlimited user-named
- Fields: `name`, `data` (raw bytes, not null-terminated), `size`, `created`, `automatic`, `order`
- Two red-black trees: by name and by insertion order (newest first)
- `buffer-limit` option (default 50) applies only to automatic buffers
- Notifications: `%paste-buffer-changed <name>` and `%paste-buffer-deleted <name>` (from `control-notify.c`)

**iTerm2 usage** (`TmuxController.m`, `TmuxGateway.m`):
- **Only `show-buffer` is used** — fetches buffer contents for clipboard sync
- Flow: `%paste-buffer-changed buffer0` → validate name matches `buffer[0-9]+` → `show-buffer -b buffer0` → set macOS pasteboard
- Clipboard sync is **opt-in** (disabled by default, user prompted on first notification)
- `list-buffers` is in iTerm2's `forbiddenCommands` array (blocked from user keybindings, not used internally)
- `set-buffer`, `paste-buffer`, `delete-buffer`: **not used at all**

**WezTerm current state**:
- `%paste-buffer-changed buffer0` already emitted on `AssignClipboard`
- `pane.send_paste()` available for paste-buffer (handles bracketed paste automatically)
- `pane.writer()` available for raw PTY input
- No in-process buffer store — only system clipboard

**Architecture**: In-process `PasteBufferStore` in `HandlerContext`, storing named buffers with auto-naming. `buffer0` synced from `AssignClipboard` notifications. No system clipboard write-back needed (WezTerm GUI handles that separately).

### 11.1 — Paste buffer store

- [x] Add `PasteBufferStore` struct to new `paste_buffer.rs` module
- [x] Fields per buffer: `name: String`, `data: String`, `automatic: bool`, `order: u64`
- [x] Methods: `set()`, `get()`, `delete()`, `list()`, `get_most_recent()`, `rename()`
- [x] Auto-naming: `buffer0`, `buffer1`, ... with monotonic counter
- [x] Buffer limit: hardcode 50 for automatic buffers; evict oldest when exceeded
- [x] Add `paste_buffers: PasteBufferStore` field to `HandlerContext`
- [x] On `AssignClipboard` notification: store content as `buffer0` (or next auto-name)
- **Files**: `paste_buffer.rs` (new), `handlers.rs`, `server.rs`, `mod.rs`
- **Difficulty**: Medium

### 11.2 — `show-buffer` command (HIGH — iTerm2 clipboard sync)

- [x] Add `ShowBuffer { buffer_name: Option<String> }` to `TmuxCliCommand`
- [x] Parse `-b <buffer-name>` flag (alias: `showb`)
- [x] Handler: resolve buffer name → return buffer content as raw text
- [x] Default (no `-b`): return most recent automatic buffer
- [x] Error: `"no buffers"` if store empty, `"unknown buffer: <name>"` if not found
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Easy

### 11.3 — `set-buffer` command (LOW — not used by iTerm2)

- [x] Add `SetBuffer { buffer_name: Option<String>, data: Option<String>, append: bool }` to `TmuxCliCommand`
- [x] Parse `-b <name>`, `-a` (append), positional data (alias: `setb`)
- [x] Handler: create or update buffer in store; emit `%paste-buffer-changed`
- [x] Auto-name if no `-b` specified
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Easy

### 11.4 — `delete-buffer` command (LOW — not used by iTerm2)

- [x] Add `DeleteBuffer { buffer_name: Option<String> }` to `TmuxCliCommand`
- [x] Parse `-b <buffer-name>` flag (alias: `deleteb`)
- [x] Handler: remove buffer from store; emit `%paste-buffer-deleted`
- [x] Default: delete most recent automatic buffer
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Easy

### 11.5 — `list-buffers` command (LOW — not used by iTerm2)

- [x] Add `ListBuffers { format: Option<String> }` to `TmuxCliCommand`
- [x] Parse `-F <format>` flag (alias: `lsb`)
- [x] Handler: iterate store, expand format per buffer
- [x] Default format: `#{buffer_name}: #{buffer_size} bytes: "#{buffer_sample}"`
- [x] Add `buffer_name`, `buffer_size`, `buffer_sample` to `FormatContext`
- **Files**: `command_parser.rs`, `handlers.rs`, `format.rs`
- **Difficulty**: Easy

### 11.6 — `paste-buffer` command (LOW — not used by iTerm2)

- [x] Add `PasteBuffer { buffer_name: Option<String>, target: Option<String>, delete: bool, bracketed: bool }` to `TmuxCliCommand`
- [x] Parse `-b <name>`, `-t <target>`, `-d` (delete after), `-p` (bracketed paste) (alias: `pasteb`)
- [x] Handler: get buffer content → `pane.send_paste()` (handles bracketed paste); `-d` → delete buffer
- [x] Note: tmux default line separator is `\r`; `pane.send_paste()` already handles line ending conversion
- **Files**: `command_parser.rs`, `handlers.rs`
- **Difficulty**: Easy

---

## Phase 12: Advanced / Modern tmux (3.2+) Features

**Priority**: MIXED — pause mode is CRITICAL, others vary
**Status**: [ ] Not started

### Verification Notes (from tmux + iTerm2 source analysis)

**tmux source** (`control.c`, `control-notify.c`, `cmd-refresh-client.c`):
- Pause mode introduced in tmux 3.2: `%pause %<pane>`, `%continue %<pane>`, `%extended-output`
- `control_pause_pane()` sets `CONTROL_PANE_PAUSED` flag, discards buffered data, sends notification
- `%extended-output` replaces `%output` when `CLIENT_CONTROL_PAUSEAFTER` flag is set — format: `%extended-output %<pane_id> <age_ms> : <vis_data>`
- Without pause-after, tmux force-disconnects clients >300s behind (`CONTROL_MAXIMUM_AGE`)
- Subscriptions: `refresh-client -B <name>:<target>:<format>` registers format watch, 1s polling interval, change detection via `last` value
- `%subscription-changed <name> $<session> @<window> <index> %<pane> : <value>`
- Additional notifications not in original plan: `%pane-mode-changed`, `%client-session-changed`, `%client-detached`, `%config-error`, `%message`, `%unlinked-window-*`

**iTerm2 usage** (`TmuxGateway.m`, `TmuxController.m`, `iTermTmuxBufferSizeMonitor.m`):
- **Pause mode**: CRITICAL — `enablePauseModeIfPossible` sends `refresh-client -fpause-after=<N>` on connect (tmux 3.2+)
- **%extended-output**: CRITICAL — `parseExtendedOutputCommandData:` extracts latency, feeds to `iTermTmuxBufferSizeMonitor` which uses linear regression on latency data points
- **%pause/%continue**: Handled — `parsePauseCommand:` triggers `tmuxWindowPaneDidPause:`, `%continue` explicitly ignored ("Don't care")
- **Subscriptions**: Actively used via `iTermTmuxOptionMonitor` with graceful fallback to `display-message` polling for tmux < 3.2
- **copy-mode -q**: Only used defensively to exit copy mode after tmux.conf errors (not for UI)
- **move-pane/move-window**: Actively used for layout reorganization
- **display-menu/display-popup**: Listed in `forbiddenCommands` — intentionally excluded, not CC protocol features
- **%pane-mode-changed**: Explicitly ignored ("Don't care")
- **Version gating**: 3.2 (pause mode, subscriptions), 3.6 (OSC queries, clipboard), 2.9 (variable window sizes)

**WezTerm current state**:
- No pause/resume mechanism for PTY output — `read_from_pane_pty` continuously reads with no throttle
- Full copy mode overlay exists (`wezterm-gui/src/overlay/copy.rs` ~2000 lines) — just needs tmux command bridge
- `move_pane_to_new_tab()` exists but no between-tab pane movement
- `Mux::subscribe()` callback system exists for notifications but no option-change events
- IdMap is pure in-memory HashMap, no serialization

### 12.1 — Pause Mode & Flow Control (CRITICAL)

**Priority**: CRITICAL — prevents unbounded memory growth, required by iTerm2
**Status**: [x] Complete

iTerm2 sends `refresh-client -fpause-after=<N>` immediately after connecting to tmux 3.2+. The pause-after flag enables `%extended-output` (with latency) instead of `%output`. When a pane's buffered output exceeds N seconds of age, tmux pauses the pane and sends `%pause %<pane_id>`. The client resumes with `refresh-client -A %<pane>:continue`.

- [x] Add `pause_age_ms: Option<u64>`, `wait_exit: bool`, `paused_panes`, `pane_output_timestamps` to `HandlerContext`
- [x] Parse `refresh-client -f pause-after=N,wait-exit` and `refresh-client -f !pause-after`
- [x] Track per-pane pause state and output age in `HandlerContext`
- [x] When `pause_age` is set, emit `%extended-output %<pane_id> <age_ms> : <vis_data>` instead of `%output`
- [x] When age exceeds `pause_age`, set pane paused flag, emit `%pause %<pane_id>`, stop forwarding output
- [x] Parse `refresh-client -A %<pane>:continue` / `refresh-client -A %<pane>:pause` / `refresh-client -A %<pane>:on` / `refresh-client -A %<pane>:off`
- [x] Add `pause_notification()`, `continue_notification()`, `extended_output_notification()` to `response.rs`
- [x] Add raw output tap infrastructure in `mux/src/lib.rs` (`register_output_tap`, `notify_output_taps`)
- [x] Wire `%output`/`%extended-output` forwarding in CC connection loop via output tap receivers
- **Files**: `mux/src/lib.rs`, `handlers.rs`, `server.rs`, `response.rs`, `command_parser.rs`
- **Difficulty**: Medium-High (core output pipeline changes, per-pane timing state)

### 12.2 — Subscription Notifications (HIGH)

**Priority**: HIGH — iTerm2 actively uses for efficient format monitoring
**Status**: [x] Complete

Subscriptions eliminate polling overhead. iTerm2's `iTermTmuxOptionMonitor` uses subscriptions when available (tmux 3.2+), falls back to periodic `display-message` calls otherwise.

- [x] Add `Subscription { name, target, format, last_values }` struct with `SubscriptionTarget` enum
- [x] Add `subscriptions: Vec<Subscription>` to `HandlerContext`
- [x] Parse `refresh-client -B <name>:<target>:<format>` to register subscriptions
- [x] Parse `refresh-client -B <name>` to unsubscribe (remove by name)
- [x] Add periodic check (1s interval) via `check_subscriptions()` in CC connection loop
- [x] Only emit notification when value changes (compare with `last_values` map)
- [x] Wire format: `%subscription-changed <name> $<session_id> @<window_id> <window_index> %<pane_id> : <value>`
- [x] Target types: `$<session>`, `@<window>`, `%<pane>`, `%*` (all panes), `@*` (all windows)
- [x] Add `subscription_changed_notification()` to `response.rs`
- [x] 13 new tests (10 handler + 2 parser + 1 response)
- **Files**: `handlers.rs`, `server.rs`, `response.rs`, `command_parser.rs`
- **Difficulty**: Medium (timer integration, format evaluation per subscription)

### 12.3 — Move Commands (MEDIUM)

**Priority**: MEDIUM — iTerm2 actively uses both for layout reorganization
**Status**: [x] Complete

- [x] Add `MovePane { src, dst, horizontal, before }` to `TmuxCliCommand`
- [x] Parse `move-pane -s <src> -t <dst> [-h|-v] [-b] [-d] [-f] [-l] [-p]` (aliases: `movep`, `join-pane`, `joinp`)
- [x] Handler: uses `Mux::split_pane()` with `SplitSource::MovePane` — removes pane from source split tree, inserts into destination
- [x] Add `MoveWindow { src, dst }` to `TmuxCliCommand`
- [x] Parse `move-window -s <src> -t <dst> [-a] [-b] [-d] [-k] [-r]` (alias: `movew`)
- [x] Handler: removes tab from source mux Window, pushes to destination mux Window
- [x] Add `join-pane`, `move-pane`, `move-window` to `handle_list_commands` (32→35 commands)
- [x] 8 new tests (5 parser + 3 handler list)
- **Files**: `command_parser.rs`, `handlers.rs`
- **Note**: Uses `Mux::split_pane(SplitSource::MovePane)` from `mux/src/lib.rs` which handles remove+insert+cleanup

### 12.4 — Copy Mode Bridge (LOW)

**Priority**: LOW — iTerm2 only uses `copy-mode -q` defensively
**Status**: [x] Complete

- [x] Add `CopyMode { quit: bool, target: Option<String> }` to `TmuxCliCommand`
- [x] Parse `copy-mode [-q] [-t target] [-e] [-H] [-M] [-u]`
- [x] Handler: no-op success (WezTerm manages its own copy overlay independently)
- [x] Add `copy-mode` to `handle_list_commands` (35→36 commands)
- [x] 5 new tests (3 parser + 2 handler)
- **Files**: `command_parser.rs`, `handlers.rs`
- **Note**: iTerm2 sends `copy-mode -q` on connect as defensive measure (tmux issue #3193)

### 12.5 — Persistent ID Mapping (MEDIUM) ✅

**Priority**: MEDIUM — enables session recovery across reconnects
**Status**: [x] Complete

- [x] Add `serde_json` dependency to `mux/Cargo.toml`
- [x] Add `IdMapSnapshot` struct with `Serialize`/`Deserialize` derives
- [x] `save()` — serializes IdMap to `<CACHE_DIR>/tmux-id-map-<workspace>.json`
- [x] `load()` — deserializes from disk, returns fresh IdMap on missing/corrupt file
- [x] `prune_stale()` — removes dead pane/tab mappings referencing IDs no longer in the Mux
- [x] `id_map_path()` — sanitizes workspace name for filename safety
- [x] `with_persistent_ids()` on HandlerContext — loads from disk + prunes stale on startup
- [x] `save_id_map()` on HandlerContext — called after each successful command dispatch
- [x] 7 new tests (prune_stale×3, snapshot round-trip, save/load round-trip, load nonexistent, path sanitization)
- **Files**: `id_map.rs`, `handlers.rs`, `server.rs`, `mux/Cargo.toml`
- **Strategy**: Monotonic counters preserved across restarts; stale entries pruned against live Mux state

### DROPPED from Phase 12

- ~~`display-menu` / `display-popup`~~ — GUI overlay commands, not CC protocol features. iTerm2 lists both in `forbiddenCommands`. No CC client uses them.
- ~~`%pane-mode-changed`~~ — iTerm2 explicitly ignores this notification ("Don't care")

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
