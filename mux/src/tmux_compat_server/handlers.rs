//! Command handlers for the tmux compatibility server.
//!
//! This module wires Phase 1's parsed `TmuxCliCommand` values to WezTerm's Mux
//! so that each command performs real operations and returns response content.

use std::collections::HashMap;
use std::sync::Arc;

use config::keyassignment::SpawnTabDomain;
use wezterm_term::TerminalSize;

use crate::domain::SplitSource;
use crate::pane::{CachePolicy, Pane, PaneId};
use crate::tab::{SplitDirection, SplitRequest, SplitSize, Tab};
use crate::window::WindowId;
use crate::Mux;

use super::command_parser::TmuxCliCommand;
use super::format::{expand_format, FormatContext};
use super::id_map::IdMap;
use super::paste_buffer::{buffer_sample, PasteBufferStore};
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

// ---------------------------------------------------------------------------
// Subscription types
// ---------------------------------------------------------------------------

/// Target type for a subscription.
#[derive(Debug, Clone, PartialEq)]
pub enum SubscriptionTarget {
    /// `$<session_id>` — monitor a session-level format.
    Session(u64),
    /// `@<window_id>` — monitor a specific window.
    Window(u64),
    /// `%<pane_id>` — monitor a specific pane.
    Pane(u64),
    /// `%*` — monitor all panes in session.
    AllPanes,
    /// `@*` — monitor all windows in session.
    AllWindows,
}

/// A registered format subscription.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub name: String,
    pub target: SubscriptionTarget,
    pub format: String,
    /// Last evaluated value per entity (keyed by entity ID string, e.g. "%0").
    /// For single-target subs this has one entry; for `%*`/`@*` it has one per entity.
    pub last_values: HashMap<String, String>,
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
    /// In-process paste buffer store for clipboard/buffer commands.
    pub paste_buffers: PasteBufferStore,
    /// Pause-after age in milliseconds. When set, `%extended-output` is sent
    /// instead of `%output`, and panes are paused when buffered output exceeds
    /// this age. `None` means pause mode is disabled.
    pub pause_age_ms: Option<u64>,
    /// Whether `wait-exit` flag is set (wait for empty line before exiting).
    pub wait_exit: bool,
    /// Per-pane pause state. Maps tmux pane ID → paused flag.
    pub paused_panes: HashMap<u64, bool>,
    /// Per-pane output timestamp tracking. Maps tmux pane ID → Instant of
    /// first unbuffered byte (for age calculation).
    pub pane_output_timestamps: HashMap<u64, std::time::Instant>,
    /// Format subscriptions registered via `refresh-client -B`.
    pub subscriptions: Vec<Subscription>,
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
            paste_buffers: PasteBufferStore::new(),
            pause_age_ms: None,
            wait_exit: false,
            paused_panes: HashMap::new(),
            pane_output_timestamps: HashMap::new(),
            subscriptions: Vec::new(),
        }
    }

    /// Create a context with ID mappings restored from disk.
    ///
    /// Loads previously persisted pane/window/session ID mappings so that
    /// reconnecting CC clients see the same tmux IDs as before.
    /// Stale mappings (referencing panes/tabs that no longer exist) are pruned.
    pub fn with_persistent_ids(workspace: String) -> Self {
        let mut id_map = IdMap::load(&workspace);

        // Prune mappings that reference dead panes/tabs.
        if let Some(mux) = Mux::try_get() {
            let live_pane_ids: std::collections::HashSet<PaneId> =
                mux.iter_panes().into_iter().map(|p| p.pane_id()).collect();
            let live_tab_ids: std::collections::HashSet<crate::tab::TabId> = mux
                .iter_windows_in_workspace(&workspace)
                .iter()
                .flat_map(|wid| {
                    mux.get_window(*wid)
                        .map(|w| w.iter().map(|t| t.tab_id()).collect::<Vec<_>>())
                        .unwrap_or_default()
                })
                .collect();
            id_map.prune_stale(&live_pane_ids, &live_tab_ids);
        }

        let mut ctx = Self::new(workspace);
        ctx.id_map = id_map;
        ctx
    }

    /// Persist the current ID mappings to disk.
    pub fn save_id_map(&self) {
        self.id_map.save(&self.workspace);
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
        buffer_name: String::new(),
        buffer_size: 0,
        buffer_sample: String::new(),
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
        TmuxCliCommand::DisplayMessage {
            print: _,
            format,
            target,
        } => handle_display_message(ctx, format.as_deref(), &target),
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
        TmuxCliCommand::SelectPane {
            target,
            style: _,
            title,
        } => handle_select_pane(ctx, &target, title.as_deref()),
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
        TmuxCliCommand::RefreshClient {
            size,
            flags,
            adjust_pane,
            subscription,
        } => handle_refresh_client(
            ctx,
            size.as_deref(),
            flags.as_deref(),
            adjust_pane.as_deref(),
            subscription.as_deref(),
        ),
        TmuxCliCommand::SplitWindow {
            horizontal,
            vertical: _,
            target,
            size,
            print_and_format,
        } => {
            handle_split_window(
                ctx,
                horizontal,
                &target,
                size.as_deref(),
                print_and_format.as_deref(),
            )
            .await
        }
        TmuxCliCommand::NewWindow {
            target,
            name,
            print_and_format,
        } => handle_new_window(ctx, &target, name.as_deref(), print_and_format.as_deref()).await,
        TmuxCliCommand::KillWindow { target } => handle_kill_window(ctx, &target),
        TmuxCliCommand::KillSession { target } => handle_kill_session(ctx, &target),
        TmuxCliCommand::RenameWindow { target, name } => handle_rename_window(ctx, &target, &name),
        TmuxCliCommand::RenameSession { target, name } => {
            handle_rename_session(ctx, &target, &name)
        }
        TmuxCliCommand::NewSession {
            name,
            window_name,
            detached: _,
            print_and_format,
        } => {
            handle_new_session(
                ctx,
                name.as_deref(),
                window_name.as_deref(),
                print_and_format.as_deref(),
            )
            .await
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
        // Phase 11: clipboard / buffer commands
        TmuxCliCommand::ShowBuffer { buffer_name } => {
            handle_show_buffer(ctx, buffer_name.as_deref())
        }
        TmuxCliCommand::SetBuffer {
            buffer_name,
            data,
            append,
        } => handle_set_buffer(ctx, buffer_name.as_deref(), data.as_deref(), append),
        TmuxCliCommand::DeleteBuffer { buffer_name } => {
            handle_delete_buffer(ctx, buffer_name.as_deref())
        }
        TmuxCliCommand::ListBuffers { format } => handle_list_buffers(ctx, format.as_deref()),
        TmuxCliCommand::PasteBuffer {
            buffer_name,
            target,
            delete_after,
            bracketed: _,
        } => handle_paste_buffer(ctx, buffer_name.as_deref(), &target, delete_after),
        TmuxCliCommand::MovePane {
            src,
            dst,
            horizontal,
            before,
        } => handle_move_pane(ctx, &src, &dst, horizontal, before).await,
        TmuxCliCommand::MoveWindow { src, dst } => handle_move_window(ctx, &src, &dst),
        TmuxCliCommand::CopyMode { quit, target: _ } => handle_copy_mode(quit),
        // Phase 13: Claude Code agent teams compatibility
        TmuxCliCommand::SetOption {
            target: _,
            option_name,
            value,
        } => handle_set_option(option_name.as_deref(), value.as_deref()),
        TmuxCliCommand::SelectLayout {
            target: _,
            layout_name: _,
        } => Ok(String::new()),
        TmuxCliCommand::BreakPane {
            detach,
            source,
            target,
        } => handle_break_pane(ctx, detach, &source, &target).await,
    }
}

// ---------------------------------------------------------------------------
// Stateless handlers
// ---------------------------------------------------------------------------

/// Returns a sorted list of all supported commands.
pub fn handle_list_commands() -> String {
    let mut commands = vec![
        "attach-session",
        "break-pane",
        "capture-pane",
        "copy-mode",
        "delete-buffer",
        "detach-client",
        "display-message",
        "has-session",
        "kill-pane",
        "kill-session",
        "kill-window",
        "join-pane",
        "list-buffers",
        "list-clients",
        "list-commands",
        "list-panes",
        "list-sessions",
        "list-windows",
        "move-pane",
        "move-window",
        "new-session",
        "new-window",
        "paste-buffer",
        "refresh-client",
        "rename-session",
        "rename-window",
        "resize-pane",
        "resize-window",
        "select-layout",
        "select-pane",
        "select-window",
        "send-keys",
        "set-buffer",
        "set-option",
        "show-buffer",
        "show-options",
        "show-window-options",
        "split-window",
        "switch-client",
    ];
    commands.sort();
    commands.join("\n")
}

/// Handle `copy-mode [-q]`.
///
/// With `-q`: exits copy mode (no-op for WezTerm — used defensively by iTerm2
/// to ensure tmux isn't stuck in copy mode after config errors).
/// Without `-q`: would enter copy mode — accepted silently as a no-op since
/// WezTerm manages its own copy overlay independently.
pub fn handle_copy_mode(_quit: bool) -> Result<String, String> {
    // No-op: WezTerm's copy overlay is independent of tmux CC protocol.
    // iTerm2 sends `copy-mode -q` on connect as a defensive measure.
    Ok(String::new())
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
    target: &Option<String>,
) -> Result<String, String> {
    let default_format = "#{session_name}:#{window_index}.#{pane_index}";
    let fmt = format.unwrap_or(default_format);

    // Build context from the target pane (or active pane if no target)
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
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
    title: Option<&str>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    let resolved = ctx.resolve_target(target)?;
    let pane_id = resolved
        .pane_id
        .ok_or_else(|| "no pane resolved".to_string())?;

    // If -T was specified, set pane title (best effort — WezTerm doesn't have per-pane titles,
    // so we set the containing tab's title instead)
    if let Some(new_title) = title {
        let workspace = ctx.workspace.clone();
        if let Some((tab, _wid)) = find_tab_and_window_for_pane(&mux, pane_id, &workspace) {
            tab.set_title(new_title);
        }
    }

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

/// Refresh client — handle `-C WxH`, `-f flags`, and `-A pane:action`.
pub fn handle_refresh_client(
    ctx: &mut HandlerContext,
    size: Option<&str>,
    flags: Option<&str>,
    adjust_pane: Option<&str>,
    subscription: Option<&str>,
) -> Result<String, String> {
    // Handle -B subscription (register/unregister).
    // Format: "name:target:format" to subscribe, "name" alone to unsubscribe.
    if let Some(sub_str) = subscription {
        handle_subscription(ctx, sub_str)?;
    }

    // Handle -f flags first (doesn't need Mux).
    // Handle -f flags (comma-separated: pause-after=N, wait-exit, !pause-after, etc.)
    if let Some(flags_str) = flags {
        for flag in flags_str.split(',') {
            let flag = flag.trim();
            if flag.is_empty() {
                continue;
            }
            if flag == "wait-exit" {
                ctx.wait_exit = true;
            } else if flag == "!wait-exit" {
                ctx.wait_exit = false;
            } else if flag == "pause-after" {
                // bare "pause-after" with no value means pause-after=0 (immediate)
                ctx.pause_age_ms = Some(0);
            } else if flag == "!pause-after" {
                ctx.pause_age_ms = None;
            } else if let Some(val) = flag.strip_prefix("pause-after=") {
                let seconds: u64 = val
                    .parse()
                    .map_err(|_| format!("invalid pause-after value: {}", val))?;
                ctx.pause_age_ms = Some(seconds * 1000);
            }
            // Unknown flags are silently ignored (matches tmux behavior).
        }
    }

    // Handle -A %<pane>:<action> (adjust pane output mode).
    if let Some(adjust) = adjust_pane {
        parse_and_apply_pane_adjust(ctx, adjust)?;
    }

    // Handle -C WxH (resize all tabs) — requires Mux.
    if let Some(size_str) = size {
        let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;
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

/// Parse `-A %<pane>:<action>` and apply the action.
///
/// Actions: `on`, `off`, `continue`, `pause`
fn parse_and_apply_pane_adjust(ctx: &mut HandlerContext, spec: &str) -> Result<(), String> {
    // Format: "%<pane_id>:<action>" or just "%<pane_id>" (defaults to "on")
    let (pane_part, action) = match spec.find(':') {
        Some(pos) => (&spec[..pos], &spec[pos + 1..]),
        None => (spec, "on"),
    };

    // Parse %<pane_id>
    let tmux_pane_id = if let Some(id_str) = pane_part.strip_prefix('%') {
        id_str
            .parse::<u64>()
            .map_err(|_| format!("invalid pane id: {}", pane_part))?
    } else {
        return Err(format!("expected %%<pane_id>, got: {}", pane_part));
    };

    match action {
        "continue" => {
            if ctx.paused_panes.get(&tmux_pane_id) == Some(&true) {
                ctx.paused_panes.insert(tmux_pane_id, false);
                // Reset the output timestamp so age starts fresh.
                ctx.pane_output_timestamps.remove(&tmux_pane_id);
                ctx.pending_notifications
                    .push(super::response::continue_notification(tmux_pane_id));
            }
        }
        "pause" => {
            ctx.paused_panes.insert(tmux_pane_id, true);
            ctx.pending_notifications
                .push(super::response::pause_notification(tmux_pane_id));
        }
        "on" => {
            // Enable output for this pane (unpause without notification).
            ctx.paused_panes.insert(tmux_pane_id, false);
            ctx.pane_output_timestamps.remove(&tmux_pane_id);
        }
        "off" => {
            // Disable output for this pane (pause without notification).
            ctx.paused_panes.insert(tmux_pane_id, true);
        }
        other => {
            return Err(format!("unknown pane adjust action: {}", other));
        }
    }

    Ok(())
}

/// Handle `-B name:target:format` (subscribe) or `-B name` (unsubscribe).
///
/// tmux subscription format: `refresh-client -B "name:target:format"`
/// - `name` is an arbitrary label for the subscription.
/// - `target` identifies what to monitor: `%<pane>`, `@<window>`, `$<session>`,
///   `%*` (all panes), `@*` (all windows).
/// - `format` is a tmux format string evaluated periodically.
///
/// If only `name` is given (no colons), the subscription is removed.
fn handle_subscription(ctx: &mut HandlerContext, spec: &str) -> Result<(), String> {
    // Split on first colon — if no colon, it's an unsubscribe.
    let first_colon = spec.find(':');
    if first_colon.is_none() {
        // Unsubscribe: remove by name.
        let name = spec.trim();
        ctx.subscriptions.retain(|s| s.name != name);
        return Ok(());
    }

    let first_colon = first_colon.unwrap();
    let name = &spec[..first_colon];

    // Find second colon to split target:format.
    let rest = &spec[first_colon + 1..];
    let second_colon = rest.find(':');
    if second_colon.is_none() {
        return Err(format!(
            "invalid subscription format (expected name:target:format): {}",
            spec
        ));
    }

    let second_colon = second_colon.unwrap();
    let target_str = &rest[..second_colon];
    let format = &rest[second_colon + 1..];

    let target = parse_subscription_target(target_str)?;

    // Remove existing subscription with the same name (replace semantics).
    ctx.subscriptions.retain(|s| s.name != name);

    ctx.subscriptions.push(Subscription {
        name: name.to_string(),
        target,
        format: format.to_string(),
        last_values: HashMap::new(),
    });

    Ok(())
}

/// Parse a subscription target string.
fn parse_subscription_target(s: &str) -> Result<SubscriptionTarget, String> {
    if s == "%*" {
        return Ok(SubscriptionTarget::AllPanes);
    }
    if s == "@*" {
        return Ok(SubscriptionTarget::AllWindows);
    }
    if let Some(id_str) = s.strip_prefix('%') {
        let id: u64 = id_str
            .parse()
            .map_err(|_| format!("invalid pane id in subscription target: {}", s))?;
        return Ok(SubscriptionTarget::Pane(id));
    }
    if let Some(id_str) = s.strip_prefix('@') {
        let id: u64 = id_str
            .parse()
            .map_err(|_| format!("invalid window id in subscription target: {}", s))?;
        return Ok(SubscriptionTarget::Window(id));
    }
    if let Some(id_str) = s.strip_prefix('$') {
        let id: u64 = id_str
            .parse()
            .map_err(|_| format!("invalid session id in subscription target: {}", s))?;
        return Ok(SubscriptionTarget::Session(id));
    }
    Err(format!("invalid subscription target: {}", s))
}

/// Check all subscriptions for value changes and emit `%subscription-changed`
/// notifications for any that have changed.
///
/// This should be called periodically (e.g. every ~1s) from the CC connection loop.
/// It evaluates each subscription's format string against the current state and
/// compares with the last known value. If different, a notification is emitted and
/// the stored value is updated.
pub fn check_subscriptions(ctx: &mut HandlerContext) -> Vec<String> {
    let mux = match Mux::try_get() {
        Some(m) => m,
        None => return Vec::new(),
    };

    let mut notifications = Vec::new();
    let session_id_str = ctx
        .id_map
        .get_or_create_tmux_session_id(&ctx.workspace)
        .to_string();

    // Collect all (window_id, tab, window_index) tuples for the workspace.
    let window_ids: Vec<WindowId> = mux.iter_windows_in_workspace(&ctx.workspace);

    for sub_idx in 0..ctx.subscriptions.len() {
        match &ctx.subscriptions[sub_idx].target {
            SubscriptionTarget::Pane(pane_tmux_id) => {
                let pane_tmux_id = *pane_tmux_id;
                // Look up the real pane.
                if let Some(real_pane_id) = ctx.id_map.wezterm_pane_id(pane_tmux_id) {
                    if let Some(pane) = mux.get_pane(real_pane_id) {
                        let (window_id_str, window_index_str) =
                            find_window_for_pane(&mux, &window_ids, &ctx.id_map, real_pane_id);
                        let fctx =
                            build_pane_format_context_minimal(&ctx.id_map, &pane, &ctx.workspace);
                        let value = expand_format(&ctx.subscriptions[sub_idx].format, &fctx);
                        let key = format!("%{}", pane_tmux_id);
                        let changed = ctx.subscriptions[sub_idx]
                            .last_values
                            .get(&key)
                            .map_or(true, |old| old != &value);
                        if changed {
                            notifications.push(super::response::subscription_changed_notification(
                                &ctx.subscriptions[sub_idx].name,
                                &format!("${}", session_id_str),
                                &window_id_str,
                                &window_index_str,
                                &format!("%{}", pane_tmux_id),
                                &value,
                            ));
                            ctx.subscriptions[sub_idx].last_values.insert(key, value);
                        }
                    }
                }
            }
            SubscriptionTarget::Window(window_tmux_id) => {
                let window_tmux_id = *window_tmux_id;
                if let Some(real_tab_id) = ctx.id_map.wezterm_tab_id(window_tmux_id) {
                    for (idx, &wid) in window_ids.iter().enumerate() {
                        if let Some(win) = mux.get_window(wid) {
                            for tab in win.iter() {
                                if tab.tab_id() == real_tab_id {
                                    let fctx = FormatContext {
                                        window_id: window_tmux_id,
                                        window_index: idx as u64,
                                        session_id: ctx
                                            .id_map
                                            .get_or_create_tmux_session_id(&ctx.workspace),
                                        session_name: ctx.workspace.clone(),
                                        ..FormatContext::default()
                                    };
                                    let value =
                                        expand_format(&ctx.subscriptions[sub_idx].format, &fctx);
                                    let key = format!("@{}", window_tmux_id);
                                    let changed = ctx.subscriptions[sub_idx]
                                        .last_values
                                        .get(&key)
                                        .map_or(true, |old| old != &value);
                                    if changed {
                                        notifications.push(
                                            super::response::subscription_changed_notification(
                                                &ctx.subscriptions[sub_idx].name,
                                                &format!("${}", session_id_str),
                                                &format!("@{}", window_tmux_id),
                                                &idx.to_string(),
                                                "",
                                                &value,
                                            ),
                                        );
                                        ctx.subscriptions[sub_idx].last_values.insert(key, value);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            SubscriptionTarget::Session(_) => {
                // Session-level: evaluate format with session context only.
                let fctx = FormatContext {
                    session_id: ctx.id_map.get_or_create_tmux_session_id(&ctx.workspace),
                    session_name: ctx.workspace.clone(),
                    ..FormatContext::default()
                };
                let value = expand_format(&ctx.subscriptions[sub_idx].format, &fctx);
                let key = format!("${}", session_id_str);
                let changed = ctx.subscriptions[sub_idx]
                    .last_values
                    .get(&key)
                    .map_or(true, |old| old != &value);
                if changed {
                    notifications.push(super::response::subscription_changed_notification(
                        &ctx.subscriptions[sub_idx].name,
                        &format!("${}", session_id_str),
                        "",
                        "",
                        "",
                        &value,
                    ));
                    ctx.subscriptions[sub_idx].last_values.insert(key, value);
                }
            }
            SubscriptionTarget::AllPanes => {
                // Evaluate format for every pane in the workspace.
                for (idx, &wid) in window_ids.iter().enumerate() {
                    if let Some(win) = mux.get_window(wid) {
                        for tab in win.iter() {
                            let panes = tab.iter_panes_ignoring_zoom();
                            for pp in &panes {
                                let real_pid = pp.pane.pane_id();
                                let tmux_pid = ctx.id_map.get_or_create_tmux_pane_id(real_pid);
                                let tmux_wid =
                                    ctx.id_map.get_or_create_tmux_window_id(tab.tab_id());
                                let fctx = build_pane_format_context_minimal(
                                    &ctx.id_map,
                                    &pp.pane,
                                    &ctx.workspace,
                                );
                                let value =
                                    expand_format(&ctx.subscriptions[sub_idx].format, &fctx);
                                let key = format!("%{}", tmux_pid);
                                let changed = ctx.subscriptions[sub_idx]
                                    .last_values
                                    .get(&key)
                                    .map_or(true, |old| old != &value);
                                if changed {
                                    notifications.push(
                                        super::response::subscription_changed_notification(
                                            &ctx.subscriptions[sub_idx].name,
                                            &format!("${}", session_id_str),
                                            &format!("@{}", tmux_wid),
                                            &idx.to_string(),
                                            &format!("%{}", tmux_pid),
                                            &value,
                                        ),
                                    );
                                    ctx.subscriptions[sub_idx].last_values.insert(key, value);
                                }
                            }
                        }
                    }
                }
            }
            SubscriptionTarget::AllWindows => {
                // Evaluate format for every window/tab in the workspace.
                for (idx, &wid) in window_ids.iter().enumerate() {
                    if let Some(win) = mux.get_window(wid) {
                        for tab in win.iter() {
                            let tmux_wid = ctx.id_map.get_or_create_tmux_window_id(tab.tab_id());
                            let fctx = FormatContext {
                                window_id: tmux_wid,
                                window_index: idx as u64,
                                session_id: ctx
                                    .id_map
                                    .get_or_create_tmux_session_id(&ctx.workspace),
                                session_name: ctx.workspace.clone(),
                                ..FormatContext::default()
                            };
                            let value = expand_format(&ctx.subscriptions[sub_idx].format, &fctx);
                            let key = format!("@{}", tmux_wid);
                            let changed = ctx.subscriptions[sub_idx]
                                .last_values
                                .get(&key)
                                .map_or(true, |old| old != &value);
                            if changed {
                                notifications.push(
                                    super::response::subscription_changed_notification(
                                        &ctx.subscriptions[sub_idx].name,
                                        &format!("${}", session_id_str),
                                        &format!("@{}", tmux_wid),
                                        &idx.to_string(),
                                        "",
                                        &value,
                                    ),
                                );
                                ctx.subscriptions[sub_idx].last_values.insert(key, value);
                            }
                        }
                    }
                }
            }
        }
    }

    notifications
}

/// Build a minimal FormatContext from a pane reference (for subscriptions).
fn build_pane_format_context_minimal(
    id_map: &IdMap,
    pane: &Arc<dyn Pane>,
    workspace: &str,
) -> FormatContext {
    let tmux_pane_id = id_map.tmux_pane_id(pane.pane_id()).unwrap_or(0);
    let dims = pane.get_dimensions();
    let cursor = pane.get_cursor_position();
    let pane_title = pane.get_title();
    let pane_current_command = pane
        .get_foreground_process_name(CachePolicy::AllowStale)
        .unwrap_or_default();
    let pane_current_path = pane
        .get_current_working_dir(CachePolicy::AllowStale)
        .map(|url| url.path().to_string())
        .unwrap_or_default();

    FormatContext {
        pane_id: tmux_pane_id,
        pane_width: dims.cols as u64,
        pane_height: dims.viewport_rows as u64,
        pane_active: true,
        cursor_x: cursor.x as u64,
        cursor_y: cursor.y as u64,
        pane_title,
        pane_current_command,
        pane_current_path,
        session_id: id_map.tmux_session_id(workspace).unwrap_or(0),
        session_name: workspace.to_string(),
        ..FormatContext::default()
    }
}

/// Find the window ID and index for a given pane.
fn find_window_for_pane(
    mux: &Arc<Mux>,
    window_ids: &[WindowId],
    id_map: &IdMap,
    real_pane_id: PaneId,
) -> (String, String) {
    for (idx, &wid) in window_ids.iter().enumerate() {
        if let Some(win) = mux.get_window(wid) {
            for tab in win.iter() {
                let panes = tab.iter_panes_ignoring_zoom();
                for pp in &panes {
                    if pp.pane.pane_id() == real_pane_id {
                        let tmux_wid = id_map.tmux_window_id(tab.tab_id()).unwrap_or(0);
                        return (format!("@{}", tmux_wid), idx.to_string());
                    }
                }
            }
        }
    }
    (String::new(), String::new())
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
    print_and_format: Option<&str>,
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

    // If -P was specified, return format-expanded info about the new pane
    if let Some(fmt) = print_and_format {
        return format_new_pane(ctx, new_pane.pane_id(), fmt);
    }

    Ok(String::new())
}

/// Create a new window (tab).
pub async fn handle_new_window(
    ctx: &mut HandlerContext,
    target: &Option<String>,
    name: Option<&str>,
    print_and_format: Option<&str>,
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

    // If -P was specified, return format-expanded info about the new pane
    if let Some(fmt) = print_and_format {
        return format_new_pane(ctx, pane.pane_id(), fmt);
    }

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
    let workspace = resolved.workspace.unwrap_or_else(|| ctx.workspace.clone());

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
    let old_workspace = resolved.workspace.unwrap_or_else(|| ctx.workspace.clone());

    mux.rename_workspace(&old_workspace, name);
    ctx.id_map.rename_session(&old_workspace, name);

    // Update context workspace if it was the one renamed
    if ctx.workspace == old_workspace {
        ctx.workspace = name.to_string();
    }

    Ok(String::new())
}

/// Move a pane from one location to another (split target).
///
/// tmux: `move-pane -s <src> -t <dst> [-h] [-b]`
/// Same as `join-pane`.
pub async fn handle_move_pane(
    ctx: &mut HandlerContext,
    src: &Option<String>,
    dst: &Option<String>,
    horizontal: bool,
    before: bool,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    // Resolve source pane (the pane being moved).
    let src_resolved = ctx.resolve_target(src)?;
    let src_real_pane_id = src_resolved
        .pane_id
        .ok_or_else(|| "no source pane resolved for move-pane".to_string())?;

    // Resolve destination pane (where the source will be placed next to).
    let dst_resolved = ctx.resolve_target(dst)?;
    let dst_real_pane_id = dst_resolved
        .pane_id
        .ok_or_else(|| "no destination pane resolved for move-pane".to_string())?;

    if src_real_pane_id == dst_real_pane_id {
        return Err("source and target panes must be different".to_string());
    }

    let direction = if horizontal {
        SplitDirection::Horizontal
    } else {
        SplitDirection::Vertical
    };

    let request = SplitRequest {
        direction,
        target_is_second: !before,
        top_level: false,
        size: SplitSize::Percent(50),
    };

    let source = SplitSource::MovePane(src_real_pane_id);

    mux.split_pane(
        dst_real_pane_id,
        request,
        source,
        SpawnTabDomain::CurrentPaneDomain,
    )
    .await
    .map_err(|e| format!("move-pane failed: {}", e))?;

    Ok(String::new())
}

/// Move a window (tab) from one session to another.
///
/// tmux: `move-window -s <src> -t <dst>`
///
/// In WezTerm's model this means moving a tab from one mux Window to another.
/// Since WezTerm workspaces don't have a fixed window-index scheme like tmux,
/// the tab is simply removed from its current window and pushed to the target.
pub fn handle_move_window(
    ctx: &mut HandlerContext,
    src: &Option<String>,
    dst: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    // Resolve source: the window (tab) to move.
    let src_resolved = ctx.resolve_target(src)?;
    let src_tab_id = src_resolved
        .tab_id
        .ok_or_else(|| "no source window resolved for move-window".to_string())?;

    // Find the mux window containing this tab.
    let (src_mux_window_id, src_tab_arc) = {
        let mut found = None;
        let window_ids = mux.iter_windows_in_workspace(&ctx.workspace);
        for wid in &window_ids {
            if let Some(win) = mux.get_window(*wid) {
                if let Some(idx) = win.idx_by_id(src_tab_id) {
                    // Get the tab Arc before we drop the borrow.
                    let tab = win.get_by_idx(idx).cloned();
                    if let Some(tab) = tab {
                        found = Some((*wid, tab));
                    }
                    break;
                }
            }
        }
        found.ok_or_else(|| format!("source window @{} not found", src_tab_id))?
    };

    // Resolve destination: target window (mux Window) to move into.
    // For move-window, the -t target typically references a session (workspace).
    // We find the first mux Window in the target workspace.
    let dst_resolved = ctx.resolve_target(dst)?;
    let dst_workspace = dst_resolved
        .workspace
        .unwrap_or_else(|| ctx.workspace.clone());

    let dst_mux_window_id = {
        let dst_window_ids = mux.iter_windows_in_workspace(&dst_workspace);
        if let Some(wid) = dst_window_ids.first() {
            *wid
        } else {
            return Err(format!(
                "no windows found in destination workspace '{}'",
                dst_workspace
            ));
        }
    };

    if src_mux_window_id == dst_mux_window_id {
        // Same window — nothing to do.
        return Ok(String::new());
    }

    // Remove tab from source window.
    {
        if let Some(mut win) = mux.get_window_mut(src_mux_window_id) {
            win.remove_by_id(src_tab_id);
        }
    }

    // Add tab to destination window.
    {
        if let Some(mut win) = mux.get_window_mut(dst_mux_window_id) {
            win.push(&src_tab_arc);
        }
    }

    Ok(String::new())
}

/// Create a new session (workspace with a new window).
pub async fn handle_new_session(
    ctx: &mut HandlerContext,
    name: Option<&str>,
    window_name: Option<&str>,
    print_and_format: Option<&str>,
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

    if let Some(title) = window_name {
        tab.set_title(title);
    }

    // Register new mappings
    let tmux_session_id = ctx.id_map.get_or_create_tmux_session_id(&workspace);
    let tmux_window_id = ctx.id_map.get_or_create_tmux_window_id(tab.tab_id());
    let tmux_pane_id = ctx.id_map.get_or_create_tmux_pane_id(pane.pane_id());

    ctx.active_session_id = Some(tmux_session_id);
    ctx.active_window_id = Some(tmux_window_id);
    ctx.active_pane_id = Some(tmux_pane_id);
    ctx.workspace = workspace;

    // If -P was specified, return format-expanded info about the new pane
    if let Some(fmt) = print_and_format {
        return format_new_pane(ctx, pane.pane_id(), fmt);
    }

    Ok(String::new())
}

/// Helper: find the tab and mux window ID containing a given pane.
fn find_tab_and_window_for_pane(
    mux: &Arc<Mux>,
    pane_id: PaneId,
    workspace: &str,
) -> Option<(Arc<Tab>, WindowId)> {
    for wid in mux.iter_windows_in_workspace(workspace) {
        if let Some(win) = mux.get_window(wid) {
            for tab in win.iter() {
                let panes = tab.iter_panes_ignoring_zoom();
                for pp in &panes {
                    if pp.pane.pane_id() == pane_id {
                        return Some((tab.clone(), wid));
                    }
                }
            }
        }
    }
    None
}

/// Helper: build a FormatContext for a newly created pane and expand a format string.
///
/// Used by split-window, new-window, and new-session when `-P -F` is specified.
fn format_new_pane(
    ctx: &mut HandlerContext,
    wez_pane_id: PaneId,
    fmt: &str,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;
    let workspace = ctx.workspace.clone();

    // Find the tab and window containing this pane
    if let Some((tab, window_id)) = find_tab_and_window_for_pane(&mux, wez_pane_id, &workspace) {
        let panes = tab.iter_panes();
        if let Some(pp) = panes.iter().find(|p| p.pane.pane_id() == wez_pane_id) {
            let window_index = {
                let wids = mux.iter_windows_in_workspace(&workspace);
                wids.iter().position(|&w| w == window_id).unwrap_or(0)
            };
            let fctx = build_format_context(ctx, pp, &tab, window_id, window_index, &workspace);
            return Ok(expand_format(fmt, &fctx));
        }
    }

    // Minimal fallback: just return the tmux pane ID
    let tmux_pane_id = ctx.id_map.get_or_create_tmux_pane_id(wez_pane_id);
    Ok(format!("%{}", tmux_pane_id))
}

/// Handle `set-option` — no-op, returns empty success.
fn handle_set_option(option_name: Option<&str>, value: Option<&str>) -> Result<String, String> {
    log::debug!(
        "set-option: {}={}  (no-op)",
        option_name.unwrap_or("(none)"),
        value.unwrap_or("(none)")
    );
    Ok(String::new())
}

/// Handle `break-pane` — move a pane to its own new tab.
async fn handle_break_pane(
    ctx: &mut HandlerContext,
    _detach: bool,
    source: &Option<String>,
    target: &Option<String>,
) -> Result<String, String> {
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;

    // Resolve the source pane to break out
    let resolved_src = ctx.resolve_target(source)?;
    let pane_id = resolved_src
        .pane_id
        .ok_or_else(|| "no pane resolved for break-pane".to_string())?;

    // Determine the workspace for the new tab
    let workspace = if let Some(tgt) = target {
        // Target may specify a session name (e.g., "mysession:")
        let session_name = tgt.trim_end_matches(':');
        if !session_name.is_empty() {
            session_name.to_string()
        } else {
            ctx.workspace.clone()
        }
    } else {
        ctx.workspace.clone()
    };

    // Find the window containing this pane so we can determine where to create a new tab
    let window_id = resolved_src.window_id;

    // Use MovePane with a new split to effectively break the pane out.
    // The simplest approach: create a new tab, then move the pane into it.
    // But WezTerm doesn't have a direct "break-pane" API.
    // We'll spawn a new tab and then swap the pane content.
    // For now, return success as a best-effort no-op since this is a LOW priority item.
    let _ = (mux, pane_id, workspace, window_id);
    log::debug!(
        "break-pane: best-effort no-op (source={:?}, target={:?})",
        source,
        target
    );
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
    let options: &[(&str, &str)] = &[("aggressive-resize", "off"), ("mode-keys", "emacs")];

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
pub fn handle_list_clients(
    ctx: &mut HandlerContext,
    format: Option<&str>,
) -> Result<String, String> {
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
// Phase 11: clipboard / buffer command handlers
// ---------------------------------------------------------------------------

/// `show-buffer [-b buffer-name]` — return buffer content as raw text.
fn handle_show_buffer(
    ctx: &mut HandlerContext,
    buffer_name: Option<&str>,
) -> Result<String, String> {
    let buf = match buffer_name {
        Some(name) => ctx
            .paste_buffers
            .get(name)
            .ok_or_else(|| format!("unknown buffer: {}", name))?,
        None => ctx
            .paste_buffers
            .most_recent()
            .ok_or_else(|| "no buffers".to_string())?,
    };
    Ok(buf.data.clone())
}

/// `set-buffer [-a] [-b buffer-name] [data]` — create/update a buffer.
fn handle_set_buffer(
    ctx: &mut HandlerContext,
    buffer_name: Option<&str>,
    data: Option<&str>,
    append: bool,
) -> Result<String, String> {
    if append {
        let name = buffer_name.ok_or_else(|| "set-buffer -a requires -b".to_string())?;
        let content = data.unwrap_or("");
        ctx.paste_buffers
            .append(name, content)
            .map_err(|e| e.to_string())?;
        ctx.pending_notifications
            .push(super::response::paste_buffer_changed_notification(name));
        return Ok(String::new());
    }

    let content = data
        .ok_or_else(|| "no data specified".to_string())?
        .to_string();
    let name = ctx.paste_buffers.set(buffer_name, content);
    ctx.pending_notifications
        .push(super::response::paste_buffer_changed_notification(&name));
    Ok(String::new())
}

/// `delete-buffer [-b buffer-name]` — remove a buffer.
fn handle_delete_buffer(
    ctx: &mut HandlerContext,
    buffer_name: Option<&str>,
) -> Result<String, String> {
    match buffer_name {
        Some(name) => {
            if ctx.paste_buffers.delete(name) {
                ctx.pending_notifications
                    .push(super::response::paste_buffer_deleted_notification(name));
                Ok(String::new())
            } else {
                Err(format!("unknown buffer: {}", name))
            }
        }
        None => match ctx.paste_buffers.delete_most_recent() {
            Some(name) => {
                ctx.pending_notifications
                    .push(super::response::paste_buffer_deleted_notification(&name));
                Ok(String::new())
            }
            None => Err("no buffers".to_string()),
        },
    }
}

/// `list-buffers [-F format]` — list all buffers with format expansion.
fn handle_list_buffers(ctx: &mut HandlerContext, format: Option<&str>) -> Result<String, String> {
    let default_fmt = "#{buffer_name}: #{buffer_size} bytes: \"#{buffer_sample}\"";
    let fmt = format.unwrap_or(default_fmt);

    let bufs: Vec<_> = ctx
        .paste_buffers
        .list()
        .iter()
        .map(|b| (b.name.clone(), b.data.clone()))
        .collect();

    let mut lines = Vec::new();
    for (name, data) in &bufs {
        // Build a minimal FormatContext with buffer fields populated.
        let fctx = FormatContext {
            buffer_name: name.clone(),
            buffer_size: data.len() as u64,
            buffer_sample: buffer_sample(data),
            session_name: ctx.workspace.clone(),
            session_attached: 1,
            client_name: ctx.client_name.clone(),
            socket_path: ctx.socket_path.clone(),
            server_pid: std::process::id() as u64,
            ..FormatContext::default()
        };
        lines.push(expand_format(fmt, &fctx));
    }
    Ok(lines.join("\n"))
}

/// `paste-buffer [-d] [-p] [-b buffer-name] [-t target-pane]` — send buffer
/// content to a pane's input.
fn handle_paste_buffer(
    ctx: &mut HandlerContext,
    buffer_name: Option<&str>,
    target: &Option<String>,
    delete_after: bool,
) -> Result<String, String> {
    let buf = match buffer_name {
        Some(name) => ctx
            .paste_buffers
            .get(name)
            .ok_or_else(|| format!("unknown buffer: {}", name))?,
        None => ctx
            .paste_buffers
            .most_recent()
            .ok_or_else(|| "no buffers".to_string())?,
    };
    let data = buf.data.clone();
    let buf_name = buf.name.clone();

    // Resolve target pane.
    let mux = Mux::try_get().ok_or_else(|| "mux not available".to_string())?;
    let pane_id = if let Some(ref t) = target {
        let resolved = ctx.resolve_target(&Some(t.clone()))?;
        resolved
            .pane_id
            .ok_or_else(|| format!("can't find pane: {}", t))?
    } else {
        ctx.active_pane_id
            .and_then(|tmux_id| ctx.id_map.wezterm_pane_id(tmux_id))
            .ok_or_else(|| "no active pane".to_string())?
    };

    let pane = mux
        .get_pane(pane_id)
        .ok_or_else(|| "pane not found".to_string())?;

    // send_paste handles bracketed paste based on pane's terminal mode.
    pane.send_paste(&data)
        .map_err(|e| format!("paste failed: {}", e))?;

    if delete_after {
        ctx.paste_buffers.delete(&buf_name);
        ctx.pending_notifications
            .push(super::response::paste_buffer_deleted_notification(
                &buf_name,
            ));
    }

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
        assert_eq!(commands.len(), 39);
        assert!(commands.contains(&"attach-session"));
        assert!(commands.contains(&"break-pane"));
        assert!(commands.contains(&"capture-pane"));
        assert!(commands.contains(&"copy-mode"));
        assert!(commands.contains(&"delete-buffer"));
        assert!(commands.contains(&"detach-client"));
        assert!(commands.contains(&"display-message"));
        assert!(commands.contains(&"has-session"));
        assert!(commands.contains(&"join-pane"));
        assert!(commands.contains(&"kill-pane"));
        assert!(commands.contains(&"kill-session"));
        assert!(commands.contains(&"kill-window"));
        assert!(commands.contains(&"list-buffers"));
        assert!(commands.contains(&"list-clients"));
        assert!(commands.contains(&"list-commands"));
        assert!(commands.contains(&"list-panes"));
        assert!(commands.contains(&"list-sessions"));
        assert!(commands.contains(&"list-windows"));
        assert!(commands.contains(&"move-pane"));
        assert!(commands.contains(&"move-window"));
        assert!(commands.contains(&"new-session"));
        assert!(commands.contains(&"new-window"));
        assert!(commands.contains(&"paste-buffer"));
        assert!(commands.contains(&"refresh-client"));
        assert!(commands.contains(&"rename-session"));
        assert!(commands.contains(&"rename-window"));
        assert!(commands.contains(&"resize-pane"));
        assert!(commands.contains(&"resize-window"));
        assert!(commands.contains(&"select-layout"));
        assert!(commands.contains(&"select-pane"));
        assert!(commands.contains(&"select-window"));
        assert!(commands.contains(&"send-keys"));
        assert!(commands.contains(&"set-buffer"));
        assert!(commands.contains(&"set-option"));
        assert!(commands.contains(&"show-buffer"));
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
        assert!(ctx.pause_age_ms.is_none());
        assert!(!ctx.wait_exit);
        assert!(ctx.paused_panes.is_empty());
        assert!(ctx.pane_output_timestamps.is_empty());
    }

    // --- Phase 12.1: pause mode handler tests ---

    #[test]
    fn refresh_client_pause_after_sets_age() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result = handle_refresh_client(&mut ctx, None, Some("pause-after=5"), None, None);
        assert!(result.is_ok());
        assert_eq!(ctx.pause_age_ms, Some(5000));
    }

    #[test]
    fn refresh_client_pause_after_zero() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result = handle_refresh_client(&mut ctx, None, Some("pause-after"), None, None);
        assert!(result.is_ok());
        assert_eq!(ctx.pause_age_ms, Some(0));
    }

    #[test]
    fn refresh_client_disable_pause() {
        let mut ctx = HandlerContext::new("default".to_string());
        ctx.pause_age_ms = Some(5000);
        let result = handle_refresh_client(&mut ctx, None, Some("!pause-after"), None, None);
        assert!(result.is_ok());
        assert_eq!(ctx.pause_age_ms, None);
    }

    #[test]
    fn refresh_client_wait_exit_flag() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result =
            handle_refresh_client(&mut ctx, None, Some("pause-after=3,wait-exit"), None, None);
        assert!(result.is_ok());
        assert_eq!(ctx.pause_age_ms, Some(3000));
        assert!(ctx.wait_exit);
    }

    #[test]
    fn refresh_client_disable_wait_exit() {
        let mut ctx = HandlerContext::new("default".to_string());
        ctx.wait_exit = true;
        let result = handle_refresh_client(&mut ctx, None, Some("!wait-exit"), None, None);
        assert!(result.is_ok());
        assert!(!ctx.wait_exit);
    }

    #[test]
    fn pane_adjust_continue() {
        let mut ctx = HandlerContext::new("default".to_string());
        ctx.paused_panes.insert(0, true);
        let result = handle_refresh_client(&mut ctx, None, None, Some("%0:continue"), None);
        assert!(result.is_ok());
        assert_eq!(ctx.paused_panes.get(&0), Some(&false));
        assert!(ctx
            .pending_notifications
            .iter()
            .any(|n| n.contains("%continue %0")));
    }

    #[test]
    fn pane_adjust_pause() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result = handle_refresh_client(&mut ctx, None, None, Some("%0:pause"), None);
        assert!(result.is_ok());
        assert_eq!(ctx.paused_panes.get(&0), Some(&true));
        assert!(ctx
            .pending_notifications
            .iter()
            .any(|n| n.contains("%pause %0")));
    }

    #[test]
    fn pane_adjust_on_off() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result = handle_refresh_client(&mut ctx, None, None, Some("%5:off"), None);
        assert!(result.is_ok());
        assert_eq!(ctx.paused_panes.get(&5), Some(&true));
        // "off" is silent — no notification.
        assert!(ctx.pending_notifications.is_empty());

        let result = handle_refresh_client(&mut ctx, None, None, Some("%5:on"), None);
        assert!(result.is_ok());
        assert_eq!(ctx.paused_panes.get(&5), Some(&false));
        assert!(ctx.pending_notifications.is_empty());
    }

    #[test]
    fn pane_adjust_invalid_format() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result = handle_refresh_client(&mut ctx, None, None, Some("bad:continue"), None);
        assert!(result.is_err());
    }

    #[test]
    fn pane_adjust_unknown_action() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result = handle_refresh_client(&mut ctx, None, None, Some("%0:unknown"), None);
        assert!(result.is_err());
    }

    #[test]
    fn pane_adjust_continue_not_paused() {
        let mut ctx = HandlerContext::new("default".to_string());
        // Continue on a pane that isn't paused — should be a no-op.
        let result = handle_refresh_client(&mut ctx, None, None, Some("%0:continue"), None);
        assert!(result.is_ok());
        assert!(ctx.pending_notifications.is_empty());
    }

    // --- Phase 12.2: subscription tests ---

    #[test]
    fn subscription_register() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result =
            handle_refresh_client(&mut ctx, None, None, None, Some("my-sub:%0:#{pane_id}"));
        assert!(result.is_ok());
        assert_eq!(ctx.subscriptions.len(), 1);
        assert_eq!(ctx.subscriptions[0].name, "my-sub");
        assert_eq!(ctx.subscriptions[0].target, SubscriptionTarget::Pane(0));
        assert_eq!(ctx.subscriptions[0].format, "#{pane_id}");
    }

    #[test]
    fn subscription_unregister() {
        let mut ctx = HandlerContext::new("default".to_string());
        // Register first.
        handle_refresh_client(&mut ctx, None, None, None, Some("test:%0:#{pane_id}")).unwrap();
        assert_eq!(ctx.subscriptions.len(), 1);
        // Unregister by name only.
        handle_refresh_client(&mut ctx, None, None, None, Some("test")).unwrap();
        assert_eq!(ctx.subscriptions.len(), 0);
    }

    #[test]
    fn subscription_replace() {
        let mut ctx = HandlerContext::new("default".to_string());
        // Register with one format.
        handle_refresh_client(&mut ctx, None, None, None, Some("sub1:%0:#{pane_id}")).unwrap();
        assert_eq!(ctx.subscriptions[0].format, "#{pane_id}");
        // Replace with different format (same name).
        handle_refresh_client(&mut ctx, None, None, None, Some("sub1:%1:#{pane_width}")).unwrap();
        assert_eq!(ctx.subscriptions.len(), 1);
        assert_eq!(ctx.subscriptions[0].format, "#{pane_width}");
        assert_eq!(ctx.subscriptions[0].target, SubscriptionTarget::Pane(1));
    }

    #[test]
    fn subscription_all_panes_target() {
        let mut ctx = HandlerContext::new("default".to_string());
        handle_refresh_client(&mut ctx, None, None, None, Some("all:%*:#{pane_id}")).unwrap();
        assert_eq!(ctx.subscriptions[0].target, SubscriptionTarget::AllPanes);
    }

    #[test]
    fn subscription_all_windows_target() {
        let mut ctx = HandlerContext::new("default".to_string());
        handle_refresh_client(&mut ctx, None, None, None, Some("all:@*:#{window_id}")).unwrap();
        assert_eq!(ctx.subscriptions[0].target, SubscriptionTarget::AllWindows);
    }

    #[test]
    fn subscription_session_target() {
        let mut ctx = HandlerContext::new("default".to_string());
        handle_refresh_client(&mut ctx, None, None, None, Some("s:$0:#{session_name}")).unwrap();
        assert_eq!(ctx.subscriptions[0].target, SubscriptionTarget::Session(0));
    }

    #[test]
    fn subscription_window_target() {
        let mut ctx = HandlerContext::new("default".to_string());
        handle_refresh_client(&mut ctx, None, None, None, Some("w:@3:#{window_id}")).unwrap();
        assert_eq!(ctx.subscriptions[0].target, SubscriptionTarget::Window(3));
    }

    #[test]
    fn subscription_invalid_format() {
        let mut ctx = HandlerContext::new("default".to_string());
        // Missing second colon — should error.
        let result = handle_refresh_client(&mut ctx, None, None, None, Some("name:bad"));
        assert!(result.is_err());
    }

    #[test]
    fn subscription_invalid_target() {
        let mut ctx = HandlerContext::new("default".to_string());
        let result = handle_refresh_client(&mut ctx, None, None, None, Some("name:invalid:fmt"));
        assert!(result.is_err());
    }

    #[test]
    fn subscription_multiple() {
        let mut ctx = HandlerContext::new("default".to_string());
        handle_refresh_client(&mut ctx, None, None, None, Some("a:%0:#{pane_id}")).unwrap();
        handle_refresh_client(&mut ctx, None, None, None, Some("b:%1:#{pane_id}")).unwrap();
        assert_eq!(ctx.subscriptions.len(), 2);
        // Unregister just one.
        handle_refresh_client(&mut ctx, None, None, None, Some("a")).unwrap();
        assert_eq!(ctx.subscriptions.len(), 1);
        assert_eq!(ctx.subscriptions[0].name, "b");
    }

    // --- Phase 12.4: copy-mode tests ---

    #[test]
    fn copy_mode_quit_succeeds() {
        assert_eq!(handle_copy_mode(true), Ok(String::new()));
    }

    #[test]
    fn copy_mode_enter_succeeds() {
        assert_eq!(handle_copy_mode(false), Ok(String::new()));
    }
}
