# Plan: Tmux Control Mode Server in WezTerm

## Current Status

**Branch:** `claude/add-python-api-western-Ik3uz`
**Phase 1: COMPLETE** (committed, pushed)
**Phase 2: COMPLETE** (handlers.rs — 1323 lines, 28 tests)
**Phase 3: COMPLETE** (server.rs — 687 lines, 17 tests)
**Phase 4-5: NOT STARTED**

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

New crate: `tmux-compat-shim/` (small Rust binary, ~200 lines)

Uses **Approach B (CC Protocol)** exclusively. The shim connects to the Phase 3
CC server socket, sends a single command as a text line, reads the `%begin`/`%end`
response, outputs the body to stdout, and exits. This is dramatically simpler
than Approach A (direct mux RPC) which would require linking `wezterm-client`,
`codec`, `mux`, and `config` — adding massive build complexity for no benefit.

### Architecture

```
tmux split-window -h -t %3          <-- Claude Code runs this
   |
   v
[tmux-compat-shim binary]
   1. Parse CLI args into command text: "split-window -h -t %3"
   2. Read socket path from WEZTERM_TMUX_CC env var
   3. Connect to CC server via Unix domain socket
   4. Skip initial handshake (greeting block + notifications)
   5. Send command text + \n
   6. Read response: find %begin, collect body until %end or %error
   7. Print body to stdout (or stderr for errors)
   8. Exit 0 (success) or exit 1 (error)
```

### 4a. Crate structure

```
tmux-compat-shim/
├── Cargo.toml
└── src/
    └── main.rs         (~200 lines)
```

**Cargo.toml:**

```toml
[package]
name = "tmux-compat-shim"
version = "0.1.0"
authors = ["Wez Furlong <wez@wezfurlong.org>"]
edition = "2018"
publish = false

[[bin]]
name = "tmux"
path = "src/main.rs"

[dependencies]
anyhow.workspace = true
wezterm-uds.workspace = true
```

**Key decisions:**
- Binary name is `tmux` (via `[[bin]]`) so it shadows real tmux on `$PATH`
- Only 2 dependencies: `anyhow` for errors, `wezterm-uds` for portable sockets
- No `clap` — hand-parse args to keep binary tiny and fast (tmux CLI is simple)
- No async runtime — synchronous I/O is fine for a one-shot command

**Workspace integration:** Add `"tmux-compat-shim"` to `[workspace] members` in
root `Cargo.toml`.

### 4b. CLI argument parsing

Hand-parsed (no clap) to keep the binary small and startup fast.

The shim receives `tmux <args>` because it's named `tmux` on PATH.
It needs to handle two modes:

**Mode 1: One-shot command** (most common for Claude Code)
```
tmux split-window -h -t %3
tmux send-keys -t %5 "echo hello" Enter
tmux capture-pane -p -t %5 -S -50
tmux list-panes -a -F "#{pane_id} #{pane_index}"
tmux list-windows -F "#{window_id}"
tmux has-session -t main
tmux kill-pane -t %5
tmux display-message -p "#{session_id}"
```

The shim reconstructs the command text by joining `args[1..]` with spaces
(preserving quotes for args that contain spaces). This is exactly what the
Phase 1 `command_parser::parse_command()` expects on the server side.

**Mode 2: Session commands** (no-ops / special handling)
```
tmux -CC new-session -t main
tmux -CC attach-session -t main
tmux -C new-session
```

These are connection-mode commands. The shim handles them as follows:
- `-C` / `-CC` flags: Ignored (the shim always uses one-shot mode)
- `new-session`: Print nothing, exit 0 (session already exists in WezTerm)
- `attach-session`: Print nothing, exit 0 (already attached)

**Mode 3: Version / server-info queries**
```
tmux -V
tmux list-commands
```
- `-V`: Print `tmux 3.3a (wezterm-compat)`, exit 0
- `list-commands`: Forward to CC server (already handled by Phase 2)

### 4c. Connection protocol

**Socket path:** Read from `WEZTERM_TMUX_CC` environment variable.
This is separate from `WEZTERM_UNIX_SOCKET` (which is the mux RPC socket).
Phase 5 sets both when spawning shells.

**Handshake skipping:** When the shim connects, the CC server sends:
1. An empty `%begin TIMESTAMP N 1` / `%end TIMESTAMP N 1` greeting block
2. `%session-changed $N NAME` notification
3. One or more `%window-add @N` notifications

The shim must read and discard all of this before sending its command.
Strategy: read lines until we see the `%end` of the greeting block (counter=1),
then drain any `%`-prefixed notification lines that follow before any command
is sent. Use a short timeout (100ms with no data) to detect when the initial
burst is done.

**Simpler alternative:** Add a `--oneshot` mode to the CC server that skips
the handshake. The shim sends `oneshot\n` as its first line, and the server
responds with no greeting — just processes the next command and closes.
This is a minor addition to `server.rs` (~10 lines) and eliminates the
handshake complexity entirely.

**Command send and response read:**
```
1. Write: "split-window -h -t %3\n"
2. Read lines until we see "%begin TIMESTAMP N 1"
3. Collect body lines until "%end TIMESTAMP N 1" (success) or "%error TIMESTAMP N 1" (error)
4. Output body to stdout/stderr
5. Close connection, exit
```

### 4d. Output formatting

The CC server already formats responses exactly as tmux CLI would:
- `list-panes` → one line per pane (format-expanded) — body is already correct
- `capture-pane -p` → raw pane text — body is already correct
- `split-window` → empty body — print nothing, exit 0
- `send-keys` → empty body — print nothing, exit 0
- `has-session` → empty body (success) or error text
- `display-message -p` → expanded format string

The shim just prints the response body verbatim. No extra formatting needed.

### 4e. Error handling and fallthrough

1. If `WEZTERM_TMUX_CC` is not set → try to exec real `tmux` (for non-WezTerm contexts)
2. If connection to CC socket fails → print error to stderr, exit 1
3. If server returns `%error` → print error body to stderr, exit 1
4. If connection drops mid-response → print error to stderr, exit 1

**Fallthrough to real tmux:**
```rust
fn exec_real_tmux() -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Search PATH for real tmux (skip ourselves)
    // On Unix: exec() replaces process
    // On Windows: std::process::Command + exit with its code
}
```

### 4f. Tests

Unit tests in `src/main.rs` (~8 tests):
- `parse_oneshot_command` — "split-window -h -t %3" → correct command text
- `parse_session_command` — "-CC new-session" → recognized as no-op
- `parse_version` — "-V" → prints version string
- `skip_handshake` — given a stream of greeting lines, correctly identifies end
- `extract_response_success` — %begin/%end block → body text
- `extract_response_error` — %begin/%error block → error text + exit 1
- `empty_response` — %begin/%end with no body → empty string (exit 0)
- `fallthrough_when_no_env` — no WEZTERM_TMUX_CC → attempts real tmux

### 4g. Implementation checklist

1. Create `tmux-compat-shim/Cargo.toml` with minimal deps
2. Add `"tmux-compat-shim"` to workspace members in root `Cargo.toml`
3. Implement `src/main.rs`:
   - `main()` → parse args, dispatch
   - `parse_args()` → detect mode (oneshot / session / version)
   - `reconstruct_command()` → join args into command text
   - `connect_and_execute()` → socket connect, handshake skip, send, read response
   - `skip_handshake()` → consume greeting + initial notifications
   - `read_response()` → parse %begin/%end/%error block
   - `exec_real_tmux()` → fallthrough
4. Add unit tests
5. Verify: `cargo check -p tmux-compat-shim`
6. Verify: `cargo test -p tmux-compat-shim`

### 4h. Optional: `--oneshot` mode in server.rs (~10 lines)

Add to `process_cc_connection()` in `server.rs`: if the first line received is
`"oneshot"`, skip the initial handshake. This makes the shim simpler and faster.

```rust
// In process_cc_connection, after creating session:
let first_line = read_first_line(&mut stream).await?;
let oneshot = first_line.trim() == "oneshot";

if !oneshot {
    // Normal CC client: send greeting
    let handshake = build_initial_handshake(&mut session);
    stream.write_all(handshake.as_bytes()).await?;
    stream.flush().await?;
    // Process first_line as a command if it's not empty
}
// For oneshot: skip greeting, wait for the actual command
```

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
TMUX=/path/to/cc-socket,{pid},0       -- makes Claude Code detect "tmux" mode
WEZTERM_TMUX_CC=/path/to/cc-socket    -- shim uses this to find the CC server
PATH=/path/to/tmux-shim-dir:$PATH     -- shim shadows real tmux
```

The CC socket path follows WezTerm's existing convention:
`config::RUNTIME_DIR/tmux-cc-{pid}` (similar to `gui-sock-{pid}`).

The `TMUX` variable is what Claude Code checks to detect tmux mode. Its format
is `socket_path,pid,session` — we reuse the CC socket path here.

### 5c. CC server startup

In the GUI startup path (`wezterm-gui/src/main.rs`), after setting
`WEZTERM_UNIX_SOCKET`, also start the CC listener:

```rust
if config.enable_tmux_compat {
    let cc_path = config::RUNTIME_DIR.join(format!("tmux-cc-{}", std::process::id()));
    std::env::set_var("WEZTERM_TMUX_CC", &cc_path);
    mux::tmux_compat_server::server::start_tmux_compat_listener(&cc_path)?;
}
```

### 5d. Shim installation

The `tmux` binary (from `tmux-compat-shim` crate) is built as part of WezTerm.
A wrapper directory containing the `tmux` binary is prepended to `$PATH` in
spawned shells so it shadows the real tmux.

On install:
```
$INSTALL_DIR/
  wezterm
  wezterm-gui
  wezterm-mux-server
  tmux-compat/
    tmux              <-- the shim binary
```

Environment setup prepends `$INSTALL_DIR/tmux-compat` to PATH.

### 5e. Cleanup

When WezTerm exits, the CC socket file is cleaned up (the listener thread exits
when the process terminates, and the socket file is already removed-on-bind in
`start_tmux_compat_listener`).

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

### Created (Phase 2):

```
mux/src/tmux_compat_server/
    handlers.rs         -- per-command handlers wired to Mux (1323 lines, 28 tests)
```

### Created (Phase 3):

```
mux/src/tmux_compat_server/
    server.rs           -- CC protocol server, socket listener, notifications (687 lines, 17 tests)
```

### Still to create:

```
tmux-compat-shim/
    Cargo.toml          -- new crate: deps are anyhow + wezterm-uds only (Phase 4)
    src/main.rs         -- CLI arg parser + CC socket client (~200 lines) (Phase 4)
```

### Files to modify:

```
Cargo.toml (workspace)      -- add tmux-compat-shim to members (Phase 4)
mux/src/tmux_compat_server/server.rs -- optional: add oneshot mode (Phase 4h)
wezterm-gui/src/main.rs     -- start CC listener on startup (Phase 5)
config/src/lib.rs           -- add enable_tmux_compat config option (Phase 5)
mux/src/domain.rs           -- set WEZTERM_TMUX_CC + TMUX + PATH in spawned shells (Phase 5)
```

## Test Strategy

| Phase | Approach | How to run |
|-------|----------|------------|
| Phase 1 | Unit tests, pure logic, no I/O (84 tests) | `cargo test -p mux --lib tmux_compat_server` |
| Phase 2 | Unit tests with mock Mux state (28 tests) | `cargo test -p mux --lib tmux_compat_server::handlers` |
| Phase 3 | Unit tests for layout/notification/line-extraction (17 tests) | `cargo test -p mux --lib tmux_compat_server::server` |
| Phase 4 | Unit tests for arg parsing + response extraction (~8 tests) | `cargo test -p tmux-compat-shim` |
| Phase 5 | Manual E2E: start WezTerm, run shim commands, verify | Manual testing |

## Implementation Order

1. ~~Phase 1 (all pure, all testable) — **DONE**~~
2. ~~Phase 2 (handlers, wired to Mux, 1323 lines, 28 tests) — **DONE**~~
3. ~~Phase 3 (server, socket, notifications, 687 lines, 17 tests) — **DONE**~~
4. Phase 4 (shim binary via CC protocol, ~200 lines, ~8 tests)
5. Phase 5 (config integration, env vars, CC server startup, ~100 lines)
