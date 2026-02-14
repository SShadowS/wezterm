//! Command handlers for the tmux compatibility server.
//!
//! This module wires Phase 1's parsed `TmuxCliCommand` values to WezTerm's Mux
//! so that each command performs real operations and returns response content.

use std::collections::HashMap;
use std::sync::Arc;

use config::keyassignment::SpawnTabDomain;
use wezterm_term::TerminalSize;

use crate::domain::SplitSource;
use crate::pane::PaneId;
use crate::tab::{SplitDirection, SplitRequest, SplitSize, Tab};
use crate::pane::CachePolicy;
use crate::window::WindowId;
use crate::Mux;

use super::command_parser::TmuxCliCommand;
use super::format::{expand_format, FormatContext};
use super::id_map::IdMap;
use super::response::session_changed_notification;
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
    /// Notifications to send after the current command response.
    pub pending_notifications: Vec<String>,
    /// Set by `detach-client` to signal the server loop to close.
    pub detach_requested: bool,
    /// Last-known active tab per mux window, for `%session-window-changed` detection.
    pub last_active_tab: HashMap<WindowId, crate::tab::TabId>,
    /// Suppression counter for `%session-window-changed` — incremented when we
    /// send `select-window` ourselves, to prevent notification feedback loops.
    pub suppress_window_changed: u32,
    /// Client name for `#{client_name}` format variable.
    pub client_name: String,
    /// Listen address for `#{socket_path}` format variable.
    pub socket_path: String,
}

impl HandlerContext {
    pub fn new(workspace: String) -> Self {
        Self {
            id_map: IdMap::new(),
            active_pane_id: None,
            active_window_id: None,
            active_session_id: None,
            workspace,
            pending_notifications: Vec::new(),
            detach_requested: false,
            last_active_tab: HashMap::new(),
            suppress_window_changed: 0,
            client_name: String::new(),
            socket_path: String::new(),
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

    // Phase 10: pane metadata
    let pane_title = pp.pane.get_title();
    let pane_current_command = pp
        .pane
        .get_foreground_process_name(CachePolicy::AllowStale)
        .unwrap_or_default();
    let pane_current_path = pp
        .pane
        .get_current_working_dir(CachePolicy::AllowStale)
        .map(|url| url.path().to_string())
        .unwrap_or_default();
    let pane_pid = pp
        .pane
        .get_foreground_process_info(CachePolicy::AllowStale)
        .map(|info| info.pid as u64)
        .unwrap_or(0);

    // Phase 10: window flags — tmux uses *=current, -=last, Z=zoomed
    let mut flags = String::new();
    // window_active is set by caller, but we can detect zoom here
    if tab.get_zoomed_pane().is_some() {
        flags.push('Z');
    }

    // Phase 10: window pane count
    let window_panes = tab.count_panes().unwrap_or(1) as u64;

    // Phase 10: session window count
    let session_windows = Mux::try_get()
        .map(|mux| mux.iter_windows_in_workspace(workspace).len() as u64)
        .unwrap_or(0);

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
        pane_title,
        pane_current_command,
        pane_current_path,
        pane_pid,
        pane_mode: String::new(), // No pane mode infrastructure yet
        window_flags: flags,
        window_panes,
        session_windows,
        session_attached: 1, // Single-client CC
        client_name: ctx.client_name.clone(),
        socket_path: ctx.socket_path.clone(),
        server_pid: std::process::id() as u64,
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
            zoom,
        } => handle_resize_pane(ctx, &target, width, height, zoom),
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
        TmuxCliCommand::KillWindow { target } => handle_kill_window(ctx, &target),
        TmuxCliCommand::KillSession { target } => handle_kill_session(ctx, &target),
        TmuxCliCommand::RenameWindow { target, name } => {
            handle_rename_window(ctx, &target, &name)
        }
        TmuxCliCommand::RenameSession { target, name } => {
            handle_rename_session(ctx, &target, &name)
        }
        TmuxCliCommand::NewSession { name } => {
            handle_new_session(ctx, name.as_deref()).await
        }
        TmuxCliCommand::ShowOptions {
            global,
            value_only,
            option_name,
        } => handle_show_options(global, value_only, option_name.as_deref()),
        TmuxCliCommand::ShowWindowOptions {
            global,
            value_only,
            option_name,
        } => handle_show_window_options(global, value_only, option_name.as_deref()),
        TmuxCliCommand::AttachSession { target } => handle_attach_session(ctx, &target),
        TmuxCliCommand::DetachClient => handle_detach_client(ctx),
        TmuxCliCommand::SwitchClient { target } => handle_attach_session(ctx, &target),
        TmuxCliCommand::ListClients { format, target: _ } => {
            handle_list_clients(ctx, format.as_deref())
        }
    }
}

// ---------------------------------------------------------------------------
// Stateless handlers
// ---------------------------------------------------------------------------

/// Returns a sorted list of all supported commands.
pub fn handle_list_commands() -> String {
    let mut commands = vec![
        "attach-session",
        "capture-pane",
        "detach-client",
        "display-message",
        "has-session",
        "kill-pane",
        "kill-session",
        "kill-window",
        "list-clients",
        "list-commands",
        "list-panes",
        "list-sessions",
        "list-windows",
        "new-session",
        "new-window",
        "refresh-client",
        "rename-session",
        "rename-window",
        "resize-pane",
        "resize-window",
        "select-pane",
        "select-window",
        "send-keys",
        "show-options",
        "show-window-options",
        "split-window",
        "switch-client",
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
                fctx.set_window_active(is_active_tab);
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
                fctx.set_window_active(is_active_tab);
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
                    fctx.set_window_active(is_active_tab);
                    lines.push(expand_format(fmt, &fctx));
                } else {
                    // Tab with no panes — build minimal context
                    let tab_size = tab.get_size();
                    let tmux_wid = ctx.id_map.get_or_create_tmux_window_id(tab.tab_id());
                    let tmux_sid = ctx.id_map.get_or_create_tmux_session_id(workspace);
                    let mut fctx = FormatContext {
                        window_id: tmux_wid,
                        window_index: window_index as u64,
                        window_name: tab.get_title(),
                        window_width: tab_size.cols as u64,
                        window_height: tab_size.rows as u64,
                        session_id: tmux_sid,
                        session_name: workspace.to_string(),
                        client_name: ctx.client_name.clone(),
                        socket_path: ctx.socket_path.clone(),
                        server_pid: std::process::id() as u64,
                        session_attached: 1,
                        ..FormatContext::default()
                    };
                    fctx.set_window_active(is_active_tab);
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

    // Suppress the resulting WindowInvalidated → %session-window-changed
    // notification to prevent a feedback loop (like iTerm2's approach).
    ctx.suppress_window_changed += 1;

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

    // Update context and last-known active tab
    ctx.active_window_id = ctx.id_map.tmux_window_id(tab_id);
    ctx.last_active_tab.insert(wid, tab_id);
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

/// Resize a pane, or toggle zoom if `-Z` was specified.
pub fn handle_resize_pane(
    ctx: &mut HandlerContext,
    target: &Option<String>,
    width: Option<u64>,
    height: Option<u64>,
    zoom: bool,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    if zoom {
        let resolved = ctx.resolve_target(target)?;
        let tab_id = resolved
            .tab_id
            .ok_or_else(|| "no window resolved for zoom".to_string())?;
        let tab = mux
            .get_tab(tab_id)
            .ok_or_else(|| format!("tab {} not found", tab_id))?;
        tab.toggle_zoom();
        return Ok(String::new());
    }

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
// Phase 7 handlers
// ---------------------------------------------------------------------------

/// Kill (remove) a window (tab).
pub fn handle_kill_window(
    ctx: &mut HandlerContext,
    target: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let tab_id = resolved
        .tab_id
        .ok_or_else(|| "no window resolved".to_string())?;

    // Clean up pane mappings for all panes in this tab
    if let Some(tab) = mux.get_tab(tab_id) {
        for pp in tab.iter_panes() {
            ctx.id_map.remove_pane(pp.pane.pane_id());
        }
    }

    // Clean up window mapping
    ctx.id_map.remove_window(tab_id);

    // Remove the tab (this also prunes empty mux windows)
    mux.remove_tab(tab_id);

    Ok(String::new())
}

/// Kill (remove) a session (all windows in a workspace).
pub fn handle_kill_session(
    ctx: &mut HandlerContext,
    target: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let workspace = resolved
        .workspace
        .unwrap_or_else(|| ctx.workspace.clone());

    // Collect all mux windows in the workspace
    let window_ids = mux.iter_windows_in_workspace(&workspace);

    // Clean up id_map entries for all tabs and panes in those windows
    for &wid in &window_ids {
        let tabs: Vec<Arc<Tab>> = {
            match mux.get_window(wid) {
                Some(win) => win.iter().map(Arc::clone).collect(),
                None => continue,
            }
        };
        for tab in &tabs {
            for pp in tab.iter_panes() {
                ctx.id_map.remove_pane(pp.pane.pane_id());
            }
            ctx.id_map.remove_window(tab.tab_id());
        }
    }

    // Kill all mux windows in the workspace
    for wid in window_ids {
        mux.kill_window(wid);
    }

    // Clean up session mapping
    ctx.id_map.remove_session(&workspace);

    Ok(String::new())
}

/// Rename a window (tab title).
pub fn handle_rename_window(
    ctx: &mut HandlerContext,
    target: &Option<String>,
    name: &str,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let tab_id = resolved
        .tab_id
        .ok_or_else(|| "no window resolved".to_string())?;

    let tab = mux
        .get_tab(tab_id)
        .ok_or_else(|| format!("tab {} not found", tab_id))?;

    tab.set_title(name);

    Ok(String::new())
}

/// Rename a session (workspace).
pub fn handle_rename_session(
    ctx: &mut HandlerContext,
    target: &Option<String>,
    name: &str,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let old_workspace = resolved
        .workspace
        .unwrap_or_else(|| ctx.workspace.clone());

    mux.rename_workspace(&old_workspace, name);
    ctx.id_map.rename_session(&old_workspace, name);

    // Update context workspace if it was the one renamed
    if ctx.workspace == old_workspace {
        ctx.workspace = name.to_string();
    }

    Ok(String::new())
}

/// Create a new session (workspace with a new window).
pub async fn handle_new_session(
    ctx: &mut HandlerContext,
    name: Option<&str>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let workspace = name.unwrap_or("default").to_string();

    // Check if workspace already exists
    if mux.iter_workspaces().contains(&workspace) {
        return Err(format!("duplicate session: {}", workspace));
    }

    let (tab, pane, _wid) = mux
        .spawn_tab_or_window(
            None, // create a new mux window
            SpawnTabDomain::CurrentPaneDomain,
            None,
            None,
            TerminalSize::default(),
            None, // no current pane
            workspace.clone(),
            None,
        )
        .await
        .map_err(|e| format!("new-session failed: {}", e))?;

    // Register new mappings
    let tmux_session_id = ctx.id_map.get_or_create_tmux_session_id(&workspace);
    let tmux_window_id = ctx.id_map.get_or_create_tmux_window_id(tab.tab_id());
    let tmux_pane_id = ctx.id_map.get_or_create_tmux_pane_id(pane.pane_id());

    ctx.active_session_id = Some(tmux_session_id);
    ctx.active_window_id = Some(tmux_window_id);
    ctx.active_pane_id = Some(tmux_pane_id);
    ctx.workspace = workspace;

    Ok(String::new())
}

/// Return hardcoded tmux server options.
///
/// iTerm2 queries: `show -gv default-terminal`, `show -gv escape-time`,
/// `show -gv set-clipboard`.
pub fn handle_show_options(
    global: bool,
    value_only: bool,
    option_name: Option<&str>,
) -> Result<String, String> {
    // Known global server options with sensible defaults
    let options: &[(&str, &str)] = &[
        ("default-terminal", "screen-256color"),
        ("escape-time", "500"),
        ("set-clipboard", "on"),
    ];

    if global {
        match option_name {
            Some(name) => {
                if let Some((_, value)) = options.iter().find(|(k, _)| *k == name) {
                    if value_only {
                        Ok(value.to_string())
                    } else {
                        Ok(format!("{} {}", name, value))
                    }
                } else {
                    Err(format!("unknown option: {}", name))
                }
            }
            None => {
                // Return all known options
                let lines: Vec<String> = options
                    .iter()
                    .map(|(k, v)| {
                        if value_only {
                            v.to_string()
                        } else {
                            format!("{} {}", k, v)
                        }
                    })
                    .collect();
                Ok(lines.join("\n"))
            }
        }
    } else {
        // Non-global options: we don't track per-session options
        match option_name {
            Some(name) => Err(format!("unknown option: {}", name)),
            None => Ok(String::new()),
        }
    }
}

/// Return hardcoded tmux window options.
///
/// iTerm2 queries: `showw -gv aggressive-resize`.
pub fn handle_show_window_options(
    global: bool,
    value_only: bool,
    option_name: Option<&str>,
) -> Result<String, String> {
    let options: &[(&str, &str)] = &[
        ("aggressive-resize", "off"),
        ("mode-keys", "emacs"),
    ];

    if global {
        match option_name {
            Some(name) => {
                if let Some((_, value)) = options.iter().find(|(k, _)| *k == name) {
                    if value_only {
                        Ok(value.to_string())
                    } else {
                        Ok(format!("{} {}", name, value))
                    }
                } else {
                    Err(format!("unknown option: {}", name))
                }
            }
            None => {
                let lines: Vec<String> = options
                    .iter()
                    .map(|(k, v)| {
                        if value_only {
                            v.to_string()
                        } else {
                            format!("{} {}", k, v)
                        }
                    })
                    .collect();
                Ok(lines.join("\n"))
            }
        }
    } else {
        match option_name {
            Some(name) => Err(format!("unknown option: {}", name)),
            None => Ok(String::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 8 handlers — session/client management
// ---------------------------------------------------------------------------

/// Attach to (switch to) a different session (workspace).
///
/// Resolves the target workspace, updates context, re-registers windows/panes
/// in the id_map, and queues a `%session-changed` notification to be sent
/// after the command response.
pub fn handle_attach_session(
    ctx: &mut HandlerContext,
    target: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    // Resolve target workspace
    let new_workspace = match target {
        Some(t) => {
            let parsed = parse_target(t).map_err(|e| e.to_string())?;
            match parsed.session {
                Some(SessionRef::Name(name)) => {
                    if mux.iter_workspaces().contains(&name) {
                        name
                    } else {
                        return Err(format!("can't find session: {}", name));
                    }
                }
                Some(SessionRef::Id(id)) => {
                    let ws = ctx
                        .id_map
                        .workspace_name(id)
                        .ok_or_else(|| format!("session ${} not found", id))?;
                    ws.to_string()
                }
                None => ctx.workspace.clone(),
            }
        }
        None => return Err("attach-session: no target specified".to_string()),
    };

    if new_workspace == ctx.workspace {
        // Already on this session — no-op
        return Ok(String::new());
    }

    // Switch context to the new workspace
    ctx.workspace = new_workspace.clone();

    // Ensure the session is registered in id_map
    let tmux_sid = ctx.id_map.get_or_create_tmux_session_id(&new_workspace);
    ctx.active_session_id = Some(tmux_sid);

    // Register windows and panes in the new workspace
    let window_ids = mux.iter_windows_in_workspace(&new_workspace);
    let mut first_tab = true;
    for &wid in &window_ids {
        let tabs: Vec<Arc<Tab>> = match mux.get_window(wid) {
            Some(win) => win.iter().map(Arc::clone).collect(),
            None => continue,
        };
        for tab in &tabs {
            let tmux_wid = ctx.id_map.get_or_create_tmux_window_id(tab.tab_id());
            if first_tab {
                ctx.active_window_id = Some(tmux_wid);
                if let Some(pane) = tab.get_active_pane() {
                    let tmux_pid = ctx.id_map.get_or_create_tmux_pane_id(pane.pane_id());
                    ctx.active_pane_id = Some(tmux_pid);
                }
                first_tab = false;
            }
            for pp in tab.iter_panes() {
                ctx.id_map.get_or_create_tmux_pane_id(pp.pane.pane_id());
            }
        }
    }

    // Queue %session-changed to be sent after the response block
    ctx.pending_notifications
        .push(session_changed_notification(tmux_sid, &new_workspace));

    Ok(String::new())
}

/// Detach the CC client.
///
/// Sets `detach_requested` so the server loop sends `%exit` and closes.
pub fn handle_detach_client(ctx: &mut HandlerContext) -> Result<String, String> {
    ctx.detach_requested = true;
    Ok(String::new())
}

/// List connected clients.
///
/// In our CC server there is always exactly one client — the CC connection
/// itself.  We return a single line with format variable expansion.
///
/// iTerm2 uses: `list-clients -t '$N' -F '#{client_name}\t#{client_control_mode}'`
pub fn handle_list_clients(ctx: &mut HandlerContext, format: Option<&str>) -> Result<String, String> {
    let default_format = "#{client_name}: #{session_name}";
    let fmt = format.unwrap_or(default_format);

    // Simple variable expansion for the client-related variables
    let session_name = &ctx.workspace;
    let tmux_sid = ctx.id_map.get_or_create_tmux_session_id(session_name);

    let output = fmt
        .replace("#{client_name}", "wezterm-cc")
        .replace("#{client_control_mode}", "1")
        .replace("#{session_name}", session_name)
        .replace("#{session_id}", &format!("${}", tmux_sid));

    Ok(output)
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
        assert_eq!(commands.len(), 27);
        assert!(commands.contains(&"attach-session"));
        assert!(commands.contains(&"capture-pane"));
        assert!(commands.contains(&"detach-client"));
        assert!(commands.contains(&"display-message"));
        assert!(commands.contains(&"has-session"));
        assert!(commands.contains(&"kill-pane"));
        assert!(commands.contains(&"kill-session"));
        assert!(commands.contains(&"kill-window"));
        assert!(commands.contains(&"list-clients"));
        assert!(commands.contains(&"list-commands"));
        assert!(commands.contains(&"list-panes"));
        assert!(commands.contains(&"list-sessions"));
        assert!(commands.contains(&"list-windows"));
        assert!(commands.contains(&"new-session"));
        assert!(commands.contains(&"new-window"));
        assert!(commands.contains(&"refresh-client"));
        assert!(commands.contains(&"rename-session"));
        assert!(commands.contains(&"rename-window"));
        assert!(commands.contains(&"resize-pane"));
        assert!(commands.contains(&"resize-window"));
        assert!(commands.contains(&"select-pane"));
        assert!(commands.contains(&"select-window"));
        assert!(commands.contains(&"send-keys"));
        assert!(commands.contains(&"show-options"));
        assert!(commands.contains(&"show-window-options"));
        assert!(commands.contains(&"split-window"));
        assert!(commands.contains(&"switch-client"));
    }

    #[test]
    fn list_commands_is_sorted() {
        let output = handle_list_commands();
        let commands: Vec<&str> = output.lines().collect();
        let mut sorted = commands.clone();
        sorted.sort();
        assert_eq!(commands, sorted);
    }

    // --- show-options tests ---

    #[test]
    fn show_options_global_value_default_terminal() {
        let result = handle_show_options(true, true, Some("default-terminal"));
        assert_eq!(result, Ok("screen-256color".to_string()));
    }

    #[test]
    fn show_options_global_value_escape_time() {
        let result = handle_show_options(true, true, Some("escape-time"));
        assert_eq!(result, Ok("500".to_string()));
    }

    #[test]
    fn show_options_global_value_set_clipboard() {
        let result = handle_show_options(true, true, Some("set-clipboard"));
        assert_eq!(result, Ok("on".to_string()));
    }

    #[test]
    fn show_options_global_key_value_format() {
        let result = handle_show_options(true, false, Some("default-terminal"));
        assert_eq!(result, Ok("default-terminal screen-256color".to_string()));
    }

    #[test]
    fn show_options_global_all() {
        let result = handle_show_options(true, false, None).unwrap();
        assert!(result.contains("default-terminal screen-256color"));
        assert!(result.contains("escape-time 500"));
        assert!(result.contains("set-clipboard on"));
    }

    #[test]
    fn show_options_unknown_option_is_error() {
        let result = handle_show_options(true, true, Some("nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn show_options_non_global_unknown_is_error() {
        let result = handle_show_options(false, false, Some("anything"));
        assert!(result.is_err());
    }

    // --- show-window-options tests ---

    #[test]
    fn show_window_options_aggressive_resize() {
        let result = handle_show_window_options(true, true, Some("aggressive-resize"));
        assert_eq!(result, Ok("off".to_string()));
    }

    #[test]
    fn show_window_options_mode_keys() {
        let result = handle_show_window_options(true, true, Some("mode-keys"));
        assert_eq!(result, Ok("emacs".to_string()));
    }

    #[test]
    fn show_window_options_key_value_format() {
        let result = handle_show_window_options(true, false, Some("aggressive-resize"));
        assert_eq!(result, Ok("aggressive-resize off".to_string()));
    }

    #[test]
    fn show_window_options_unknown_is_error() {
        let result = handle_show_window_options(true, true, Some("nonexistent"));
        assert!(result.is_err());
    }

    // --- Phase 8: detach-client tests ---

    #[test]
    fn detach_client_sets_flag() {
        let mut ctx = HandlerContext::new("default".to_string());
        assert!(!ctx.detach_requested);
        let result = handle_detach_client(&mut ctx);
        assert_eq!(result, Ok(String::new()));
        assert!(ctx.detach_requested);
    }

    // --- Phase 8: list-clients tests ---

    #[test]
    fn list_clients_default_format() {
        let mut ctx = HandlerContext::new("mywork".to_string());
        let result = handle_list_clients(&mut ctx, None).unwrap();
        assert_eq!(result, "wezterm-cc: mywork");
    }

    #[test]
    fn list_clients_iterm2_format() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result =
            handle_list_clients(&mut ctx, Some("#{client_name}\t#{client_control_mode}")).unwrap();
        assert_eq!(result, "wezterm-cc\t1");
    }

    #[test]
    fn list_clients_session_id_expansion() {
        let mut ctx = HandlerContext::new("work".to_string());
        // Pre-register the session so we know the ID
        let sid = ctx.id_map.get_or_create_tmux_session_id("work");
        let result = handle_list_clients(&mut ctx, Some("#{session_id}")).unwrap();
        assert_eq!(result, format!("${}", sid));
    }

    // --- Phase 8: attach-session tests (no-Mux, error path) ---

    #[test]
    fn attach_session_no_target_is_error() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result = handle_attach_session(&mut ctx, &None);
        assert!(result.is_err());
    }

    // --- Phase 8: pending_notifications / detach_requested defaults ---

    #[test]
    fn handler_context_defaults() {
        let ctx = HandlerContext::new("test".to_string());
        assert!(ctx.pending_notifications.is_empty());
        assert!(!ctx.detach_requested);
    }
}
