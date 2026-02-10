# WezTerm (Fork)

This is a fork of [wez/wezterm](https://github.com/wezterm/wezterm), a GPU-accelerated cross-platform terminal emulator and multiplexer written in Rust.

For upstream docs and installation, see: https://wezterm.org/

## Additions in this fork

### Per-pane header bars

Optional 1-row colored headers at the top of each pane, useful for identifying panes in split layouts.

- **Lua API**: `pane:set_header("label")` / `pane:get_header()`
- **Format callback**: `format-pane-header` event (works like `format-tab-title`)
- **Config options**: `pane_header_active_fg_color`, `pane_header_active_bg_color`, `pane_header_inactive_fg_color`, `pane_header_inactive_bg_color`

See [window.md](window.md) for full usage details.
