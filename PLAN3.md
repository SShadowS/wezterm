# PLAN3.md — Claude Code Agent Teams Compatibility Fixes

**Created**: 2026-02-14
**Status**: Complete — All 6 phases (14-19) implemented and tested
**Prerequisite**: Phases 1-13 from PLAN2.md (all complete)

---

## Background

Investigation using Claude Code agent teams revealed that wezterm's tmux compat server
(Phases 1-13) is **nearly complete** but has several gaps that prevent Claude Code from
working correctly. The primary symptom is:

```
Could not determine pane count for current window
```

This occurs when Claude Code tries to spawn teammate agents via `tmux split-window` and
query pane state. The root cause is a combination of missing format aliases and option
query gaps.

### How Claude Code Uses tmux (Agent Teams)

Claude Code's `PaneBackendExecutor.spawn()` sequence:
1. `tmux display-message -p '#{pane_id}'` — get current pane ID
2. `tmux split-window -h -P -F '#{pane_id}'` — spawn teammate pane, get its ID
3. `tmux send-keys -t %<id> "cd /path && env ... claude --agent-id ..." C-m` — start agent
4. `tmux list-panes -F '#{pane_index} #{pane_id}'` — enumerate panes
5. `tmux capture-pane -p -t %<id> -S -50` — monitor teammate output
6. `tmux kill-pane -t %<id>` — cleanup on shutdown

Claude Code also queries options like `pane-base-index` and `base-index` and uses
short-form format aliases (`#S`, `#I`, `#P`, `#D`) in some code paths.

---

## Phase 14: Short-Form Format Aliases

**Priority**: CRITICAL — Most likely root cause of agent team spawn failure
**Difficulty**: Easy
**Status**: [x] Complete
**Files**: `mux/src/tmux_compat_server/format.rs`

### Problem

The format engine only handles `#{variable_name}` long form. tmux also supports
single-character shortcuts like `#S` for `#{session_name}`. Claude Code and many
tmux scripts use these shortcuts in `display-message -p` and `list-panes -F` calls.

When the format engine encounters `#S`, it outputs the literal text `#S` instead of
the session name. Claude Code can't parse this as valid data.

### 14.1 — Add short-form alias expansion

- [x] In `expand_format()`, scan for `#X` single-char aliases via `short_alias_to_variable()`
- [x] Map the following aliases (from tmux `format.c` `format_table[]`):

| Short | Long form | Description |
|-------|-----------|-------------|
| `#D` | `#{pane_id}` | Unique pane ID (`%0`, `%1`) |
| `#F` | `#{window_flags}` | Window flags (`*`, `-`, `Z`) |
| `#I` | `#{window_index}` | Window index |
| `#P` | `#{pane_index}` | Pane index |
| `#S` | `#{session_name}` | Session name |
| `#T` | `#{pane_title}` | Pane title |
| `#W` | `#{window_name}` | Window name |

- [x] Handle `##` → literal `#` (tmux escape for `#` character)
- [x] Add unit tests for each alias (7 tests)
- [x] Add test for `##` literal escape (2 tests: standalone + in text)
- [x] Add test for mixed short-form and long-form: `#S:#{window_index}.#P`
- [x] Add test verifying all short aliases match their long-form equivalents
- [x] Add Claude Code usage pattern tests (`#S:#I.#P`, `#D #P`)
- [x] Add edge case tests (unrecognized alias, hash at end of string)
- [x] All 392 tmux compat tests pass (16 new Phase 14 tests)

### 14.2 — Verify format expansion in command contexts

Format expansion is shared across all commands via `expand_format()`, so short-form
aliases automatically work in `display-message -p`, `list-panes -F`, `split-window -P -F`,
etc. — verified by existing integration tests still passing (392/392).

---

## Phase 15: Option Query Improvements

**Priority**: HIGH — Claude Code queries these to determine pane indexing
**Difficulty**: Easy
**Status**: [x] Complete (15.1 and 15.2; 15.3 deferred)
**Files**: `mux/src/tmux_compat_server/handlers.rs`, `mux/src/tmux_compat_server/command_parser.rs`

### Problem

`show-options` returns a small hardcoded set of options. Claude Code queries
`pane-base-index` and `base-index` to determine pane numbering (known bug
[#23527](https://github.com/anthropics/claude-code/issues/23527)). If these
return empty/error, Claude Code may miscalculate pane targets.

### 15.1 — Add missing option values to `show-options`

- [x] Add these to the hardcoded global options map (12 total, up from 3):

| Option | Value | Notes |
|--------|-------|-------|
| `base-index` | `0` | Window numbering starts at 0 |
| `pane-base-index` | `0` | Pane numbering starts at 0 |
| `default-shell` | `/bin/sh` | Default shell |
| `status` | `off` | No status bar in CC mode |
| `mouse` | `off` | Default off |
| `focus-events` | `on` | iTerm2 queries this |
| `set-titles` | `off` | Terminal title control |
| `allow-rename` | `on` | Allow processes to rename windows |
| `renumber-windows` | `off` | Don't renumber after close |

- [x] Add these to the hardcoded window options map (5 total, up from 2):

| Option | Value | Notes |
|--------|-------|-------|
| `mode-keys` | `emacs` | Already present |
| `aggressive-resize` | `off` | Already present |
| `pane-base-index` | `0` | Some tools query at window scope |
| `remain-on-exit` | `off` | Default |
| `allow-rename` | `on` | Default |

### 15.2 — Support `-q` (quiet) and `-s` (server) flags

- [x] Added `quiet: bool` field to `ShowOptions` and `ShowWindowOptions` structs
- [x] Updated parsers to handle combined flag strings like `-gqv`, `-svq`, etc.
- [x] `-s` flag treated as equivalent to `-g` (server scope = global)
- [x] When `-q` is set, unknown options return empty success instead of error
- [x] Added parser tests: `show-options -gqv nonexistent`, `show-options -sv`
- [x] Added handler tests: quiet unknown, quiet non-global, new option values
- [x] All 405 tmux compat tests pass (13 new tests)

### 15.3 — Make `set-option` track values (optional enhancement)

- [ ] Deferred — currently `set-option` is a no-op which is sufficient for Claude Code

---

## Phase 16: Command Alias Completeness

**Priority**: MEDIUM — Prevents "unknown command" errors for common aliases
**Difficulty**: Easy
**Status**: [x] Complete
**Files**: `mux/src/tmux_compat_server/command_parser.rs`

### 16.1–16.5 — Add missing aliases

- [x] Added 15 new aliases to `parse_command()` dispatch table:

| Alias | Full command | Notes |
|-------|-------------|-------|
| `ls` | `list-sessions` | Most common tmux shorthand |
| `lsp` | `list-panes` | |
| `lsw` | `list-windows` | |
| `splitw` | `split-window` | |
| `neww` | `new-window` | |
| `selectw` | `select-window` | |
| `selectp` | `select-pane` | |
| `killp` | `kill-pane` | |
| `kills` | `kill-session` | |
| `capturep` | `capture-pane` | |
| `send` | `send-keys` | |
| `display` | `display-message` | |
| `has` | `has-session` | |
| `resizew` | `resize-window` | |
| `refresh` | `refresh-client` | |

- [x] 16 new tests covering all aliases with realistic flag combinations
- [x] All 421 tmux compat tests pass

### 16.6 — `respawn-pane` deferred (not needed by Claude Code)

### 16.7 — Alias audit complete

Full alias coverage now: `attach`, `breakp`, `capturep`, `deleteb`, `detach`,
`display`, `has`, `joinp`, `killp`, `kills`, `killw`, `ls`, `lsb`, `lsc`,
`lscm`, `lsp`, `lsw`, `movep`, `movew`, `new`, `neww`, `pasteb`, `refresh`,
`rename`, `renamew`, `resizep`, `resizew`, `selectl`, `selectp`, `selectw`,
`send`, `set`, `setb`, `show`, `show-option`, `show-window-option`, `showb`,
`showw`, `splitw`, `switchc`

---

## Phase 17: Missing Commands

**Priority**: MEDIUM-LOW — Used by cleanup scripts and orchestration tools
**Difficulty**: Easy-Medium
**Status**: [x] Complete
**Files**: `mux/src/tmux_compat_server/command_parser.rs`, `mux/src/tmux_compat_server/handlers.rs`

### 17.1 — `kill-server` command

- [x] Added `KillServer` variant to `TmuxCliCommand`
- [x] Parse: `kill-server` (no flags)
- [x] Handler: sets `detach_requested = true` on `TmuxCommandContext` to trigger shutdown
- [x] Parser + handler tests

### 17.2 — `wait-for` command (stub)

- [x] Added `WaitFor { signal: bool, channel: String }` to `TmuxCliCommand`
- [x] Parse: `wait-for [-L|-U|-S] <channel>` (aliases: `wait`)
- [x] Stub handler: returns empty success immediately for all flag combinations
- [x] Parser tests for `-S`, `-L`, and alias

### 17.3 — `pipe-pane` command (stub)

- [x] Added `PipePane { target, command }` to `TmuxCliCommand`
- [x] Parse: `pipe-pane [-o] [-O] [-I] [-t target] [command]` (aliases: `pipep`)
- [x] Stub handler: returns empty success
- [x] Parser tests with target, without args, and alias

### 17.4 — `display-popup` command (stub)

- [x] Added `DisplayPopup { target }` to `TmuxCliCommand`
- [x] Parse: `display-popup [-E] [-d dir] [-w width] [-h height] [-t target] [command]` (aliases: `popup`, `display-menu`, `menu`)
- [x] No-op handler: returns empty success
- [x] Parser tests: basic, alias, with multiple flags

### 17.5 — `run-shell` command

- [x] Added `RunShell { background, target, command }` to `TmuxCliCommand`
- [x] Parse: `run-shell [-b] [-C] [-d delay] [-t target] <command>` (aliases: `run`)
- [x] Handler: executes command via `std::process::Command` (sh -c / cmd /C), returns stdout
- [x] Parser tests: basic, background, with target, alias, no command
- [x] Handler tests: echo, no command, empty command
- [x] All 441 tmux compat tests pass (20 new Phase 17 tests)

---

## Phase 18: Robustness & Edge Cases

**Priority**: MEDIUM — Fixes known failure modes from Claude Code GitHub issues
**Difficulty**: Medium
**Status**: [x] Complete
**Files**: `mux/src/tmux_compat_server/command_parser.rs`, `mux/src/tmux_compat_server/handlers.rs`, `mux/src/tmux_compat_server/server.rs`

### 18.1 — Handle concurrent `split-window` race conditions

**Reference**: [Claude Code Issue #23615](https://github.com/anthropics/claude-code/issues/23615)

- [x] Verified: command processing is already sequential per-connection via blocking
  `sync_channel(1)` + `recv_timeout` loop in `process_cc_connection_sync()`
- [x] Each command completes and its response is sent before the next command is read
- [x] `spawn_into_main_thread` callbacks complete before the response channel fires
- [x] No race condition possible within a single connection; cross-connection races are
  handled by mux-level locking in `split_pane()`

### 18.2 — Handle `send-keys` with special characters

**Reference**: [Claude Code Issue #25375](https://github.com/anthropics/claude-code/issues/25375)

- [x] Verified: `env VAR=value command` syntax works in send-keys (shell_words handles quoting)
- [x] Added parser tests: literal mode with special chars, env syntax, named control keys
- [x] Added handler tests: `resolve_named_key` for C-c/C-d/C-z, special keys (Enter, Space,
  Tab, Escape, BSpace), arrow keys, function keys, unknown keys
- [x] All named key resolution already correct: C-a through C-z → control bytes, Enter → `\r`,
  Escape → `\x1b`, arrow keys → ANSI escapes, F1-F12 supported

### 18.3 — Graceful handling of unknown commands

- [x] Verified: unknown commands already produce well-formed `%begin`/`%error`/`%end` blocks
  via `parse_command()` → `Err` → `session.writer.error()` → `format_guard_block()`
- [x] Added `log::warn!` for parse errors in server.rs (includes command text for debugging)
- [x] Error format is already tmux-style: `unknown tmux command: "name"`
- [x] Added parser tests: unknown command returns error, empty command returns error

### 18.4 — Handle `-c <directory>` flag in spawn commands

- [x] Added `cwd: Option<String>` field to `SplitWindow`, `NewWindow`, `NewSession` enums
- [x] Updated parsers to store `-c` value instead of discarding it
- [x] `handle_split_window`: passes `cwd` to `SplitSource::Spawn { command_dir }`
- [x] `handle_new_window`: passes `cwd` to `spawn_tab_or_window(command_dir)`
- [x] `handle_new_session`: passes `cwd` to `spawn_tab_or_window(command_dir)`
- [x] Added parser tests for all three commands with `-c` flag

### 18.5 — Handle `-e <env>` flag in spawn commands

- [x] Added `env: Vec<String>` field to `SplitWindow`, `NewWindow`, `NewSession` enums
- [x] Updated parsers to collect multiple `-e KEY=VALUE` flags
- [x] When env is non-empty, creates `CommandBuilder::new_default_prog()` with env vars applied
- [x] `handle_split_window`: passes builder via `SplitSource::Spawn { command }`
- [x] `handle_new_window`: passes builder to `spawn_tab_or_window(command)`
- [x] `handle_new_session`: passes builder to `spawn_tab_or_window(command)`
- [x] Added parser tests: single -e, multiple -e, combined -c and -e
- [x] All 458 tmux compat tests pass (17 new Phase 18 tests)

---

## Phase 19: Diagnostic & Debugging Support

**Priority**: LOW — Nice for troubleshooting
**Difficulty**: Easy
**Status**: [x] Complete
**Files**: `mux/src/tmux_compat_server/command_parser.rs`, `mux/src/tmux_compat_server/handlers.rs`

### 19.1 — `server-info` / `info` command

- [x] Added `ServerInfo` variant to `TmuxCliCommand`
- [x] Parse: `server-info` and `info` alias
- [x] Handler returns: wezterm version, pid, session/window/pane counts, active workspace,
  active session/window/pane IDs
- [x] Added to `handle_list_commands()` (45 total commands)
- [x] Handler + parser tests

### 19.2 — Improve error messages

- [x] All error messages now use tmux-style `can't find` prefix:
  - `can't find session: <name>`, `can't find session: $<id>`
  - `can't find window: @<id>`, `can't find window: <name>`, `can't find window: index <n>`
  - `can't find pane: %<id>`, `can't find pane`, `can't find pane: index <n>`
  - `can't find pane for split`, `can't find pane for break-pane`
  - `can't find window containing tab <id>`, `can't find window for tab <id>`
  - `can't find source window: @<id>`

### 19.3 — Add `-v` verbose flag to `display-message`

- [x] Added `verbose: bool` field to `DisplayMessage` enum
- [x] Parser captures `-v` flag (previously accepted but ignored)
- [x] When `-v` is set, output includes comment lines showing:
  - `# format: <original_format_string>`
  - `# <variable_name> -> <resolved_value>` for each referenced variable
  - Short aliases shown as `# #X (variable_name) -> <value>`
  - Final line is the normal expanded result
- [x] Parser tests: with and without `-v` flag
- [x] Handler test: verbose output format verification
- [x] All 467 tmux compat tests pass (9 new Phase 19 tests)

---

## Summary / Priority Order

| Phase | Items | Priority | Status | Tests Added |
|-------|-------|----------|--------|-------------|
| **14** | Short-form format aliases | CRITICAL | Complete | 16 |
| **15** | Option query improvements | HIGH | Complete | 13 |
| **16** | Command alias completeness | MEDIUM | Complete | 16 |
| **17** | Missing commands | MEDIUM-LOW | Complete | 20 |
| **18** | Robustness & edge cases | MEDIUM | Complete | 17 |
| **19** | Diagnostic support | LOW | Complete | 9 |

**All 6 phases complete.** Total: 467 tmux compat tests (91 new tests across Phases 14-19).
