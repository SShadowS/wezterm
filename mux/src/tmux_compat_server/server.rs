//! CC (control mode) protocol server for the tmux compatibility layer.
//!
//! Accepts connections on a Unix domain socket, reads tmux commands as text
//! lines, dispatches them through Phase 2's `dispatch_command`, and writes
//! `%begin`/`%end` response blocks.  Mux notifications are forwarded as
//! CC-style `%`-prefixed notification lines.

use std::sync::Arc;
use std::time::Instant;

use crate::tab::{PositionedPane, Tab};
use crate::{Mux, MuxNotification};

use super::command_parser::parse_command;
use super::handlers::{dispatch_command, HandlerContext};
use super::layout::{generate_layout_string, LayoutNode};
use super::response::{
    exit_notification, extended_output_notification, layout_change_notification,
    output_notification, paste_buffer_changed_notification, pause_notification,
    session_changed_notification, session_renamed_notification,
    session_window_changed_notification, sessions_changed_notification, window_add_notification,
    window_close_notification, window_pane_changed_notification, window_renamed_notification,
    ResponseWriter,
};

// ---------------------------------------------------------------------------
// TmuxCompatSession
// ---------------------------------------------------------------------------

/// Per-client session state for a CC protocol connection.
pub struct TmuxCompatSession {
    pub ctx: HandlerContext,
    pub writer: ResponseWriter,
    pub line_buffer: String,
}

impl TmuxCompatSession {
    pub fn new(workspace: String) -> Self {
        Self {
            ctx: HandlerContext::with_persistent_ids(workspace),
            writer: ResponseWriter::new(),
            line_buffer: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Initial handshake
// ---------------------------------------------------------------------------

/// Build the initial handshake payload sent when a client first connects.
///
/// Per real tmux CC behavior this sends:
/// 1. An empty `%begin`/`%end` greeting block
/// 2. `%session-changed` for the current workspace
/// 3. `%window-add` for each existing tab in the workspace
pub fn build_initial_handshake(session: &mut TmuxCompatSession) -> String {
    let mut out = String::new();

    // 1. Empty greeting block
    out.push_str(&session.writer.empty_success());

    // 2. Session-changed notification
    let tmux_sid = session
        .ctx
        .id_map
        .get_or_create_tmux_session_id(&session.ctx.workspace.clone());
    let workspace = session.ctx.workspace.clone();
    out.push_str(&session_changed_notification(tmux_sid, &workspace));

    // 3. Window-add for each tab in the workspace
    if let Some(mux) = Mux::try_get() {
        let window_ids = mux.iter_windows_in_workspace(&workspace);
        for wid in window_ids {
            let tabs: Vec<Arc<Tab>> = match mux.get_window(wid) {
                Some(win) => win.iter().map(Arc::clone).collect(),
                None => continue,
            };
            for tab in tabs {
                let tmux_wid = session
                    .ctx
                    .id_map
                    .get_or_create_tmux_window_id(tab.tab_id());
                out.push_str(&window_add_notification(tmux_wid));

                // Also register panes so they're available immediately
                for pp in tab.iter_panes() {
                    session
                        .ctx
                        .id_map
                        .get_or_create_tmux_pane_id(pp.pane.pane_id());
                }
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Layout builder
// ---------------------------------------------------------------------------

/// Convert a tab's positioned panes into a `LayoutNode` tree for
/// `%layout-change` notifications.
///
/// Simplified approach: single pane → `LayoutNode::Pane`.  Multiple panes
/// sharing the same `top` → `HorizontalSplit`.  Multiple panes sharing the
/// same `left` → `VerticalSplit`.  Otherwise fall back to a flat horizontal
/// arrangement.
pub fn build_layout_for_tab(ctx: &mut HandlerContext, tab: &Arc<Tab>) -> String {
    let panes = tab.iter_panes();
    let tab_size = tab.get_size();

    let root = if panes.len() <= 1 {
        // Single pane (or empty tab)
        if let Some(pp) = panes.first() {
            pane_to_layout_node(ctx, pp)
        } else {
            LayoutNode::Pane {
                pane_id: 0,
                width: tab_size.cols as u64,
                height: tab_size.rows as u64,
                left: 0,
                top: 0,
            }
        }
    } else {
        build_split_node(ctx, &panes, tab_size.cols as u64, tab_size.rows as u64)
    };

    generate_layout_string(&root)
}

fn pane_to_layout_node(ctx: &mut HandlerContext, pp: &PositionedPane) -> LayoutNode {
    let tmux_pid = ctx.id_map.get_or_create_tmux_pane_id(pp.pane.pane_id());
    LayoutNode::Pane {
        pane_id: tmux_pid,
        width: pp.width as u64,
        height: pp.height as u64,
        left: pp.left as u64,
        top: pp.top as u64,
    }
}

fn build_split_node(
    ctx: &mut HandlerContext,
    panes: &[PositionedPane],
    width: u64,
    height: u64,
) -> LayoutNode {
    let children: Vec<LayoutNode> = panes
        .iter()
        .map(|pp| pane_to_layout_node(ctx, pp))
        .collect();

    // Detect split orientation from positions
    let all_same_top = panes.windows(2).all(|w| w[0].top == w[1].top);
    let all_same_left = panes.windows(2).all(|w| w[0].left == w[1].left);

    if all_same_top {
        LayoutNode::HorizontalSplit {
            width,
            height,
            left: 0,
            top: 0,
            children,
        }
    } else if all_same_left {
        LayoutNode::VerticalSplit {
            width,
            height,
            left: 0,
            top: 0,
            children,
        }
    } else {
        // Complex nested splits — fall back to flat horizontal arrangement
        LayoutNode::HorizontalSplit {
            width,
            height,
            left: 0,
            top: 0,
            children,
        }
    }
}

// ---------------------------------------------------------------------------
// Notification translation
// ---------------------------------------------------------------------------

/// Translate a `MuxNotification` into a CC notification string.
///
/// Returns `None` for notifications that have no CC equivalent or that are
/// intentionally ignored (e.g. `PaneOutput`, `Empty`).
pub fn translate_notification(
    session: &mut TmuxCompatSession,
    notif: MuxNotification,
) -> Option<String> {
    match notif {
        MuxNotification::TabResized(tab_id) => {
            let mux = Mux::try_get()?;
            let tab = mux.get_tab(tab_id)?;
            let tmux_wid = session.ctx.id_map.get_or_create_tmux_window_id(tab_id);
            let layout = build_layout_for_tab(&mut session.ctx, &tab);
            Some(layout_change_notification(tmux_wid, &layout))
        }

        MuxNotification::TabAddedToWindow { tab_id, window_id } => {
            let tmux_wid = session.ctx.id_map.get_or_create_tmux_window_id(tab_id);
            // Track tab→mux_window relationship for %window-close
            if let Some(mux) = Mux::try_get() {
                if let Some(win) = mux.get_window(window_id) {
                    let ws = win.get_workspace().to_string();
                    session
                        .ctx
                        .id_map
                        .track_tab_in_window(window_id, tab_id, &ws);
                }
                // Also register any panes in the new tab
                if let Some(tab) = mux.get_tab(tab_id) {
                    for pp in tab.iter_panes() {
                        session
                            .ctx
                            .id_map
                            .get_or_create_tmux_pane_id(pp.pane.pane_id());
                    }
                }
            }
            Some(window_add_notification(tmux_wid))
        }

        MuxNotification::WindowCreated(window_id) => {
            // Track the workspace for this mux window; emit %sessions-changed
            // if this is a new workspace we haven't seen before.
            if let Some(mux) = Mux::try_get() {
                if let Some(win) = mux.get_window(window_id) {
                    let ws = win.get_workspace().to_string();
                    let is_new = session.ctx.id_map.tmux_session_id(&ws).is_none();
                    session
                        .ctx
                        .id_map
                        .track_mux_window_workspace(window_id, &ws);
                    if is_new {
                        // New workspace appeared — register it and notify
                        session.ctx.id_map.get_or_create_tmux_session_id(&ws);
                        return Some(sessions_changed_notification());
                    }
                }
            }
            None
        }

        MuxNotification::WindowRemoved(window_id) => {
            // Look up which tabs were in this mux window and emit
            // %window-close for each.
            let workspace = session
                .ctx
                .id_map
                .mux_window_workspace(window_id)
                .map(|s| s.to_string());
            let tab_ids = session.ctx.id_map.remove_mux_window(window_id);
            let mut out = String::new();
            for tab_id in &tab_ids {
                if let Some(tmux_wid) = session.ctx.id_map.tmux_window_id(*tab_id) {
                    out.push_str(&window_close_notification(tmux_wid));
                    session.ctx.id_map.remove_window(*tab_id);
                }
            }
            // If the workspace has no more windows, the session is gone
            if let Some(ws) = &workspace {
                if let Some(mux) = Mux::try_get() {
                    let remaining = mux.iter_windows_in_workspace(ws);
                    if remaining.is_empty() {
                        out.push_str(&sessions_changed_notification());
                    }
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }

        MuxNotification::PaneFocused(pane_id) => {
            let tmux_pid = session.ctx.id_map.tmux_pane_id(pane_id)?;
            // Find the tab containing this pane
            let mux = Mux::try_get()?;
            let tab_id = {
                let mut found = None;
                // Walk windows to find the tab
                for wid in mux.iter_windows_in_workspace(&session.ctx.workspace) {
                    if let Some(win) = mux.get_window(wid) {
                        for tab in win.iter() {
                            for pp in tab.iter_panes() {
                                if pp.pane.pane_id() == pane_id {
                                    found = Some(tab.tab_id());
                                    break;
                                }
                            }
                            if found.is_some() {
                                break;
                            }
                        }
                    }
                    if found.is_some() {
                        break;
                    }
                }
                found?
            };
            let tmux_wid = session.ctx.id_map.get_or_create_tmux_window_id(tab_id);
            Some(window_pane_changed_notification(tmux_wid, tmux_pid))
        }

        MuxNotification::TabTitleChanged { tab_id, title } => {
            let tmux_wid = session.ctx.id_map.tmux_window_id(tab_id)?;
            Some(window_renamed_notification(tmux_wid, &title))
        }

        MuxNotification::PaneRemoved(pane_id) => {
            // Clean up id_map, but don't emit a separate notification.
            // The layout-change from TabResized covers the visual change.
            session.ctx.id_map.remove_pane(pane_id);
            None
        }

        MuxNotification::WorkspaceRenamed {
            old_workspace,
            new_workspace,
        } => {
            // Re-key the session mapping and emit %session-renamed
            if let Some(tmux_sid) = session
                .ctx
                .id_map
                .rename_session(&old_workspace, &new_workspace)
            {
                Some(session_renamed_notification(tmux_sid, &new_workspace))
            } else {
                None
            }
        }

        MuxNotification::WindowInvalidated(window_id) => {
            // Detect active-tab changes → %session-window-changed.
            // Compare the current active tab against the last known one.
            if session.ctx.suppress_window_changed > 0 {
                session.ctx.suppress_window_changed -= 1;
                return None;
            }
            let mux = Mux::try_get()?;
            let win = mux.get_window(window_id)?;
            let active_tab = win.get_active()?;
            let active_tab_id = active_tab.tab_id();
            let workspace = win.get_workspace().to_string();
            drop(win);

            let prev = session.ctx.last_active_tab.get(&window_id).copied();
            session.ctx.last_active_tab.insert(window_id, active_tab_id);

            if prev.is_some() && prev != Some(active_tab_id) {
                // Active tab actually changed — emit notification
                let tmux_sid = session.ctx.id_map.get_or_create_tmux_session_id(&workspace);
                let tmux_wid = session
                    .ctx
                    .id_map
                    .get_or_create_tmux_window_id(active_tab_id);
                Some(session_window_changed_notification(tmux_sid, tmux_wid))
            } else {
                None
            }
        }

        MuxNotification::AssignClipboard { clipboard, .. } => {
            // Clipboard content changed → store in paste buffer and notify
            if let Some(content) = clipboard {
                session.ctx.paste_buffers.set(None, content);
            }
            Some(paste_buffer_changed_notification("buffer0"))
        }

        // Notifications with no CC equivalent — silently ignore.
        MuxNotification::PaneOutput(_)
        | MuxNotification::PaneAdded(_)
        | MuxNotification::WindowWorkspaceChanged(_)
        | MuxNotification::ActiveWorkspaceChanged(_)
        | MuxNotification::Alert { .. }
        | MuxNotification::Empty
        | MuxNotification::SaveToDownloads { .. }
        | MuxNotification::WindowTitleChanged { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// Line extraction
// ---------------------------------------------------------------------------

/// Extract complete lines from the line buffer.
///
/// Returns a `Vec` of complete lines (without trailing `\n`).  Any partial
/// line (content after the last `\n`) is left in the buffer.
pub fn extract_lines(buf: &mut String) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(pos) = buf.find('\n') {
        let line: String = buf[..pos].to_string();
        // Remove the line + the newline character
        buf.drain(..=pos);
        lines.push(line);
    }
    lines
}

// ---------------------------------------------------------------------------
// Main connection loop
// ---------------------------------------------------------------------------

/// Process a single CC protocol connection synchronously.
///
/// Runs on a dedicated thread per connection.  Command dispatch hops to the
/// main GUI thread via `spawn_into_main_thread` (required for `Mux::get()`).
/// Generic over any `Read + Write` stream so the listener can pass either a
/// Unix domain socket (unix) or a TCP stream (Windows).
/// Monotonic counter for generating unique client names.
static CLIENT_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Drain pending output from output tap receivers and write `%output` or
/// `%extended-output` notifications to the stream.
///
/// Returns `Err` if a write fails (connection broken).
fn drain_output_taps(
    session: &mut TmuxCompatSession,
    output_rx: &std::sync::mpsc::Receiver<(crate::pane::PaneId, Vec<u8>, Instant)>,
    stream: &mut impl std::io::Write,
) -> anyhow::Result<()> {
    while let Ok((wezterm_pane_id, data, when)) = output_rx.try_recv() {
        // Map WezTerm pane ID to tmux pane ID.
        let tmux_pid = match session.ctx.id_map.tmux_pane_id(wezterm_pane_id) {
            Some(id) => id,
            None => continue, // Not a tracked pane.
        };

        // Skip if this pane is paused.
        if session.ctx.paused_panes.get(&tmux_pid) == Some(&true) {
            continue;
        }

        // Format as %output or %extended-output.
        let notif = if session.ctx.pause_age_ms.is_some() {
            // Pause mode enabled — compute age and use %extended-output.
            let age_ms = {
                let first_ts = session
                    .ctx
                    .pane_output_timestamps
                    .entry(tmux_pid)
                    .or_insert(when);
                when.saturating_duration_since(*first_ts).as_millis() as u64
            };

            // Check if age exceeds pause threshold → auto-pause.
            if let Some(limit_ms) = session.ctx.pause_age_ms {
                if limit_ms > 0 && age_ms > limit_ms {
                    session.ctx.paused_panes.insert(tmux_pid, true);
                    let pause = pause_notification(tmux_pid);
                    std::io::Write::write_all(stream, pause.as_bytes())?;
                    continue;
                }
            }

            extended_output_notification(tmux_pid, age_ms, &data)
        } else {
            output_notification(tmux_pid, &data)
        };

        std::io::Write::write_all(stream, notif.as_bytes())?;
    }
    std::io::Write::flush(stream)?;
    Ok(())
}

/// Register output taps for all panes in the workspace and start a forwarder
/// thread that reads raw output from taps and sends it to a unified channel.
///
/// Returns a receiver for `(wezterm_pane_id, data, timestamp)`.
fn start_output_forwarder(
    workspace: &str,
) -> std::sync::mpsc::Receiver<(crate::pane::PaneId, Vec<u8>, Instant)> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1024);

    // Register taps for all existing panes.
    if let Some(mux) = Mux::try_get() {
        let window_ids = mux.iter_windows_in_workspace(workspace);
        for wid in window_ids {
            if let Some(win) = mux.get_window(wid) {
                for tab in win.iter() {
                    for pp in tab.iter_panes() {
                        let pane_id = pp.pane.pane_id();
                        let tap_rx = crate::register_output_tap(pane_id);
                        let tx2 = tx.clone();
                        std::thread::Builder::new()
                            .name(format!("cc-output-tap-{}", pane_id))
                            .spawn(move || {
                                for (data, when) in tap_rx {
                                    if tx2.send((pane_id, data, when)).is_err() {
                                        break; // Connection closed.
                                    }
                                }
                            })
                            .ok();
                    }
                }
            }
        }
    }

    rx
}

fn process_cc_connection_sync(
    mut stream: impl std::io::Read + std::io::Write,
    listen_addr: &str,
) -> anyhow::Result<()> {
    // Build session and handshake directly on this thread.
    // Mux::try_get() uses a global Arc and works from any thread.
    let workspace = Mux::try_get()
        .map(|mux| mux.active_workspace().to_string())
        .unwrap_or_else(|| "default".to_string());
    let mut session = TmuxCompatSession::new(workspace.clone());
    let client_num = CLIENT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    session.ctx.client_name = format!("/dev/pts/{}", client_num);
    session.ctx.socket_path = listen_addr.to_string();
    let handshake = build_initial_handshake(&mut session);

    std::io::Write::write_all(&mut stream, handshake.as_bytes())?;
    std::io::Write::flush(&mut stream)?;
    log::info!("tmux CC: handshake sent ({} bytes)", handshake.len());

    // Start output forwarder for all panes in the workspace.
    let output_rx = start_output_forwarder(&workspace);

    // Manual line-buffered read loop.  We avoid BufReader because we need
    // to alternate reads and writes on the same stream, and writes through
    // BufReader::get_mut() don't work reliably on Windows.
    let mut read_buf = vec![0u8; 4096];
    let mut accum = String::new();
    let mut last_subscription_check = std::time::Instant::now();
    loop {
        // Drain any pending output before blocking on read.
        drain_output_taps(&mut session, &output_rx, &mut stream)?;

        // Extract any complete line already in the accumulator.
        let line = loop {
            if let Some(pos) = accum.find('\n') {
                let line = accum[..pos].to_string();
                accum.drain(..=pos);
                break line;
            }
            // Need more data — read with a short timeout so we can also
            // drain output taps periodically.
            let n = std::io::Read::read(&mut stream, &mut read_buf)?;
            if n == 0 {
                log::trace!("CC client disconnected (EOF)");
                return Ok(());
            }
            accum.push_str(&String::from_utf8_lossy(&read_buf[..n]));

            // Also drain output between reads.
            drain_output_taps(&mut session, &output_rx, &mut stream)?;
        };

        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }

        // Dispatch the command on the main thread via spawn_local.
        let cmd_line = trimmed;
        let mut ctx = std::mem::replace(&mut session.ctx, HandlerContext::new(String::new()));
        let (resp_tx, resp_rx) = std::sync::mpsc::sync_channel(1);
        promise::spawn::spawn_into_main_thread(async move {
            promise::spawn::spawn(async move {
                let resp = match parse_command(&cmd_line) {
                    Ok(cmd) => match dispatch_command(&mut ctx, cmd).await {
                        Ok(body) => Ok(body),
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(format!("{}", e)),
                };
                let _ = resp_tx.send((resp, ctx));
            })
            .detach();
        })
        .detach();

        // While waiting for the command response, keep draining output.
        let (response, ctx_back) = loop {
            match resp_rx.recv_timeout(std::time::Duration::from_millis(10)) {
                Ok(result) => break result,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    drain_output_taps(&mut session, &output_rx, &mut stream)?;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(anyhow::anyhow!("failed to receive command response"));
                }
            }
        };
        session.ctx = ctx_back;

        // Persist ID mappings after each command (best-effort).
        if response.is_ok() {
            session.ctx.save_id_map();
        }

        let formatted = match response {
            Ok(ref body) if body.is_empty() => session.writer.empty_success(),
            Ok(body) => session.writer.success(&body),
            Err(e) => session.writer.error(&e),
        };
        // Write response directly — no BufReader::get_mut().
        std::io::Write::write_all(&mut stream, formatted.as_bytes())?;
        std::io::Write::flush(&mut stream)?;

        // Drain any pending notifications queued by the handler
        // (e.g. %session-changed after attach-session).
        for notif in session.ctx.pending_notifications.drain(..) {
            std::io::Write::write_all(&mut stream, notif.as_bytes())?;
        }
        std::io::Write::flush(&mut stream)?;

        // Drain output after command response too.
        drain_output_taps(&mut session, &output_rx, &mut stream)?;

        // Check subscriptions periodically (every ~1s).
        if last_subscription_check.elapsed() >= std::time::Duration::from_secs(1) {
            let sub_notifs = super::handlers::check_subscriptions(&mut session.ctx);
            for notif in sub_notifs {
                std::io::Write::write_all(&mut stream, notif.as_bytes())?;
            }
            if !session.ctx.subscriptions.is_empty() {
                std::io::Write::flush(&mut stream)?;
            }
            last_subscription_check = std::time::Instant::now();
        }

        // If detach was requested, send %exit and close the connection.
        if session.ctx.detach_requested {
            let exit = exit_notification(None);
            std::io::Write::write_all(&mut stream, exit.as_bytes())?;
            std::io::Write::flush(&mut stream)?;
            log::info!("tmux CC: client detached");
            return Ok(());
        }
    }
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// Start the tmux CC compatibility listener.
///
/// On Windows, binds a TCP listener on `127.0.0.1:0` (random port) because
/// `uds_windows` AF_UNIX sockets have unreliable data delivery in the
/// WezTerm process environment.  On Unix, uses a Unix domain socket at
/// `socket_path`.
///
/// Returns the address string to set in `WEZTERM_TMUX_CC`:
/// - Unix: the socket file path
/// - Windows: `tcp:127.0.0.1:PORT`
///
/// Spawns a background thread that accepts connections.  Each connection is
/// handled synchronously on its own thread.
pub fn start_tmux_compat_listener(_socket_path: &std::path::Path) -> anyhow::Result<String> {
    #[cfg(windows)]
    {
        start_tmux_compat_listener_tcp()
    }
    #[cfg(not(windows))]
    {
        start_tmux_compat_listener_uds(_socket_path)
    }
}

/// TCP-based listener for Windows.
#[cfg(windows)]
fn start_tmux_compat_listener_tcp() -> anyhow::Result<String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let addr_str = format!("tcp:{}", addr);
    log::info!("tmux CC compat listener started on {}", addr_str);

    let addr_for_thread = addr_str.clone();
    let _thread = std::thread::Builder::new()
        .name("tmux-cc-listener".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        // Disable Nagle — without this, writes after the
                        // initial handshake stall due to delayed-ACK
                        // interaction on Windows localhost TCP.
                        let _ = stream.set_nodelay(true);
                        log::info!("tmux CC: accepted new TCP connection");
                        let addr = addr_for_thread.clone();
                        std::thread::Builder::new()
                            .name("tmux-cc-conn".to_string())
                            .spawn(move || {
                                if let Err(e) = process_cc_connection_sync(stream, &addr) {
                                    log::error!("tmux CC connection error: {}", e);
                                }
                            })
                            .ok();
                    }
                    Err(e) => {
                        log::error!("tmux CC accept error: {}", e);
                    }
                }
            }
        })?;

    Ok(addr_str)
}

/// UDS-based listener for Unix.
#[cfg(not(windows))]
fn start_tmux_compat_listener_uds(socket_path: &std::path::Path) -> anyhow::Result<String> {
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }

    let listener = wezterm_uds::UnixListener::bind(socket_path)?;
    let addr_str = socket_path.to_string_lossy().to_string();
    log::info!("tmux CC compat listener started on {}", addr_str);

    let addr_for_thread = addr_str.clone();
    let _thread = std::thread::Builder::new()
        .name("tmux-cc-listener".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        log::info!("tmux CC: accepted new connection");
                        let addr = addr_for_thread.clone();
                        std::thread::Builder::new()
                            .name("tmux-cc-conn".to_string())
                            .spawn(move || {
                                if let Err(e) = process_cc_connection_sync(stream, &addr) {
                                    log::error!("tmux CC connection error: {}", e);
                                }
                            })
                            .ok();
                    }
                    Err(e) => {
                        log::error!("tmux CC accept error: {}", e);
                    }
                }
            }
        })?;

    Ok(addr_str)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- extract_lines tests ---

    #[test]
    fn extract_lines_single_complete() {
        let mut buf = "hello\n".to_string();
        let lines = extract_lines(&mut buf);
        assert_eq!(lines, vec!["hello"]);
        assert_eq!(buf, "");
    }

    #[test]
    fn extract_lines_multiple() {
        let mut buf = "line1\nline2\nline3\n".to_string();
        let lines = extract_lines(&mut buf);
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
        assert_eq!(buf, "");
    }

    #[test]
    fn extract_lines_partial() {
        let mut buf = "line1\npartial".to_string();
        let lines = extract_lines(&mut buf);
        assert_eq!(lines, vec!["line1"]);
        assert_eq!(buf, "partial");
    }

    #[test]
    fn extract_lines_no_newline() {
        let mut buf = "no newline".to_string();
        let lines = extract_lines(&mut buf);
        assert!(lines.is_empty());
        assert_eq!(buf, "no newline");
    }

    #[test]
    fn extract_lines_empty_buffer() {
        let mut buf = String::new();
        let lines = extract_lines(&mut buf);
        assert!(lines.is_empty());
        assert_eq!(buf, "");
    }

    #[test]
    fn extract_lines_empty_line() {
        let mut buf = "\n".to_string();
        let lines = extract_lines(&mut buf);
        assert_eq!(lines, vec![""]);
        assert_eq!(buf, "");
    }

    #[test]
    fn extract_lines_consecutive_accumulation() {
        let mut buf = "partial".to_string();
        assert!(extract_lines(&mut buf).is_empty());
        buf.push_str(" data\nmore");
        let lines = extract_lines(&mut buf);
        assert_eq!(lines, vec!["partial data"]);
        assert_eq!(buf, "more");
        buf.push_str(" stuff\n");
        let lines = extract_lines(&mut buf);
        assert_eq!(lines, vec!["more stuff"]);
        assert_eq!(buf, "");
    }

    // --- build_layout tests ---

    #[test]
    fn build_split_node_horizontal() {
        let mut ctx = HandlerContext::new("test".to_string());
        // Two panes side by side (same top=0)
        let node = build_split_node(
            &mut ctx,
            &[], // We test via build_layout_for_tab indirectly, but
            // for the node builder we need PositionedPane which
            // requires Pane trait objects, so test the logic at a
            // higher level.
            160,
            40,
        );
        // Empty panes → empty children
        match node {
            LayoutNode::HorizontalSplit { children, .. } => {
                assert!(children.is_empty());
            }
            _ => panic!("expected HorizontalSplit for empty panes"),
        }
    }

    // --- translate_notification tests ---

    #[test]
    fn translate_ignores_pane_output() {
        let mut session = TmuxCompatSession::new("test".to_string());
        assert!(translate_notification(&mut session, MuxNotification::PaneOutput(0)).is_none());
    }

    #[test]
    fn translate_ignores_empty() {
        let mut session = TmuxCompatSession::new("test".to_string());
        assert!(translate_notification(&mut session, MuxNotification::Empty).is_none());
    }

    #[test]
    fn translate_ignores_pane_added() {
        let mut session = TmuxCompatSession::new("test".to_string());
        assert!(translate_notification(&mut session, MuxNotification::PaneAdded(0)).is_none());
    }

    #[test]
    fn translate_tab_title_changed_unknown_tab() {
        let mut session = TmuxCompatSession::new("test".to_string());
        // Tab not in id_map → None
        let notif = MuxNotification::TabTitleChanged {
            tab_id: 999,
            title: "new title".to_string(),
        };
        assert!(translate_notification(&mut session, notif).is_none());
    }

    #[test]
    fn translate_tab_title_changed_known_tab() {
        let mut session = TmuxCompatSession::new("test".to_string());
        let tmux_wid = session.ctx.id_map.get_or_create_tmux_window_id(42);
        let notif = MuxNotification::TabTitleChanged {
            tab_id: 42,
            title: "editor".to_string(),
        };
        let result = translate_notification(&mut session, notif);
        assert_eq!(
            result,
            Some(window_renamed_notification(tmux_wid, "editor"))
        );
    }

    #[test]
    fn translate_pane_removed_cleans_id_map() {
        let mut session = TmuxCompatSession::new("test".to_string());
        session.ctx.id_map.get_or_create_tmux_pane_id(10);
        assert!(session.ctx.id_map.tmux_pane_id(10).is_some());

        let result = translate_notification(&mut session, MuxNotification::PaneRemoved(10));
        assert!(result.is_none());
        assert!(session.ctx.id_map.tmux_pane_id(10).is_none());
    }

    #[test]
    fn translate_tab_added_registers_window() {
        let mut session = TmuxCompatSession::new("test".to_string());
        let notif = MuxNotification::TabAddedToWindow {
            tab_id: 5,
            window_id: 1,
        };
        let result = translate_notification(&mut session, notif);
        // Even without Mux, should get window-add with new tmux ID
        let tmux_wid = session.ctx.id_map.tmux_window_id(5).unwrap();
        assert_eq!(result, Some(window_add_notification(tmux_wid)));
    }

    // --- initial handshake tests ---

    #[test]
    fn initial_handshake_starts_with_begin_end() {
        let mut session = TmuxCompatSession::new("default".to_string());
        let handshake = build_initial_handshake(&mut session);
        // Should start with %begin ... %end (the greeting)
        assert!(handshake.starts_with("%begin "));
        // Should contain session-changed
        assert!(handshake.contains("%session-changed"));
    }

    #[test]
    fn initial_handshake_session_id_matches() {
        let mut session = TmuxCompatSession::new("myworkspace".to_string());
        let handshake = build_initial_handshake(&mut session);
        // The session ID allocated should be $0
        assert!(handshake.contains("%session-changed $0 myworkspace"));
    }

    // --- Phase 6: notification tests ---

    #[test]
    fn translate_workspace_renamed_emits_session_renamed() {
        let mut session = TmuxCompatSession::new("old".to_string());
        // Pre-register the workspace so it has a tmux session ID
        session.ctx.id_map.get_or_create_tmux_session_id("old");
        let notif = MuxNotification::WorkspaceRenamed {
            old_workspace: "old".to_string(),
            new_workspace: "new".to_string(),
        };
        let result = translate_notification(&mut session, notif);
        assert_eq!(result, Some(session_renamed_notification(0, "new")));
        // Verify id_map was re-keyed
        assert_eq!(session.ctx.id_map.tmux_session_id("old"), None);
        assert_eq!(session.ctx.id_map.tmux_session_id("new"), Some(0));
    }

    #[test]
    fn translate_workspace_renamed_unknown_returns_none() {
        let mut session = TmuxCompatSession::new("test".to_string());
        let notif = MuxNotification::WorkspaceRenamed {
            old_workspace: "unknown".to_string(),
            new_workspace: "new".to_string(),
        };
        assert!(translate_notification(&mut session, notif).is_none());
    }

    #[test]
    fn translate_window_removed_emits_window_close() {
        let mut session = TmuxCompatSession::new("test".to_string());
        // Set up: register two tabs in mux window 1
        let tmux_w0 = session.ctx.id_map.get_or_create_tmux_window_id(10);
        let tmux_w1 = session.ctx.id_map.get_or_create_tmux_window_id(20);
        session.ctx.id_map.track_tab_in_window(1, 10, "test");
        session.ctx.id_map.track_tab_in_window(1, 20, "test");

        let result = translate_notification(&mut session, MuxNotification::WindowRemoved(1));
        let out = result.unwrap();
        // Should contain %window-close for both tabs
        assert!(out.contains(&format!("%window-close @{}", tmux_w0)));
        assert!(out.contains(&format!("%window-close @{}", tmux_w1)));
        // Tab mappings should be cleaned up
        assert!(session.ctx.id_map.tmux_window_id(10).is_none());
        assert!(session.ctx.id_map.tmux_window_id(20).is_none());
    }

    #[test]
    fn translate_window_removed_unknown_returns_none() {
        let mut session = TmuxCompatSession::new("test".to_string());
        // No tabs tracked for mux window 999
        assert!(
            translate_notification(&mut session, MuxNotification::WindowRemoved(999)).is_none()
        );
    }

    #[test]
    fn translate_window_created_without_mux_returns_none() {
        let mut session = TmuxCompatSession::new("test".to_string());
        // Without Mux singleton, WindowCreated can't look up workspace
        assert!(translate_notification(&mut session, MuxNotification::WindowCreated(1)).is_none());
    }

    // --- Phase 9: %paste-buffer-changed tests ---

    #[test]
    fn translate_assign_clipboard_emits_paste_buffer_changed() {
        let mut session = TmuxCompatSession::new("test".to_string());
        let notif = MuxNotification::AssignClipboard {
            pane_id: 0,
            selection: wezterm_term::ClipboardSelection::Clipboard,
            clipboard: Some("hello".to_string()),
        };
        let result = translate_notification(&mut session, notif);
        assert_eq!(result, Some("%paste-buffer-changed buffer0\n".to_string()));
    }

    #[test]
    fn translate_assign_clipboard_none_content() {
        let mut session = TmuxCompatSession::new("test".to_string());
        let notif = MuxNotification::AssignClipboard {
            pane_id: 0,
            selection: wezterm_term::ClipboardSelection::PrimarySelection,
            clipboard: None,
        };
        let result = translate_notification(&mut session, notif);
        assert_eq!(result, Some("%paste-buffer-changed buffer0\n".to_string()));
    }

    // --- Phase 9: %session-window-changed tests ---

    #[test]
    fn translate_window_invalidated_without_mux_returns_none() {
        let mut session = TmuxCompatSession::new("test".to_string());
        // Without Mux singleton, can't look up window
        assert!(
            translate_notification(&mut session, MuxNotification::WindowInvalidated(1)).is_none()
        );
    }

    #[test]
    fn translate_window_invalidated_suppressed() {
        let mut session = TmuxCompatSession::new("test".to_string());
        session.ctx.suppress_window_changed = 2;
        let result = translate_notification(&mut session, MuxNotification::WindowInvalidated(1));
        assert!(result.is_none());
        assert_eq!(session.ctx.suppress_window_changed, 1);
    }

    #[test]
    fn translate_window_invalidated_suppression_decrements() {
        let mut session = TmuxCompatSession::new("test".to_string());
        session.ctx.suppress_window_changed = 1;
        translate_notification(&mut session, MuxNotification::WindowInvalidated(1));
        assert_eq!(session.ctx.suppress_window_changed, 0);
        // Next one should NOT be suppressed (but will return None without Mux)
        let result = translate_notification(&mut session, MuxNotification::WindowInvalidated(1));
        // Without Mux, returns None (can't look up window)
        assert!(result.is_none());
        assert_eq!(session.ctx.suppress_window_changed, 0);
    }

    // --- Phase 9: handler context defaults ---

    #[test]
    fn session_last_active_tab_default_empty() {
        let session = TmuxCompatSession::new("test".to_string());
        assert!(session.ctx.last_active_tab.is_empty());
        assert_eq!(session.ctx.suppress_window_changed, 0);
    }
}
