# Plan: Tmux Control Mode Server in WezTerm

## Current Status

**Branch:** `claude/add-python-api-western-Ik3uz`
**Phase 1: COMPLETE** (committed, pushed, needs local test validation)
**Phase 2-5: NOT STARTED**

### What to do first on your machine

```bash
git fetch origin claude/add-python-api-western-Ik3uz
git checkout claude/add-python-api-western-Ik3uz
cargo test -p mux --lib tmux_compat_server
```

This should run all Phase 1 unit tests. If everything passes, Phase 2 can begin.

---

## Phase 1 Status: COMPLETE

### Commits

| Hash | Description |
|------|-------------|
| `4749784` | Add implementation plan (PLAN.md) |
| `4063039` | Add tmux compat server Phase 1: pure logic modules (3,352 lines) |
| `4a9e079` | Add *.rlib to .gitignore |

### Files Created (all in `mux/src/tmux_compat_server/`)

| File | Lines | Description | Test count |
|------|-------|-------------|------------|
| `mod.rs` | 12 | Module root, re-exports all submodules | — |
| `target.rs` | 454 | Parses tmux target strings (`SESSION:WINDOW.PANE`) into `TmuxTarget` | 18 tests |
| `format.rs` | 524 | Expands `#{variable}` and `#{?cond,true,false}` format strings | 16 tests |
| `command_parser.rs` | 1047 | Parses tmux CLI command text into `TmuxCliCommand` enum | 23 tests |
| `response.rs` | 455 | Generates CC wire-format responses (`%begin`/`%end`/`%error`) + `vis_encode()` | 14 tests |
| `layout.rs` | 589 | Generates tmux layout description strings + checksum | 8 tests |
| `id_map.rs` | 270 | Bidirectional WezTerm <-> tmux ID mapping | 5 tests |

**Total: 3,351 lines, ~84 unit tests**

### Files Modified

- `mux/src/lib.rs` — added `pub mod tmux_compat_server;`

### Key Design Decisions Made in Phase 1

1. **`id_map.rs` uses simple `HashMap` pairs** (not a third-party BiMap crate) to avoid adding dependencies.
2. **`response.rs` uses `std::time::SystemTime`** for timestamps (not `chrono::Utc::now()`) because the workspace's chrono doesn't have the `clock` feature.
3. **`command_parser.rs` is hand-written** (no parser combinator library) — uses a custom token iterator over shell-style arguments with quote handling.
4. **`layout.rs` checksum** implements the exact same `csum` algorithm from tmux source (`layout_checksum`).
5. **`target.rs`** supports the full `$SESSION:@WINDOW.%PANE` syntax including name-based, index-based, and ID-based refs.
6. **`format.rs`** supports all variables Claude Code uses plus conditionals `#{?cond,true,false}` and nested `#{}`.

### Known Issues

- **Build environment limitation**: In the sandbox where this was developed, `mux/src/client.rs` fails to compile because `chrono::Utc::now()` requires the `clock` feature which isn't explicitly enabled in `Cargo.toml`. This is a **pre-existing issue** unrelated to our code. It prevents `cargo test -p mux` from running in the sandbox but should work on machines where chrono's `clock` feature is pulled in transitively by the full workspace build.

---

## Goal

Make WezTerm act as a tmux-compatible server so tools like Claude Code's Agent Teams
can use their existing tmux integration (split-window, send-keys, capture-pane, list-panes)
without knowing they're in WezTerm.

## Architecture

```
[Claude Code]
   runs: tmux split-window -h
      |
      v
[tmux-compat shim binary]      <-- new Rust binary in wezterm repo
   parses tmux CLI args
   connects to CC socket
   sends command, reads response
   formats output as tmux would
      |
      v
[WezTerm Tmux CC Server]       <-- new module in mux/
   listens on Unix socket
   speaks tmux CC wire protocol
   translates commands to Mux operations
   sends %begin/%end responses
   streams %output/%layout-change notifications
      |
      v
[WezTerm Mux]                  <-- existing infrastructure
   split panes, read content, write to panes, list panes
```

Environment: WezTerm sets `TMUX=/tmp/wezterm-tmux-{pid},{pid},0` and puts the shim
on `$PATH` so Claude Code auto-detects tmux mode.

## Existing Code We Build On

| Component | Location | Reuse |
|---|---|---|
| CC protocol parser (PEG grammar) | `wezterm-escape-parser/src/tmux_cc/` | Reuse `Event`, `Guarded`, `unvis()` types for testing |
| Tmux commands (client side) | `mux/src/tmux_commands.rs` | Reference for how commands map to mux ops |
| Mux notification system | `mux/src/lib.rs:MuxNotification` | Subscribe for `%output`, `%layout-change` events |
| Mux core (panes, tabs, windows) | `mux/src/{pane,tab,window,domain}.rs` | Direct access to split, read, write, list |
| CLI implementations | `wezterm/src/cli/{split_pane,send_text,get_text,list}.rs` | Reference for how CLI maps to codec RPCs |

---

## Phase 2: Command Handlers (wired to WezTerm Mux) — NOT STARTED

New file: `mux/src/tmux_compat_server/handlers.rs`

Each handler takes a parsed `TmuxCliCommand` + mux access, performs the operation,
and returns the response content (the text between %begin and %end).

### Implementation approach

The handlers need access to `Mux::get()` (the global mux singleton). Key references for how to interact with the mux:

- **Split pane**: See `wezterm/src/cli/split_pane.rs` — uses `codec::SplitPane` RPC or direct `tab.split_and_insert()`
- **Send keys**: See `wezterm/src/cli/send_text.rs` — uses `pane.writer().write_all()`
- **Capture pane**: See `wezterm/src/cli/get_text.rs` — uses `pane.get_lines(range)`
- **List panes**: See `wezterm/src/cli/list.rs` — iterates `mux.iter_panes()`
- **Kill pane**: See existing `mux/src/tmux_commands.rs:PaneKill`

### 2a. `handle_split_window`

```
Input:  SplitWindow { horizontal: true, target: None }
Action: Mux::get() -> find active pane -> tab.split_and_insert()
Output: "" (empty success) + side-effects: %layout-change, %output notifications
```

Maps to existing: `wezterm cli split-pane --right` (for -h) or `--bottom` (for -v)

**Tests**:
- Split with no target -> splits active pane, returns empty success
- Split with target `%3` -> splits pane 3
- Split when pane too small -> returns error "create pane failed: pane too small"

### 2b. `handle_send_keys`

```
Input:  SendKeys { target: Some("%1"), keys: ["echo hello", "Enter"] }
Action: Resolve target pane -> pane.writer().write_all(resolved_keys)
Output: "" (empty success)
```

Key resolution: `"Enter"` -> `\r`, `"Space"` -> ` `, `"0x68"` -> `h`, quoted strings -> literal bytes

**Tests**:
- send-keys with named keys (Enter, Space, Tab, Escape, BSpace)
- send-keys with hex keys (0x68 0x69)
- send-keys with quoted string ("echo hello")
- send-keys with invalid target -> error

### 2c. `handle_capture_pane`

```
Input:  CapturePane { print: true, target: Some("%1"), .. }
Action: Resolve target -> pane.get_lines(range) -> format as text
Output: pane text content (each line terminated by \n)
```

Maps to existing: `wezterm cli get-text --pane-id N`

**Tests**:
- Capture simple pane content -> correct text
- Capture with -e flag -> includes escape sequences
- Capture with -C flag -> octal-escaped non-printables
- Capture with -S (start line) -> limited scrollback

### 2d. `handle_list_panes`

```
Input:  ListPanes { all: true, format: Some("#{pane_index} #{pane_id}") }
Action: Mux::get() -> iterate all windows/tabs/panes -> expand format per pane
Output: one line per pane with expanded format
```

**Tests**:
- List all panes with Claude Code's format -> `"0 %0\n1 %1\n"`
- List panes with default format -> includes dimensions, history, active flag
- List with -t target -> only panes in that window/session

### 2e. `handle_list_windows`

Similar to list-panes but iterates windows (tabs).

### 2f. `handle_new_window`

```
Input:  NewWindow { target: None }
Action: Mux::get() -> domain.spawn() -> creates new tab
Output: "" (empty success) + %window-add notification
```

### 2g. `handle_select_pane`, `handle_select_window`, `handle_kill_pane`

Straightforward mappings to existing mux operations.

### 2h. `handle_display_message`

```
Input:  DisplayMessage { print: true, format: Some("#{session_id}") }
Action: Expand format string with current context
Output: expanded string
```

### 2i. `handle_list_commands`

Returns list of supported commands (for WezTerm's own tmux client init sequence).

### 2j. `handle_resize_pane`

```
Input:  ResizePane { target: Some("%1"), width: Some(100), height: Some(40) }
Action: Resolve target pane -> resize the pane
Output: "" (empty success)
```

### 2k. `handle_has_session`

```
Input:  HasSession { target: Some("main") }
Action: Check if session/workspace exists
Output: "" (empty success) or error
```

---

## Phase 3: CC Protocol Server — NOT STARTED

New file: `mux/src/tmux_compat_server/server.rs`

### 3a. Session state

```rust
pub struct TmuxCompatSession {
    id_map: IdMap,
    response_writer: ResponseWriter,
    writer: Box<dyn Write + Send>,  // output to client
    active_pane: Option<u64>,       // tmux pane ID
    active_window: Option<u64>,     // tmux window ID
    active_session: Option<u64>,    // tmux session ID
    client_size: Option<(u64, u64)>, // from refresh-client -C
}
```

### 3b. Connection handler

```rust
pub async fn handle_connection(
    reader: impl BufRead,
    writer: impl Write,
) {
    // 1. Send initial DCS sequence (for -CC mode): \x1bP1000p
    // 2. Send initial %begin/%end guard
    // 3. Send initial notifications (%session-changed, %window-add, etc.)
    // 4. Read commands line by line
    // 5. For each command: parse -> handle -> write response
    // 6. On empty line or error: send %exit, cleanup
}
```

### 3c. Unix socket listener

```rust
pub fn start_tmux_compat_listener(socket_path: &Path) -> Result<()> {
    // Create Unix socket at socket_path
    // For each connection: spawn handle_connection
    // Register with Mux for notifications
}
```

### 3d. Notification forwarding

Subscribe to `MuxNotification` and translate to CC notifications:

| MuxNotification | CC Notification |
|---|---|
| `PaneOutput(id)` | `%output %N <vis_data>` |
| `TabResized(id)` | `%layout-change @N <layout>` |
| `PaneAdded(id)` | (part of %layout-change after split) |
| `PaneRemoved(id)` | `%window-close @N` (if last pane in tab) |
| `WindowCreated(id)` | `%window-add @N` |
| `WindowRemoved(id)` | `%window-close @N` |
| `PaneFocused(id)` | `%window-pane-changed @N %M` |

**Note on %output**: Start WITHOUT %output forwarding. Claude Code uses `capture-pane`
to read pane content, not %output. Add %output later if needed.

---

## Phase 4: CLI Shim Binary — NOT STARTED

New crate: `tmux-compat-shim/` (small Rust binary)

### 4a. CLI argument parser

Parse tmux CLI arguments. Only need to handle the commands Claude Code uses:

```
tmux [-C|-CC] [new-session|attach-session] [-t TARGET]
tmux split-window [-h|-v] [-t TARGET]
tmux send-keys [-t TARGET] KEY...
tmux capture-pane [-p] [-t TARGET] [-e] [-C] [-S N]
tmux list-panes [-a] [-s] [-F FORMAT] [-t TARGET]
tmux list-windows [-F FORMAT] [-t TARGET]
tmux has-session [-t TARGET]
tmux kill-pane [-t TARGET]
tmux display-message [-p] [FORMAT]
```

### 4b. Connection to WezTerm

Two options (implement both, prefer A):

**A) Direct mux RPC**: Connect to WezTerm's existing Unix socket using the
WezTerm codec. Reuses existing client infrastructure.

**B) CC protocol**: Connect to the CC server socket, send one command, read the
%begin/%end response, output the content, disconnect.

### 4c. Output formatting

Format output exactly as `tmux` would for CLI mode:
- `list-panes` -> one line per pane (format-expanded)
- `capture-pane -p` -> raw pane text to stdout
- `split-window` -> no output (exit 0)
- `send-keys` -> no output (exit 0)

### 4d. Environment detection

The shim checks `WEZTERM_UNIX_SOCKET` to find the mux server.
If the var isn't set, fall through to real `tmux` if available.

---

## Phase 5: Integration and Environment Setup — NOT STARTED

### 5a. WezTerm configuration

Add config option to enable tmux compat mode:

```lua
config.enable_tmux_compat = true
-- or more granular:
config.tmux_compat = {
    enabled = true,
    socket_path = "/tmp/wezterm-tmux-{pid}",
}
```

### 5b. Environment variables

When tmux compat is enabled, WezTerm sets in spawned shells:

```
TMUX=/tmp/wezterm-tmux-{pid},{pid},0
PATH=/path/to/tmux-shim-dir:$PATH
```

### 5c. Shim installation

The tmux-compat shim binary is built as part of WezTerm and installed alongside.
A wrapper directory containing a symlink `tmux -> wezterm-tmux-compat` is prepended
to `$PATH`.

---

## File Summary

### Already created (Phase 1):

```
mux/src/tmux_compat_server/
    mod.rs              -- module root, re-exports (12 lines)
    target.rs           -- tmux target string parser (454 lines, 18 tests)
    format.rs           -- #{variable} format expansion (524 lines, 16 tests)
    command_parser.rs   -- parse tmux CLI commands (1047 lines, 23 tests)
    response.rs         -- CC wire format responses + vis encoding (455 lines, 14 tests)
    layout.rs           -- tmux layout strings + checksum (589 lines, 8 tests)
    id_map.rs           -- bidirectional ID mapping (270 lines, 5 tests)
```

### Still to create:

```
mux/src/tmux_compat_server/
    handlers.rs         -- per-command handlers wired to Mux (Phase 2)
    server.rs           -- CC protocol server, socket listener, notifications (Phase 3)

tmux-compat-shim/
    Cargo.toml          -- new crate for the shim binary (Phase 4)
    src/main.rs         -- tmux CLI argument parser + WezTerm mux client (Phase 4)
```

### Files to modify:

```
mux/src/lib.rs              -- already modified (pub mod tmux_compat_server)
mux/Cargo.toml              -- may need new deps for Phase 2-3
Cargo.toml (workspace)      -- add tmux-compat-shim crate (Phase 4)
```

## Test Strategy

| Phase | Approach | How to run |
|-------|----------|------------|
| Phase 1 | Unit tests, pure logic, no I/O | `cargo test -p mux --lib tmux_compat_server` |
| Phase 2 | Integration tests needing Mux singleton | `cargo test -p mux --lib tmux_compat_server::handlers` (may need `#[cfg(test)]` mock setup) |
| Phase 3 | Server integration (spawn, connect, verify) | `cargo test -p mux --test tmux_compat_server_integration` |
| Phase 4 | E2E (spawn WezTerm + shim, verify behavior) | Manual or `cargo test -p tmux-compat-shim` |

## Implementation Order

1. ~~Phase 1 (all pure, all testable) — **DONE**~~
2. Phase 2 (handlers, needs mux access, ~400 lines)
3. Phase 3 (server, socket, notifications, ~300 lines)
4. Phase 4 (shim binary, ~200 lines)
5. Phase 5 (config integration, env vars, ~100 lines)
