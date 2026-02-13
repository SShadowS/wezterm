# Plan: Tmux Control Mode Server in WezTerm

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

## Phase 1: Core Protocol Infrastructure (pure, testable, no I/O)

New files in `mux/src/tmux_compat_server/`:

### 1a. `target.rs` — Parse tmux target strings

Parse `SESSION:WINDOW.PANE` target syntax into resolved IDs.

```rust
pub struct TmuxTarget {
    pub session: Option<SessionRef>,  // $N, name, or None
    pub window: Option<WindowRef>,    // @N, index, name, or None
    pub pane: Option<PaneRef>,        // %N, index, or None
}

pub enum SessionRef { Id(u64), Name(String) }
pub enum WindowRef  { Id(u64), Index(u64), Name(String) }
pub enum PaneRef    { Id(u64), Index(u64) }

pub fn parse_target(target: &str) -> Result<TmuxTarget>;
```

**Tests** (input → expected output):
- `"%5"` → pane=Id(5)
- `"$0:@1.%2"` → session=Id(0), window=Id(1), pane=Id(2)
- `"mysession:0.1"` → session=Name("mysession"), window=Index(0), pane=Index(1)
- `":0.0"` → session=None, window=Index(0), pane=Index(0)
- `"@3"` → window=Id(3)
- `""` → all None (current context)

### 1b. `format.rs` — Expand tmux format strings

Expand `#{variable}` and `#{?cond,true,false}` in format strings.

```rust
pub struct FormatContext {
    pub pane_id: u64,
    pub pane_index: u64,
    pub pane_width: u64,
    pub pane_height: u64,
    pub pane_active: bool,
    pub pane_left: u64,
    pub pane_top: u64,
    pub window_id: u64,
    pub window_index: u64,
    pub window_name: String,
    pub window_active: bool,
    pub window_width: u64,
    pub window_height: u64,
    pub session_id: u64,
    pub session_name: String,
    pub cursor_x: u64,
    pub cursor_y: u64,
    pub history_limit: u64,
    pub history_size: u64,
}

pub fn expand_format(fmt: &str, ctx: &FormatContext) -> String;
```

**Tests** (format string + context → expected output):
- `"#{pane_id}"` with pane_id=5 → `"%5"`
- `"#{window_id}"` with window_id=1 → `"@1"`
- `"#{session_id}"` with session_id=0 → `"$0"`
- `"#{pane_index} #{pane_id}"` with index=0, id=3 → `"0 %3"`
- `"#{pane_width}x#{pane_height}"` with 80x24 → `"80x24"`
- `"#{?pane_active,active,}"` with active=true → `"active"`
- `"#{?pane_active,active,}"` with active=false → `""`
- Claude Code's exact format: `"#{pane_index} #{pane_id}"` with index=1, id=7 → `"1 %7"`

### 1c. `command_parser.rs` — Parse tmux CLI commands

Parse the text commands sent over CC protocol into structured command objects.

```rust
pub enum TmuxCliCommand {
    SplitWindow {
        horizontal: bool,          // -h flag
        vertical: bool,            // -v flag
        target: Option<String>,    // -t TARGET
        size: Option<String>,      // -l SIZE
    },
    SendKeys {
        target: Option<String>,    // -t TARGET
        keys: Vec<String>,         // remaining args: "echo hello" Enter
    },
    CapturePane {
        print: bool,               // -p flag
        target: Option<String>,    // -t TARGET
        escape: bool,              // -e flag
        octal_escape: bool,        // -C flag
        start_line: Option<i64>,   // -S N
    },
    ListPanes {
        all: bool,                 // -a flag
        session: bool,             // -s flag
        format: Option<String>,    // -F FORMAT
        target: Option<String>,    // -t TARGET
    },
    ListWindows {
        format: Option<String>,    // -F FORMAT
        target: Option<String>,    // -t TARGET
    },
    ListSessions {
        format: Option<String>,    // -F FORMAT
    },
    NewWindow {
        target: Option<String>,    // -t TARGET
    },
    SelectWindow {
        target: Option<String>,    // -t TARGET
    },
    SelectPane {
        target: Option<String>,    // -t TARGET
    },
    KillPane {
        target: Option<String>,    // -t TARGET
    },
    ResizePane {
        target: Option<String>,    // -t TARGET
        width: Option<u64>,        // -x W
        height: Option<u64>,       // -y H
    },
    RefreshClient {
        size: Option<String>,      // -C WxH
        flags: Option<String>,     // -f FLAGS
    },
    DisplayMessage {
        print: bool,               // -p flag
        format: Option<String>,    // format string
    },
    HasSession {
        target: Option<String>,    // -t TARGET
    },
    ListCommands,
}

pub fn parse_command(line: &str) -> Result<TmuxCliCommand>;
```

**Tests** (command string → parsed struct):
- `"split-window -h"` → SplitWindow { horizontal: true, .. }
- `"split-window -v -t %3"` → SplitWindow { vertical: true, target: Some("%3") }
- `"send-keys -t $0:@0.%1 \"echo hello\" Enter"` → SendKeys { target, keys: ["echo hello", "Enter"] }
- `"send-keys -t %5 0x68 0x69 0xA"` → SendKeys { target: Some("%5"), keys: ["0x68", "0x69", "0xA"] }
- `"capture-pane -p -t %1"` → CapturePane { print: true, target: Some("%1") }
- `"capture-pane -p -t %1 -e -C -S -32768"` → CapturePane { print, target, escape, octal_escape, start_line: Some(-32768) }
- `"list-panes -a -F '#{pane_index} #{pane_id}'"` → ListPanes { all: true, format: Some("#{pane_index} #{pane_id}") }
- `"list-windows -F '#{window_id} #{window_name}'"` → ListWindows { format: Some(...) }
- `"new-window"` → NewWindow { target: None }
- `"select-pane -t %2"` → SelectPane { target: Some("%2") }
- `"kill-pane -t %3"` → KillPane { target: Some("%3") }
- `"refresh-client -C 160x40"` → RefreshClient { size: Some("160x40") }
- `"display-message -p '#{session_id}'"` → DisplayMessage { print: true, format: Some("#{session_id}") }
- `"list-commands"` → ListCommands
- `"has-session -t mysession"` → HasSession { target: Some("mysession") }

### 1d. `response.rs` — Generate CC wire-format responses

Generate `%begin`/`%end`/`%error` guarded blocks and notification lines.

```rust
pub struct ResponseWriter {
    counter: u64,  // monotonically increasing command counter
}

impl ResponseWriter {
    pub fn new() -> Self;

    /// Generate a successful response block
    pub fn success(&mut self, output: &str) -> String;
    // Returns: "%begin <ts> <n> 1\n{output}%end <ts> <n> 1\n"

    /// Generate an empty success response (for send-keys, split-window, etc.)
    pub fn empty_success(&mut self) -> String;
    // Returns: "%begin <ts> <n> 1\n%end <ts> <n> 1\n"

    /// Generate an error response
    pub fn error(&mut self, message: &str) -> String;
    // Returns: "%begin <ts> <n> 1\n{message}\n%error <ts> <n> 1\n"

    // --- Notification generators (no counter increment) ---

    pub fn output_notification(pane_id: u64, data: &[u8]) -> String;
    // Returns: "%output %{pane_id} {vis_encoded_data}\n"

    pub fn layout_change_notification(window_id: u64, layout: &str) -> String;
    // Returns: "%layout-change @{window_id} {layout}\n"

    pub fn window_add_notification(window_id: u64) -> String;
    // Returns: "%window-add @{window_id}\n"

    pub fn window_close_notification(window_id: u64) -> String;
    // Returns: "%window-close @{window_id}\n"

    pub fn session_changed_notification(session_id: u64, name: &str) -> String;
    // Returns: "%session-changed ${session_id} {name}\n"

    pub fn window_pane_changed_notification(window_id: u64, pane_id: u64) -> String;
    // Returns: "%window-pane-changed @{window_id} %{pane_id}\n"

    pub fn exit_notification(reason: Option<&str>) -> String;
    // Returns: "%exit\n" or "%exit {reason}\n"
}

/// Encode bytes using OpenBSD vis(3) format (for %output data)
pub fn vis_encode(data: &[u8]) -> String;
```

**Tests** (verify exact wire format):
- `success("0 %0\n1 %1\n")` → `"%begin <ts> 1 1\n0 %0\n1 %1\n%end <ts> 1 1\n"` (verify timestamps match, counter=1)
- `empty_success()` → `"%begin <ts> 2 1\n%end <ts> 2 1\n"` (counter increments)
- `error("session not found: foo")` → `"%begin <ts> 3 1\nsession not found: foo\n%error <ts> 3 1\n"`
- `output_notification(1, b"hello\r\n")` → `"%output %1 hello\015\012\n"`
- `output_notification(3, b"\x1b[1mtest")` → `"%output %3 \033[1mtest\n"`
- `vis_encode(b"hello\r\n")` → `"hello\015\012"`
- `vis_encode(b"back\\slash")` → `"back\134slash"`
- `vis_encode(b"\x1b[31m")` → `"\033[31m"`
- `layout_change_notification(0, "b25f,80x24,0,0,2")` → `"%layout-change @0 b25f,80x24,0,0,2\n"`
- `exit_notification(None)` → `"%exit\n"`
- Round-trip test: generate response → parse with existing `tmux_cc::Parser` → verify Event matches

### 1e. `layout.rs` — Generate tmux layout strings from WezTerm tab tree

Convert WezTerm's tab split tree into tmux layout description format.

```rust
/// Generate a tmux layout string (with checksum) from a WezTerm Tab
pub fn generate_layout_string(tab: &Tab) -> String;
// Example output: "b25f,80x24,0,0,2" (single pane)
// Example output: "a]2f,160x40,0,0{80x40,0,0,0,79x40,81,0,3}" (h-split)

/// Compute the tmux layout checksum (csum function from tmux source)
fn layout_checksum(layout: &str) -> u16;
```

**Tests**:
- Single pane (80x24, pane_id 0) → `"b25f,80x24,0,0,0"`
- Horizontal split → `"XXXX,160x40,0,0{80x40,0,0,0,79x40,81,0,1}"`
- Vertical split → `"XXXX,80x48,0,0[80x24,0,0,0,80x23,0,25,1]"`
- Nested splits → verify valid checksum and parseable by existing `parse_layout()`

### 1f. `id_map.rs` — Bidirectional mapping between WezTerm and tmux IDs

```rust
pub struct IdMap {
    // WezTerm PaneId <-> tmux pane ID (u64)
    // WezTerm TabId  <-> tmux window ID (u64)
    // WezTerm workspace name <-> tmux session ID (u64)
    pane_map: BiMap<PaneId, u64>,
    tab_map: BiMap<TabId, u64>,
    session_map: BiMap<String, u64>,
    next_pane: u64,
    next_window: u64,
    next_session: u64,
}

impl IdMap {
    pub fn new() -> Self;
    pub fn get_or_create_tmux_pane_id(&mut self, wez_id: PaneId) -> u64;
    pub fn get_or_create_tmux_window_id(&mut self, wez_id: TabId) -> u64;
    pub fn get_or_create_tmux_session_id(&mut self, workspace: &str) -> u64;
    pub fn wezterm_pane_id(&self, tmux_id: u64) -> Option<PaneId>;
    pub fn wezterm_tab_id(&self, tmux_id: u64) -> Option<TabId>;
    pub fn workspace_name(&self, tmux_id: u64) -> Option<&str>;
}
```

**Tests**:
- Create mappings, verify bidirectional lookup
- Verify IDs are stable (same input always returns same ID)
- Verify new IDs are unique and incrementing

---

## Phase 2: Command Handlers (wired to WezTerm Mux)

New file: `mux/src/tmux_compat_server/handlers.rs`

Each handler takes a parsed `TmuxCliCommand` + mux access, performs the operation,
and returns the response content (the text between %begin and %end).

### 2a. `handle_split_window`

```
Input:  SplitWindow { horizontal: true, target: None }
Action: Mux::get() → find active pane → tab.split_and_insert()
Output: "" (empty success) + side-effects: %layout-change, %output notifications
```

Maps to existing: `wezterm cli split-pane --right` (for -h) or `--bottom` (for -v)

**Tests**:
- Split with no target → splits active pane, returns empty success
- Split with target `%3` → splits pane 3
- Split when pane too small → returns error "create pane failed: pane too small"

### 2b. `handle_send_keys`

```
Input:  SendKeys { target: Some("%1"), keys: ["echo hello", "Enter"] }
Action: Resolve target pane → pane.writer().write_all(resolved_keys)
Output: "" (empty success)
```

Key resolution: `"Enter"` → `\r`, `"Space"` → ` `, `"0x68"` → `h`, quoted strings → literal bytes

**Tests**:
- send-keys with named keys (Enter, Space, Tab, Escape, BSpace)
- send-keys with hex keys (0x68 0x69)
- send-keys with quoted string ("echo hello")
- send-keys with invalid target → error

### 2c. `handle_capture_pane`

```
Input:  CapturePane { print: true, target: Some("%1"), .. }
Action: Resolve target → pane.get_lines(range) → format as text
Output: pane text content (each line terminated by \n)
```

Maps to existing: `wezterm cli get-text --pane-id N`

**Tests**:
- Capture simple pane content → correct text
- Capture with -e flag → includes escape sequences
- Capture with -C flag → octal-escaped non-printables
- Capture with -S (start line) → limited scrollback

### 2d. `handle_list_panes`

```
Input:  ListPanes { all: true, format: Some("#{pane_index} #{pane_id}") }
Action: Mux::get() → iterate all windows/tabs/panes → expand format per pane
Output: one line per pane with expanded format
```

**Tests**:
- List all panes with Claude Code's format → `"0 %0\n1 %1\n"`
- List panes with default format → includes dimensions, history, active flag
- List with -t target → only panes in that window/session

### 2e. `handle_list_windows`

Similar to list-panes but iterates windows (tabs).

### 2f. `handle_new_window`

```
Input:  NewWindow { target: None }
Action: Mux::get() → domain.spawn() → creates new tab
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

---

## Phase 3: CC Protocol Server

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
    // 5. For each command: parse → handle → write response
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

**Note on %output**: This is the hardest notification to implement correctly.
Real tmux sends raw pane output (what the PTY produces). WezTerm's `PaneOutput`
notification just signals "pane has new content" without the raw bytes. We may need
to either:
- Tap into the PTY reader thread (complex, invasive)
- Use `pane.get_lines()` differentially (compare old vs new content)
- Skip %output initially and rely on capture-pane polling (Claude Code already does this)

**Recommendation**: Start WITHOUT %output forwarding. Claude Code uses `capture-pane`
to read pane content, not %output. Add %output later if needed.

---

## Phase 4: CLI Shim Binary

New crate: `tmux-compat-shim/` (small Rust binary)

### 4a. CLI argument parser

Parse tmux CLI arguments. Only need to handle the commands Claude Code uses plus
basic session management:

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
WezTerm codec. This reuses existing client infrastructure and is more efficient.

**B) CC protocol**: Connect to the CC server socket, send one command, read the
%begin/%end response, output the content, disconnect.

### 4c. Output formatting

Format output exactly as `tmux` would for CLI mode:
- `list-panes` → one line per pane (format-expanded)
- `capture-pane -p` → raw pane text to stdout
- `split-window` → no output (exit 0)
- `send-keys` → no output (exit 0)

### 4d. Environment detection

The shim checks `WEZTERM_UNIX_SOCKET` to find the mux server.
If the var isn't set, fall through to real `tmux` if available.

---

## Phase 5: Integration and Environment Setup

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

The `TMUX` format matches what real tmux sets: `socket_path,server_pid,session_id`.

### 5c. Shim installation

The tmux-compat shim binary is built as part of WezTerm and installed alongside.
A wrapper directory containing a symlink `tmux → wezterm-tmux-compat` is prepended
to `$PATH`.

---

## File Summary

New files to create:

```
mux/src/tmux_compat_server/
    mod.rs              -- module root, public API
    target.rs           -- tmux target string parser
    format.rs           -- #{variable} format expansion
    command_parser.rs   -- parse tmux CLI commands
    response.rs         -- generate CC wire format responses + vis encoding
    layout.rs           -- generate tmux layout strings
    id_map.rs           -- bidirectional ID mapping
    handlers.rs         -- per-command handlers (wired to Mux)
    server.rs           -- CC protocol server, socket listener, notifications

tmux-compat-shim/
    Cargo.toml          -- new crate for the shim binary
    src/main.rs         -- tmux CLI argument parser + WezTerm mux client
```

Files to modify:

```
mux/src/lib.rs              -- add `pub mod tmux_compat_server;`
mux/Cargo.toml              -- (any new deps)
Cargo.toml (workspace)      -- add tmux-compat-shim crate
```

## Test Strategy

Every component in Phase 1 is **pure logic with no I/O**, making tests straightforward:

1. **target.rs**: ~10 parse_target tests (various target formats)
2. **format.rs**: ~10 expand_format tests (each variable, conditionals)
3. **command_parser.rs**: ~15 parse_command tests (each command variant with flags)
4. **response.rs**: ~10 wire format tests + round-trip tests with existing CC parser
5. **layout.rs**: ~5 layout generation tests + round-trip with existing parse_layout()
6. **id_map.rs**: ~5 bidirectional mapping tests

Phase 2 handlers need mux integration tests (harder, may need mock Mux or real mux setup).

Phase 3-5 need integration/e2e tests (spawn WezTerm mux server, connect shim, verify behavior).

## Implementation Order

1. Phase 1 first (all pure, all testable, ~800 lines)
2. Phase 2 next (handlers, needs mux access, ~400 lines)
3. Phase 3 (server, socket, notifications, ~300 lines)
4. Phase 4 (shim binary, ~200 lines)
5. Phase 5 (config integration, env vars, ~100 lines)

Total estimate: ~1800 lines of new code across all phases.
