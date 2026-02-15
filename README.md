# WezTerm Evo

**WezTerm fork evolved for AI-native terminal workflows.**

A GPU-accelerated cross-platform terminal emulator forked from [wezterm/wezterm](https://github.com/wezterm/wezterm), enhanced specifically for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) and its Agent Team feature.

For upstream documentation and general WezTerm usage, see [wezterm.org](https://wezterm.org/).

## Key Features

### Claude Code Agent Team Integration

WezTerm Evo includes a full **tmux CC (Control-Client) protocol server** that lets Claude Code agents manage terminal panes natively using standard tmux commands — no actual tmux installation required.

- **44 tmux-compatible commands** — `split-window`, `send-keys`, `list-panes`, `select-layout`, `capture-pane`, `kill-pane`, and many more
- **10 real-time notifications** — pane focus changes, tab/window lifecycle events, layout changes, clipboard sync
- **36 format variables** — `pane_id`, `window_name`, `session_name`, `pane_current_path`, etc.
- **Tmux-compat shim binary** (`tmux-compat-shim/`) — drop-in `tmux` executable so Claude agents use familiar `tmux` commands that route to WezTerm's CC server
- **`env` shim for Windows** (`env-shim/`) — emulates the Unix `env` command (`KEY=VAL command args...`, `-i`, `-u`) so cross-platform scripts just work

### Per-Pane Header Bars

Optional 1-row colored headers at the top of each pane, useful for identifying agent panes in split layouts.

- **Lua API**: `pane:set_header("label")` / `pane:get_header()`
- **Format callback**: `format-pane-header` event (works like `format-tab-title`)
- **Config colors**: `pane_header_active_fg_color`, `pane_header_active_bg_color`, `pane_header_inactive_fg_color`, `pane_header_inactive_bg_color`

### Tab Layout System

Switch between tmux-style layouts via Lua, CLI, or keybinding:

| Layout | Description |
|--------|-------------|
| `even-horizontal` | Equal-width columns |
| `even-vertical` | Equal-height rows |
| `main-vertical` | Primary pane left, others stacked right |
| `main-horizontal` | Primary pane top, others side-by-side below |
| `tiled` | Grid layout (as square as possible) |

```lua
-- Keybinding
{ key = "1", mods = "ALT", action = act.SetTabLayout("even-horizontal") }
```

```bash
# CLI
wezterm cli set-tab-layout tiled
```

### Split Divider Percentage Indicator

A floating percentage label appears centered on the divider while dragging splits, giving precise visual feedback on pane sizing. Controlled by `show_split_size_indicator` (default: `true`).

### All Upstream WezTerm Features

Everything from upstream WezTerm is preserved:

- GPU-accelerated rendering (wgpu)
- Lua-based configuration with hot-reload
- Built-in multiplexer and SSH domains
- Cross-platform: Windows, macOS, Linux
- Font shaping with HarfBuzz, ligature support
- Hyperlinks, image protocol support, and more

See the full feature list at [wezterm.org](https://wezterm.org/).

## Quick Start

### Building from Source

```bash
git clone --recurse-submodules <this-repo>
cd wezterm
cargo build --release -p wezterm-gui
```

The `tmux-compat-shim` and `env-shim` binaries are built alongside:

```bash
cargo build --release -p tmux-compat-shim
cargo build --release -p env-shim
```

For upstream installation and platform-specific instructions, see the [WezTerm installation docs](https://wezterm.org/installation).

### Tmux-Compat Shim Setup

Place the built `tmux` shim binary on your `PATH` (before any real tmux, if installed). Claude Code agents will then use standard `tmux` commands that automatically route through WezTerm's CC protocol server.

The shim reads the `WEZTERM_TMUX_CC` environment variable for the socket path.

## Configuration

Fork-specific options in your `.wezterm.lua`:

```lua
local config = wezterm.config_builder()

-- Per-pane header colors
config.pane_header_active_fg_color = "#ffffff"
config.pane_header_active_bg_color = "#336699"
config.pane_header_inactive_fg_color = "#aaaaaa"
config.pane_header_inactive_bg_color = "#333333"

-- Split drag indicator
config.show_split_size_indicator = true  -- default
```

All standard WezTerm configuration options continue to work as documented at [wezterm.org/config](https://wezterm.org/docs/config/lua/config/).

## Acknowledgments

This project is built on top of [wezterm/wezterm](https://github.com/wezterm/wezterm) by Wez Furlong and contributors. All credit for the core terminal emulator, GPU rendering pipeline, multiplexer architecture, Lua config system, and cross-platform windowing goes to the upstream project.
