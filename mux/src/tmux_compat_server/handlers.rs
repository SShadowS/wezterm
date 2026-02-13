//! Command handlers for the tmux compatibility server.
//!
//! This module wires Phase 1's parsed `TmuxCliCommand` values to WezTerm's Mux
//! so that each command performs real operations and returns response content.

use std::sync::Arc;

use config::keyassignment::SpawnTabDomain;
use wezterm_term::TerminalSize;

use crate::domain::SplitSource;
use crate::pane::PaneId;
use crate::tab::{SplitDirection, SplitRequest, SplitSize, Tab};
use crate::window::WindowId;
use crate::Mux;

use super::command_parser::TmuxCliCommand;
use super::format::{expand_format, FormatContext};
use super::id_map::IdMap;
use super::target::{parse_target, PaneRef, SessionRef, TmuxTarget, WindowRef};

/// Resolved WezTerm IDs from a tmux target specification.
#[derive(Debug, Default)]
pub struct ResolvedTarget {
    pub pane_id: Option<PaneId>,
    pub tab_id: Option<crate::tab::TabId>,
    pub window_id: Option<WindowId>,
    pub workspace: Option<String>,
}

/// Per-client connection state for the tmux compat server.
pub struct HandlerContext {
    pub id_map: IdMap,
    pub active_pane_id: Option<u64>,
    pub active_window_id: Option<u64>,
    pub active_session_id: Option<u64>,
    pub workspace: String,
}

impl HandlerContext {
    pub fn new(workspace: String) -> Self {
        Self {
            id_map: IdMap::new(),
            active_pane_id: None,
            active_window_id: None,
            active_session_id: None,
            workspace,
        }
    }
}

// ---------------------------------------------------------------------------
// Key resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a single named key to its byte sequence.
pub fn resolve_named_key(name: &str) -> Option<Vec<u8>> {
    match name {
        "Enter" | "CR" => Some(b"\r".to_vec()),
        "Space" => Some(b" ".to_vec()),
        "Tab" | "BTab" => Some(b"\t".to_vec()),
        "Escape" => Some(b"\x1b".to_vec()),
        "BSpace" => Some(b"\x7f".to_vec()),
        "Up" => Some(b"\x1b[A".to_vec()),
        "Down" => Some(b"\x1b[B".to_vec()),
        "Right" => Some(b"\x1b[C".to_vec()),
        "Left" => Some(b"\x1b[D".to_vec()),
        "Home" => Some(b"\x1b[H".to_vec()),
        "End" => Some(b"\x1b[F".to_vec()),
        "Insert" => Some(b"\x1b[2~".to_vec()),
        "Delete" | "DC" => Some(b"\x1b[3~".to_vec()),
        "PageUp" | "PgUp" | "PPage" => Some(b"\x1b[5~".to_vec()),
        "PageDown" | "PgDn" | "NPage" => Some(b"\x1b[6~".to_vec()),
        "F1" => Some(b"\x1bOP".to_vec()),
        "F2" => Some(b"\x1bOQ".to_vec()),
        "F3" => Some(b"\x1bOR".to_vec()),
        "F4" => Some(b"\x1bOS".to_vec()),
        "F5" => Some(b"\x1b[15~".to_vec()),
        "F6" => Some(b"\x1b[17~".to_vec()),
        "F7" => Some(b"\x1b[18~".to_vec()),
        "F8" => Some(b"\x1b[19~".to_vec()),
        "F9" => Some(b"\x1b[20~".to_vec()),
        "F10" => Some(b"\x1b[21~".to_vec()),
        "F11" => Some(b"\x1b[23~".to_vec()),
        "F12" => Some(b"\x1b[24~".to_vec()),
        _ => {
            // C-a through C-z: control characters
            if name.starts_with("C-") && name.len() == 3 {
                let ch = name.as_bytes()[2];
                if ch.is_ascii_lowercase() {
                    return Some(vec![ch - b'a' + 1]);
                }
            }
            None
        }
    }
}

/// Resolve a single key argument from send-keys to bytes.
///
/// If `hex` is true, the key is a hex-encoded byte value (e.g. "0x1b" or "1b").
/// If `literal` is true, the key is sent as literal UTF-8 text.
/// Otherwise, try named key resolution first, then fall back to literal.
pub fn resolve_key(key: &str, literal: bool, hex: bool) -> Result<Vec<u8>, String> {
    if hex {
        let hex_str = key.strip_prefix("0x").unwrap_or(key);
        let byte =
            u8::from_str_radix(hex_str, 16).map_err(|_| format!("invalid hex key: {}", key))?;
        return Ok(vec![byte]);
    }
    if literal {
        return Ok(key.as_bytes().to_vec());
    }
    // Try named key, fall back to literal
    if let Some(bytes) = resolve_named_key(key) {
        Ok(bytes)
    } else {
        Ok(key.as_bytes().to_vec())
    }
}

/// Parse a tmux split size specification.
///
/// `"50%"` → `SplitSize::Percent(50)`, `"20"` → `SplitSize::Cells(20)`.
/// Returns `SplitSize::default()` (50%) if `None`.
pub fn parse_split_size(size: Option<&str>) -> Result<SplitSize, String> {
    match size {
        None => Ok(SplitSize::default()),
        Some(s) => {
            if let Some(pct) = s.strip_suffix('%') {
                let n: u8 = pct
                    .parse()
                    .map_err(|_| format!("invalid percent size: {}", s))?;
                if n == 0 || n > 100 {
                    return Err(format!("percent out of range: {}", n));
                }
                Ok(SplitSize::Percent(n))
            } else {
                let n: usize = s.parse().map_err(|_| format!("invalid cell size: {}", s))?;
                if n == 0 {
                    return Err("cell size must be > 0".to_string());
                }
                Ok(SplitSize::Cells(n))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Target resolution
// ---------------------------------------------------------------------------

impl HandlerContext {
    /// Resolve a tmux target string to WezTerm IDs.
    pub fn resolve_target(&self, target: &Option<String>) -> Result<ResolvedTarget, String> {
        let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

        let tmux_target = match target {
            Some(t) => parse_target(t).map_err(|e| e.to_string())?,
            None => TmuxTarget::default(),
        };

        let mut resolved = ResolvedTarget::default();

        // Resolve session → workspace
        resolved.workspace = match &tmux_target.session {
            Some(SessionRef::Id(id)) => {
                let ws = self
                    .id_map
                    .workspace_name(*id)
                    .ok_or_else(|| format!("session $${} not found", id))?;
                Some(ws.to_string())
            }
            Some(SessionRef::Name(name)) => {
                let workspaces = mux.iter_workspaces();
                if workspaces.contains(name) {
                    Some(name.clone())
                } else {
                    return Err(format!("session '{}' not found", name));
                }
            }
            None => Some(self.workspace.clone()),
        };

        let workspace = resolved.workspace.as_deref().unwrap_or(&self.workspace);

        // Resolve window → tab
        let window_ids = mux.iter_windows_in_workspace(workspace);

        resolved.window_id = match &tmux_target.window {
            Some(WindowRef::Id(id)) => {
                let tab_id = self
                    .id_map
                    .wezterm_tab_id(*id)
                    .ok_or_else(|| format!("window @{} not found", id))?;
                // Find the mux window containing this tab
                let wid = mux
                    .window_containing_tab(tab_id)
                    .ok_or_else(|| format!("tab {} not found in any window", tab_id))?;
                resolved.tab_id = Some(tab_id);
                Some(wid)
            }
            Some(WindowRef::Index(idx)) => {
                let wid = window_ids
                    .get(*idx as usize)
                    .copied()
                    .ok_or_else(|| format!("window index {} out of range", idx))?;
                // Get the active tab in that window
                let window = mux
                    .get_window(wid)
                    .ok_or_else(|| format!("window {} not found", wid))?;
                resolved.tab_id = window.get_active().map(|t| t.tab_id());
                Some(wid)
            }
            Some(WindowRef::Name(name)) => {
                // Search for a tab by title in the workspace
                let mut found = None;
                for &wid in &window_ids {
                    if let Some(win) = mux.get_window(wid) {
                        for tab in win.iter() {
                            if tab.get_title() == *name {
                                found = Some((wid, tab.tab_id()));
                                break;
                            }
                        }
                        if found.is_some() {
                            break;
                        }
                    }
                }
                let (wid, tid) = found.ok_or_else(|| format!("window '{}' not found", name))?;
                resolved.tab_id = Some(tid);
                Some(wid)
            }
            None => {
                // Use active window in workspace, falling back to first
                if let Some(active_wid) = self.active_window_id {
                    // active_window_id is a tmux window id; find the mux window
                    if let Some(tab_id) = self.id_map.wezterm_tab_id(active_wid) {
                        if let Some(wid) = mux.window_containing_tab(tab_id) {
                            resolved.tab_id = Some(tab_id);
                            Some(wid)
                        } else {
                            window_ids.first().copied()
                        }
                    } else {
                        window_ids.first().copied()
                    }
                } else {
                    window_ids.first().copied()
                }
            }
        };

        // If we have a window but no tab yet, get the active tab
        if resolved.tab_id.is_none() {
            if let Some(wid) = resolved.window_id {
                if let Some(window) = mux.get_window(wid) {
                    resolved.tab_id = window.get_active().map(|t| t.tab_id());
                }
            }
        }

        // Resolve pane
        resolved.pane_id = match &tmux_target.pane {
            Some(PaneRef::Id(id)) => {
                let wez_id = self
                    .id_map
                    .wezterm_pane_id(*id)
                    .ok_or_else(|| format!("pane %{} not found", id))?;
                Some(wez_id)
            }
            Some(PaneRef::Index(idx)) => {
                if let Some(tab_id) = resolved.tab_id {
                    let tab = mux
                        .get_tab(tab_id)
                        .ok_or_else(|| format!("tab {} not found", tab_id))?;
                    let panes = tab.iter_panes();
                    let pp = panes
                        .get(*idx as usize)
                        .ok_or_else(|| format!("pane index {} out of range", idx))?;
                    Some(pp.pane.pane_id())
                } else {
                    return Err("no window resolved for pane index lookup".to_string());
                }
            }
            None => {
                // Use active pane
                if let Some(active_pid) = self.active_pane_id {
                    self.id_map.wezterm_pane_id(active_pid)
                } else if let Some(tab_id) = resolved.tab_id {
                    let tab = mux
                        .get_tab(tab_id)
                        .ok_or_else(|| format!("tab {} not found", tab_id))?;
                    tab.get_active_pane().map(|p| p.pane_id())
                } else {
                    None
                }
            }
        };

        Ok(resolved)
    }
}

// ---------------------------------------------------------------------------
// Format context builders
// ---------------------------------------------------------------------------

/// Build a `FormatContext` from a positioned pane and its surrounding context.
pub fn build_format_context(
    ctx: &mut HandlerContext,
    pp: &crate::tab::PositionedPane,
    tab: &Arc<Tab>,
    _window_id: WindowId,
    window_index: usize,
    workspace: &str,
) -> FormatContext {
    let tmux_pane_id = ctx.id_map.get_or_create_tmux_pane_id(pp.pane.pane_id());
    let tmux_window_id = ctx.id_map.get_or_create_tmux_window_id(tab.tab_id());
    let tmux_session_id = ctx.id_map.get_or_create_tmux_session_id(workspace);

    let dims = pp.pane.get_dimensions();
    let cursor = pp.pane.get_cursor_position();
    let tab_size = tab.get_size();

    FormatContext {
        pane_id: tmux_pane_id,
        pane_index: pp.index as u64,
        pane_width: pp.width as u64,
        pane_height: pp.height as u64,
        pane_active: pp.is_active,
        pane_left: pp.left as u64,
        pane_top: pp.top as u64,
        pane_dead: pp.pane.is_dead(),
        window_id: tmux_window_id,
        window_index: window_index as u64,
        window_name: tab.get_title(),
        window_active: false, // Will be set by caller if needed
        window_width: tab_size.cols as u64,
        window_height: tab_size.rows as u64,
        session_id: tmux_session_id,
        session_name: workspace.to_string(),
        cursor_x: cursor.x as u64,
        cursor_y: cursor.y as u64,
        history_limit: dims.scrollback_rows as u64,
        history_size: dims.physical_top.saturating_sub(dims.scrollback_top) as u64,
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Main dispatch: route a parsed TmuxCliCommand to the appropriate handler.
pub async fn dispatch_command(
    ctx: &mut HandlerContext,
    cmd: TmuxCliCommand,
) -> Result<String, String> {
    match cmd {
        TmuxCliCommand::ListCommands => Ok(handle_list_commands()),
        TmuxCliCommand::HasSession { target } => handle_has_session(ctx, &target),
        TmuxCliCommand::ListPanes {
            all,
            session,
            format,
            target,
        } => handle_list_panes(ctx, all, session, format.as_deref(), &target),
        TmuxCliCommand::ListWindows {
            all,
            format,
            target,
        } => handle_list_windows(ctx, all, format.as_deref(), &target),
        TmuxCliCommand::ListSessions { format } => handle_list_sessions(ctx, format.as_deref()),
        TmuxCliCommand::DisplayMessage { print: _, format } => {
            handle_display_message(ctx, format.as_deref())
        }
        TmuxCliCommand::CapturePane {
            print: _,
            target,
            escape: _,
            octal_escape: _,
            start_line,
            end_line,
        } => handle_capture_pane(ctx, &target, start_line, end_line),
        TmuxCliCommand::SendKeys {
            target,
            literal,
            hex,
            keys,
        } => handle_send_keys(ctx, &target, literal, hex, &keys),
        TmuxCliCommand::SelectPane { target } => handle_select_pane(ctx, &target),
        TmuxCliCommand::SelectWindow { target } => handle_select_window(ctx, &target),
        TmuxCliCommand::KillPane { target } => handle_kill_pane(ctx, &target),
        TmuxCliCommand::ResizePane {
            target,
            width,
            height,
        } => handle_resize_pane(ctx, &target, width, height),
        TmuxCliCommand::ResizeWindow {
            target,
            width,
            height,
        } => handle_resize_window(ctx, &target, width, height),
        TmuxCliCommand::RefreshClient { size, flags: _ } => {
            handle_refresh_client(ctx, size.as_deref())
        }
        TmuxCliCommand::SplitWindow {
            horizontal,
            vertical: _,
            target,
            size,
        } => handle_split_window(ctx, horizontal, &target, size.as_deref()).await,
        TmuxCliCommand::NewWindow { target, name } => {
            handle_new_window(ctx, &target, name.as_deref()).await
        }
    }
}

// ---------------------------------------------------------------------------
// Stateless handlers
// ---------------------------------------------------------------------------

/// Returns a sorted list of all supported commands.
pub fn handle_list_commands() -> String {
    let mut commands = vec![
        "capture-pane",
        "display-message",
        "has-session",
        "kill-pane",
        "list-commands",
        "list-panes",
        "list-sessions",
        "list-windows",
        "new-window",
        "refresh-client",
        "resize-pane",
        "resize-window",
        "select-pane",
        "select-window",
        "send-keys",
        "split-window",
    ];
    commands.sort();
    commands.join("\n")
}

/// Check whether a session (workspace) exists.
pub fn handle_has_session(ctx: &HandlerContext, target: &Option<String>) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;
    let workspaces = mux.iter_workspaces();

    let workspace_to_check = match target {
        Some(t) => {
            let parsed = parse_target(t).map_err(|e| e.to_string())?;
            match parsed.session {
                Some(SessionRef::Name(name)) => name,
                Some(SessionRef::Id(id)) => {
                    let ws = ctx
                        .id_map
                        .workspace_name(id)
                        .ok_or_else(|| format!("session $${} not found", id))?;
                    ws.to_string()
                }
                None => ctx.workspace.clone(),
            }
        }
        None => ctx.workspace.clone(),
    };

    if workspaces.contains(&workspace_to_check) {
        Ok(String::new())
    } else {
        Err(format!("can't find session: {}", workspace_to_check))
    }
}

// ---------------------------------------------------------------------------
// Read-only handlers
// ---------------------------------------------------------------------------

/// List panes. `-a` lists all panes across all sessions, `-s` lists all panes
/// in the session, default lists panes in the target window.
pub fn handle_list_panes(
    ctx: &mut HandlerContext,
    all: bool,
    session: bool,
    format: Option<&str>,
    target: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let default_format =
        "#{pane_index}: [#{pane_width}x#{pane_height}] %#{pane_id}#{?pane_active, (active),}";
    let fmt = format.unwrap_or(default_format);

    let mut lines = Vec::new();

    if all {
        // All panes across all workspaces
        for workspace in mux.iter_workspaces() {
            collect_panes_in_workspace(ctx, &mux, &workspace, fmt, &mut lines)?;
        }
    } else if session {
        // All panes in the session (workspace)
        let resolved = ctx.resolve_target(target)?;
        let workspace = resolved.workspace.unwrap_or_else(|| ctx.workspace.clone());
        collect_panes_in_workspace(ctx, &mux, &workspace, fmt, &mut lines)?;
    } else {
        // Panes in the target window only
        let resolved = ctx.resolve_target(target)?;
        if let Some(tab_id) = resolved.tab_id {
            let tab = mux
                .get_tab(tab_id)
                .ok_or_else(|| format!("tab {} not found", tab_id))?;
            let wid = resolved.window_id.unwrap_or(0);
            let workspace = resolved.workspace.unwrap_or_else(|| ctx.workspace.clone());

            // Determine window index
            let window_index = {
                let window_ids = mux.iter_windows_in_workspace(&workspace);
                window_ids.iter().position(|&w| w == wid).unwrap_or(0)
            };

            let active_tab_id = {
                mux.get_window(wid)
                    .and_then(|w| w.get_active().map(|t| t.tab_id()))
            };
            let is_active_tab = active_tab_id == Some(tab_id);

            for pp in tab.iter_panes() {
                let mut fctx = build_format_context(ctx, &pp, &tab, wid, window_index, &workspace);
                fctx.window_active = is_active_tab;
                lines.push(expand_format(fmt, &fctx));
            }
        }
    }

    Ok(lines.join("\n"))
}

fn collect_panes_in_workspace(
    ctx: &mut HandlerContext,
    mux: &Arc<Mux>,
    workspace: &str,
    fmt: &str,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    let window_ids = mux.iter_windows_in_workspace(workspace);
    for (window_index, &wid) in window_ids.iter().enumerate() {
        let tabs: Vec<Arc<Tab>> = {
            match mux.get_window(wid) {
                Some(win) => win.iter().map(Arc::clone).collect(),
                None => continue,
            }
        };

        let active_tab_id = mux.get_active_tab_for_window(wid).map(|t| t.tab_id());

        for tab in &tabs {
            let is_active_tab = active_tab_id == Some(tab.tab_id());
            for pp in tab.iter_panes() {
                let mut fctx = build_format_context(ctx, &pp, tab, wid, window_index, workspace);
                fctx.window_active = is_active_tab;
                lines.push(expand_format(fmt, &fctx));
            }
        }
    }
    Ok(())
}

/// List windows (tabs).
pub fn handle_list_windows(
    ctx: &mut HandlerContext,
    all: bool,
    format: Option<&str>,
    target: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let default_format = "#{window_index}: #{window_name} (#{window_width}x#{window_height})#{?window_active, (active),}";
    let fmt = format.unwrap_or(default_format);

    let mut lines = Vec::new();

    let workspaces: Vec<String> = if all {
        mux.iter_workspaces()
    } else {
        let resolved = ctx.resolve_target(target)?;
        let ws = resolved.workspace.unwrap_or_else(|| ctx.workspace.clone());
        vec![ws]
    };

    for workspace in &workspaces {
        let window_ids = mux.iter_windows_in_workspace(workspace);
        for (window_index, &wid) in window_ids.iter().enumerate() {
            let tabs: Vec<Arc<Tab>> = {
                match mux.get_window(wid) {
                    Some(win) => win.iter().map(Arc::clone).collect(),
                    None => continue,
                }
            };
            let active_tab_id = mux.get_active_tab_for_window(wid).map(|t| t.tab_id());

            for tab in &tabs {
                let is_active_tab = active_tab_id == Some(tab.tab_id());
                // Build a context from the first pane (or default)
                let panes = tab.iter_panes();
                if let Some(pp) = panes.first() {
                    let mut fctx = build_format_context(ctx, pp, tab, wid, window_index, workspace);
                    fctx.window_active = is_active_tab;
                    lines.push(expand_format(fmt, &fctx));
                } else {
                    // Tab with no panes — build minimal context
                    let tab_size = tab.get_size();
                    let tmux_wid = ctx.id_map.get_or_create_tmux_window_id(tab.tab_id());
                    let tmux_sid = ctx.id_map.get_or_create_tmux_session_id(workspace);
                    let fctx = FormatContext {
                        window_id: tmux_wid,
                        window_index: window_index as u64,
                        window_name: tab.get_title(),
                        window_active: is_active_tab,
                        window_width: tab_size.cols as u64,
                        window_height: tab_size.rows as u64,
                        session_id: tmux_sid,
                        session_name: workspace.to_string(),
                        ..FormatContext::default()
                    };
                    lines.push(expand_format(fmt, &fctx));
                }
            }
        }
    }

    Ok(lines.join("\n"))
}

/// List sessions (workspaces).
pub fn handle_list_sessions(
    ctx: &mut HandlerContext,
    format: Option<&str>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let default_format = "#{session_name}: #{session_id}";
    let fmt = format.unwrap_or(default_format);

    let mut lines = Vec::new();
    for workspace in mux.iter_workspaces() {
        let tmux_sid = ctx.id_map.get_or_create_tmux_session_id(&workspace);
        let fctx = FormatContext {
            session_id: tmux_sid,
            session_name: workspace.to_string(),
            ..FormatContext::default()
        };
        lines.push(expand_format(fmt, &fctx));
    }

    Ok(lines.join("\n"))
}

/// Display a message by expanding a format string against the active context.
pub fn handle_display_message(
    ctx: &mut HandlerContext,
    format: Option<&str>,
) -> Result<String, String> {
    let default_format = "#{session_name}:#{window_index}.#{pane_index}";
    let fmt = format.unwrap_or(default_format);

    // Build context from the current active pane
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(&None)?;
    if let (Some(pane_id), Some(tab_id), Some(wid)) =
        (resolved.pane_id, resolved.tab_id, resolved.window_id)
    {
        let workspace = resolved.workspace.unwrap_or_else(|| ctx.workspace.clone());
        let tab = mux
            .get_tab(tab_id)
            .ok_or_else(|| format!("tab {} not found", tab_id))?;
        let panes = tab.iter_panes();
        if let Some(pp) = panes.iter().find(|p| p.pane.pane_id() == pane_id) {
            let window_index = {
                let wids = mux.iter_windows_in_workspace(&workspace);
                wids.iter().position(|&w| w == wid).unwrap_or(0)
            };
            let fctx = build_format_context(ctx, pp, &tab, wid, window_index, &workspace);
            return Ok(expand_format(fmt, &fctx));
        }
    }

    // Fallback: expand with default context
    Ok(expand_format(fmt, &FormatContext::default()))
}

/// Capture pane content as text.
pub fn handle_capture_pane(
    ctx: &mut HandlerContext,
    target: &Option<String>,
    start_line: Option<i64>,
    end_line: Option<i64>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let pane_id = resolved
        .pane_id
        .ok_or_else(|| "no pane resolved".to_string())?;

    let pane = mux
        .get_pane(pane_id)
        .ok_or_else(|| format!("pane {} not found", pane_id))?;

    let dims = pane.get_dimensions();
    let viewport_rows = dims.viewport_rows as isize;
    let physical_top = dims.physical_top;

    // Resolve start/end lines relative to the visible area.
    // In tmux, line 0 is the first visible line, negative values are scrollback.
    let start = match start_line {
        Some(s) => physical_top + s as isize,
        None => physical_top,
    };
    let end = match end_line {
        Some(e) => physical_top + e as isize + 1,
        None => physical_top + viewport_rows,
    };

    if start >= end {
        return Ok(String::new());
    }

    let (_first_row, lines) = pane.get_lines(start..end);
    let mut output = String::new();
    for line in &lines {
        let text = line.as_str();
        output.push_str(text.trim_end());
        output.push('\n');
    }

    Ok(output)
}

// ---------------------------------------------------------------------------
// Write handlers
// ---------------------------------------------------------------------------

/// Send keys to a pane.
pub fn handle_send_keys(
    ctx: &mut HandlerContext,
    target: &Option<String>,
    literal: bool,
    hex: bool,
    keys: &[String],
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let pane_id = resolved
        .pane_id
        .ok_or_else(|| "no pane resolved".to_string())?;

    let pane = mux
        .get_pane(pane_id)
        .ok_or_else(|| format!("pane {} not found", pane_id))?;

    let mut all_bytes = Vec::new();
    for key in keys {
        let bytes = resolve_key(key, literal, hex)?;
        all_bytes.extend_from_slice(&bytes);
    }

    pane.writer()
        .write_all(&all_bytes)
        .map_err(|e| format!("failed to write to pane: {}", e))?;

    Ok(String::new())
}

/// Select (focus) a pane.
pub fn handle_select_pane(
    ctx: &mut HandlerContext,
    target: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let pane_id = resolved
        .pane_id
        .ok_or_else(|| "no pane resolved".to_string())?;

    mux.focus_pane_and_containing_tab(pane_id)
        .map_err(|e| format!("select-pane failed: {}", e))?;

    // Update active pane in context
    ctx.active_pane_id = ctx.id_map.tmux_pane_id(pane_id);

    Ok(String::new())
}

/// Select (activate) a window (tab).
pub fn handle_select_window(
    ctx: &mut HandlerContext,
    target: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let tab_id = resolved
        .tab_id
        .ok_or_else(|| "no window resolved".to_string())?;
    let wid = resolved
        .window_id
        .ok_or_else(|| "no window resolved".to_string())?;

    // Find the tab's index in the window and activate it
    {
        let mut window = mux
            .get_window_mut(wid)
            .ok_or_else(|| format!("window {} not found", wid))?;
        let idx = window
            .idx_by_id(tab_id)
            .ok_or_else(|| format!("tab {} not in window {}", tab_id, wid))?;
        window.save_and_then_set_active(idx);
    }

    // Update context
    ctx.active_window_id = ctx.id_map.tmux_window_id(tab_id);
    let tab = mux.get_tab(tab_id);
    if let Some(tab) = tab {
        if let Some(active_pane) = tab.get_active_pane() {
            ctx.active_pane_id = ctx.id_map.tmux_pane_id(active_pane.pane_id());
        }
    }

    Ok(String::new())
}

/// Kill (remove) a pane.
pub fn handle_kill_pane(
    ctx: &mut HandlerContext,
    target: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let pane_id = resolved
        .pane_id
        .ok_or_else(|| "no pane resolved".to_string())?;

    ctx.id_map.remove_pane(pane_id);
    mux.remove_pane(pane_id);

    Ok(String::new())
}

/// Resize a pane.
pub fn handle_resize_pane(
    ctx: &mut HandlerContext,
    target: &Option<String>,
    width: Option<u64>,
    height: Option<u64>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let pane_id = resolved
        .pane_id
        .ok_or_else(|| "no pane resolved".to_string())?;

    let pane = mux
        .get_pane(pane_id)
        .ok_or_else(|| format!("pane {} not found", pane_id))?;

    let dims = pane.get_dimensions();
    let new_cols = width.map(|w| w as usize).unwrap_or(dims.cols);
    let new_rows = height.map(|h| h as usize).unwrap_or(dims.viewport_rows);

    let size = TerminalSize {
        cols: new_cols,
        rows: new_rows,
        pixel_width: 0,
        pixel_height: 0,
        dpi: dims.dpi,
    };

    pane.resize(size)
        .map_err(|e| format!("resize-pane failed: {}", e))?;

    Ok(String::new())
}

/// Resize a window (all panes in tab).
pub fn handle_resize_window(
    ctx: &mut HandlerContext,
    target: &Option<String>,
    width: Option<u64>,
    height: Option<u64>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let tab_id = resolved
        .tab_id
        .ok_or_else(|| "no window resolved".to_string())?;

    let tab = mux
        .get_tab(tab_id)
        .ok_or_else(|| format!("tab {} not found", tab_id))?;

    let current_size = tab.get_size();
    let new_cols = width.map(|w| w as usize).unwrap_or(current_size.cols);
    let new_rows = height.map(|h| h as usize).unwrap_or(current_size.rows);

    let size = TerminalSize {
        cols: new_cols,
        rows: new_rows,
        pixel_width: 0,
        pixel_height: 0,
        dpi: current_size.dpi,
    };

    tab.resize(size);

    Ok(String::new())
}

/// Refresh client — parse `WxH` from size and resize all tabs in workspace.
pub fn handle_refresh_client(
    ctx: &mut HandlerContext,
    size: Option<&str>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    if let Some(size_str) = size {
        let parts: Vec<&str> = size_str.split(',').collect();
        if let Some(dim_str) = parts.first() {
            let dims: Vec<&str> = dim_str.split('x').collect();
            if dims.len() == 2 {
                let cols: usize = dims[0]
                    .parse()
                    .map_err(|_| format!("invalid width: {}", dims[0]))?;
                let rows: usize = dims[1]
                    .parse()
                    .map_err(|_| format!("invalid height: {}", dims[1]))?;

                let term_size = TerminalSize {
                    cols,
                    rows,
                    pixel_width: 0,
                    pixel_height: 0,
                    dpi: 0,
                };

                let window_ids = mux.iter_windows_in_workspace(&ctx.workspace);
                for wid in window_ids {
                    let tabs: Vec<Arc<Tab>> = {
                        match mux.get_window(wid) {
                            Some(win) => win.iter().map(Arc::clone).collect(),
                            None => continue,
                        }
                    };
                    for tab in tabs {
                        tab.resize(term_size);
                    }
                }
            }
        }
    }

    Ok(String::new())
}

// ---------------------------------------------------------------------------
// Async handlers
// ---------------------------------------------------------------------------

/// Split a window pane.
///
/// tmux `-h` = horizontal split (side by side) = WezTerm `SplitDirection::Horizontal`
/// default (no flag or `-v`) = vertical split (stacked) = WezTerm `SplitDirection::Vertical`
pub async fn handle_split_window(
    ctx: &mut HandlerContext,
    horizontal: bool,
    target: &Option<String>,
    size: Option<&str>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let pane_id = resolved
        .pane_id
        .ok_or_else(|| "no pane resolved for split".to_string())?;

    let split_size = parse_split_size(size)?;

    let direction = if horizontal {
        SplitDirection::Horizontal
    } else {
        SplitDirection::Vertical
    };

    let request = SplitRequest {
        direction,
        target_is_second: true,
        top_level: false,
        size: split_size,
    };

    let source = SplitSource::Spawn {
        command: None,
        command_dir: None,
    };

    let (new_pane, _new_size) = mux
        .split_pane(pane_id, request, source, SpawnTabDomain::CurrentPaneDomain)
        .await
        .map_err(|e| format!("split-window failed: {}", e))?;

    // Register the new pane in the id map
    let tmux_pane_id = ctx.id_map.get_or_create_tmux_pane_id(new_pane.pane_id());
    ctx.active_pane_id = Some(tmux_pane_id);

    Ok(String::new())
}

/// Create a new window (tab).
pub async fn handle_new_window(
    ctx: &mut HandlerContext,
    target: &Option<String>,
    name: Option<&str>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    // Determine which mux window to add the tab to
    let resolved = ctx.resolve_target(target)?;
    let window_id = resolved.window_id;
    let workspace = resolved.workspace.unwrap_or_else(|| ctx.workspace.clone());

    let current_pane_id = resolved.pane_id;

    let (tab, pane, _wid) = mux
        .spawn_tab_or_window(
            window_id,
            SpawnTabDomain::CurrentPaneDomain,
            None,
            None,
            TerminalSize::default(),
            current_pane_id,
            workspace,
            None,
        )
        .await
        .map_err(|e| format!("new-window failed: {}", e))?;

    if let Some(title) = name {
        tab.set_title(title);
    }

    // Register new tab and pane
    let tmux_window_id = ctx.id_map.get_or_create_tmux_window_id(tab.tab_id());
    let tmux_pane_id = ctx.id_map.get_or_create_tmux_pane_id(pane.pane_id());
    ctx.active_window_id = Some(tmux_window_id);
    ctx.active_pane_id = Some(tmux_pane_id);

    Ok(String::new())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- resolve_named_key tests ---

    #[test]
    fn named_key_enter() {
        assert_eq!(resolve_named_key("Enter"), Some(b"\r".to_vec()));
        assert_eq!(resolve_named_key("CR"), Some(b"\r".to_vec()));
    }

    #[test]
    fn named_key_space() {
        assert_eq!(resolve_named_key("Space"), Some(b" ".to_vec()));
    }

    #[test]
    fn named_key_tab() {
        assert_eq!(resolve_named_key("Tab"), Some(b"\t".to_vec()));
    }

    #[test]
    fn named_key_escape() {
        assert_eq!(resolve_named_key("Escape"), Some(b"\x1b".to_vec()));
    }

    #[test]
    fn named_key_bspace() {
        assert_eq!(resolve_named_key("BSpace"), Some(b"\x7f".to_vec()));
    }

    #[test]
    fn named_key_arrows() {
        assert_eq!(resolve_named_key("Up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(resolve_named_key("Down"), Some(b"\x1b[B".to_vec()));
        assert_eq!(resolve_named_key("Right"), Some(b"\x1b[C".to_vec()));
        assert_eq!(resolve_named_key("Left"), Some(b"\x1b[D".to_vec()));
    }

    #[test]
    fn named_key_home_end() {
        assert_eq!(resolve_named_key("Home"), Some(b"\x1b[H".to_vec()));
        assert_eq!(resolve_named_key("End"), Some(b"\x1b[F".to_vec()));
    }

    #[test]
    fn named_key_function_keys() {
        assert_eq!(resolve_named_key("F1"), Some(b"\x1bOP".to_vec()));
        assert_eq!(resolve_named_key("F2"), Some(b"\x1bOQ".to_vec()));
        assert_eq!(resolve_named_key("F3"), Some(b"\x1bOR".to_vec()));
        assert_eq!(resolve_named_key("F4"), Some(b"\x1bOS".to_vec()));
        assert_eq!(resolve_named_key("F5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(resolve_named_key("F6"), Some(b"\x1b[17~".to_vec()));
        assert_eq!(resolve_named_key("F7"), Some(b"\x1b[18~".to_vec()));
        assert_eq!(resolve_named_key("F8"), Some(b"\x1b[19~".to_vec()));
        assert_eq!(resolve_named_key("F9"), Some(b"\x1b[20~".to_vec()));
        assert_eq!(resolve_named_key("F10"), Some(b"\x1b[21~".to_vec()));
        assert_eq!(resolve_named_key("F11"), Some(b"\x1b[23~".to_vec()));
        assert_eq!(resolve_named_key("F12"), Some(b"\x1b[24~".to_vec()));
    }

    #[test]
    fn named_key_page_up_down() {
        assert_eq!(resolve_named_key("PageUp"), Some(b"\x1b[5~".to_vec()));
        assert_eq!(resolve_named_key("PgUp"), Some(b"\x1b[5~".to_vec()));
        assert_eq!(resolve_named_key("PPage"), Some(b"\x1b[5~".to_vec()));
        assert_eq!(resolve_named_key("PageDown"), Some(b"\x1b[6~".to_vec()));
        assert_eq!(resolve_named_key("PgDn"), Some(b"\x1b[6~".to_vec()));
        assert_eq!(resolve_named_key("NPage"), Some(b"\x1b[6~".to_vec()));
    }

    #[test]
    fn named_key_insert_delete() {
        assert_eq!(resolve_named_key("Insert"), Some(b"\x1b[2~".to_vec()));
        assert_eq!(resolve_named_key("Delete"), Some(b"\x1b[3~".to_vec()));
        assert_eq!(resolve_named_key("DC"), Some(b"\x1b[3~".to_vec()));
    }

    #[test]
    fn named_key_ctrl_a_through_z() {
        assert_eq!(resolve_named_key("C-a"), Some(vec![1]));
        assert_eq!(resolve_named_key("C-c"), Some(vec![3]));
        assert_eq!(resolve_named_key("C-z"), Some(vec![26]));
    }

    #[test]
    fn named_key_unknown() {
        assert_eq!(resolve_named_key("FooBar"), None);
        assert_eq!(resolve_named_key(""), None);
    }

    // --- resolve_key tests ---

    #[test]
    fn resolve_key_hex_with_prefix() {
        assert_eq!(resolve_key("0x1b", false, true), Ok(vec![0x1b]));
    }

    #[test]
    fn resolve_key_hex_without_prefix() {
        assert_eq!(resolve_key("0d", false, true), Ok(vec![0x0d]));
    }

    #[test]
    fn resolve_key_hex_invalid() {
        assert!(resolve_key("zz", false, true).is_err());
    }

    #[test]
    fn resolve_key_literal() {
        assert_eq!(resolve_key("Enter", true, false), Ok(b"Enter".to_vec()));
    }

    #[test]
    fn resolve_key_named_fallback() {
        assert_eq!(resolve_key("Enter", false, false), Ok(b"\r".to_vec()));
    }

    #[test]
    fn resolve_key_plain_text_fallback() {
        assert_eq!(resolve_key("hello", false, false), Ok(b"hello".to_vec()));
    }

    // --- parse_split_size tests ---

    #[test]
    fn split_size_none_is_default() {
        assert_eq!(parse_split_size(None), Ok(SplitSize::default()));
    }

    #[test]
    fn split_size_percent() {
        assert_eq!(parse_split_size(Some("50%")), Ok(SplitSize::Percent(50)));
        assert_eq!(parse_split_size(Some("25%")), Ok(SplitSize::Percent(25)));
    }

    #[test]
    fn split_size_cells() {
        assert_eq!(parse_split_size(Some("20")), Ok(SplitSize::Cells(20)));
    }

    #[test]
    fn split_size_invalid_percent() {
        assert!(parse_split_size(Some("abc%")).is_err());
    }

    #[test]
    fn split_size_zero_percent() {
        assert!(parse_split_size(Some("0%")).is_err());
    }

    #[test]
    fn split_size_over_100_percent() {
        assert!(parse_split_size(Some("101%")).is_err());
    }

    #[test]
    fn split_size_zero_cells() {
        assert!(parse_split_size(Some("0")).is_err());
    }

    #[test]
    fn split_size_invalid_cells() {
        assert!(parse_split_size(Some("abc")).is_err());
    }

    // --- handle_list_commands tests ---

    #[test]
    fn list_commands_contains_all() {
        let output = handle_list_commands();
        let commands: Vec<&str> = output.lines().collect();
        assert_eq!(commands.len(), 16);
        assert!(commands.contains(&"capture-pane"));
        assert!(commands.contains(&"display-message"));
        assert!(commands.contains(&"has-session"));
        assert!(commands.contains(&"kill-pane"));
        assert!(commands.contains(&"list-commands"));
        assert!(commands.contains(&"list-panes"));
        assert!(commands.contains(&"list-sessions"));
        assert!(commands.contains(&"list-windows"));
        assert!(commands.contains(&"new-window"));
        assert!(commands.contains(&"refresh-client"));
        assert!(commands.contains(&"resize-pane"));
        assert!(commands.contains(&"resize-window"));
        assert!(commands.contains(&"select-pane"));
        assert!(commands.contains(&"select-window"));
        assert!(commands.contains(&"send-keys"));
        assert!(commands.contains(&"split-window"));
    }

    #[test]
    fn list_commands_is_sorted() {
        let output = handle_list_commands();
        let commands: Vec<&str> = output.lines().collect();
        let mut sorted = commands.clone();
        sorted.sort();
        assert_eq!(commands, sorted);
    }
}
