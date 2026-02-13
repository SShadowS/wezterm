//! CC (control mode) protocol server for the tmux compatibility layer.
//!
//! Accepts connections on a Unix domain socket, reads tmux commands as text
//! lines, dispatches them through Phase 2's `dispatch_command`, and writes
//! `%begin`/`%end` response blocks.  Mux notifications are forwarded as
//! CC-style `%`-prefixed notification lines.

use std::sync::Arc;

use futures::FutureExt;
use smol::prelude::*;

use crate::tab::{PositionedPane, Tab};
use crate::{Mux, MuxNotification};

use super::command_parser::parse_command;
use super::handlers::{dispatch_command, HandlerContext};
use super::layout::{generate_layout_string, LayoutNode};
use super::response::{
    exit_notification, layout_change_notification, session_changed_notification,
    window_add_notification, window_pane_changed_notification, window_renamed_notification,
    ResponseWriter,
};

// ---------------------------------------------------------------------------
// CC channel item
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum CcItem {
    Notif(MuxNotification),
    Readable,
}

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
            ctx: HandlerContext::new(workspace),
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

        MuxNotification::TabAddedToWindow { tab_id, .. } => {
            let tmux_wid = session.ctx.id_map.get_or_create_tmux_window_id(tab_id);
            // Also register any panes in the new tab
            if let Some(mux) = Mux::try_get() {
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

        MuxNotification::WindowRemoved(_window_id) => {
            // We don't have a direct mux-window→tabs mapping in id_map, so
            // just emit nothing here.  Layout changes and tab removals
            // will handle the actual cleanup.
            None
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

        // Notifications with no CC equivalent — silently ignore.
        MuxNotification::PaneOutput(_)
        | MuxNotification::PaneAdded(_)
        | MuxNotification::WindowCreated(_)
        | MuxNotification::WindowInvalidated(_)
        | MuxNotification::WindowWorkspaceChanged(_)
        | MuxNotification::ActiveWorkspaceChanged(_)
        | MuxNotification::Alert { .. }
        | MuxNotification::Empty
        | MuxNotification::AssignClipboard { .. }
        | MuxNotification::SaveToDownloads { .. }
        | MuxNotification::WindowTitleChanged { .. }
        | MuxNotification::WorkspaceRenamed { .. } => None,
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

// Platform-specific trait for raw descriptor access, mirroring
// `wezterm-mux-server-impl/src/dispatch.rs`.
#[cfg(unix)]
pub trait AsRawDesc: std::os::unix::io::AsRawFd + std::os::fd::AsFd {}
#[cfg(windows)]
pub trait AsRawDesc: std::os::windows::io::AsRawSocket + std::os::windows::io::AsSocket {}

impl AsRawDesc for wezterm_uds::UnixStream {}

/// Process a single CC protocol connection.
///
/// Follows the same async pattern as `wezterm-mux-server-impl/src/dispatch.rs`:
/// uses `smol::Async<T>`, `smol::channel`, and `smol::future::or` to
/// multiplex between reading commands and receiving Mux notifications.
pub async fn process_cc_connection<T>(stream: T) -> anyhow::Result<()>
where
    T: 'static + std::io::Read + std::io::Write + std::fmt::Debug + AsRawDesc + async_io::IoSafe,
{
    let mut stream = smol::Async::new(stream)?;

    let workspace = {
        let mux = Mux::get();
        mux.active_workspace().to_string()
    };

    let mut session = TmuxCompatSession::new(workspace);

    // Subscribe to Mux notifications
    let (item_tx, item_rx) = smol::channel::unbounded::<CcItem>();
    {
        let mux = Mux::get();
        let tx = item_tx.clone();
        mux.subscribe(move |n| tx.try_send(CcItem::Notif(n)).is_ok());
    }

    // Send initial handshake
    let handshake = build_initial_handshake(&mut session);
    stream.write_all(handshake.as_bytes()).await?;
    stream.flush().await?;

    // Main loop
    let mut read_buf = [0u8; 4096];
    loop {
        let rx_msg = item_rx.recv();
        let wait_for_read = stream.readable().map(|_| Ok(CcItem::Readable));

        match smol::future::or(rx_msg, wait_for_read).await {
            Ok(CcItem::Readable) => {
                let n = match stream.read(&mut read_buf).await {
                    Ok(0) => {
                        // EOF — client disconnected
                        log::trace!("CC client disconnected (EOF)");
                        return Ok(());
                    }
                    Ok(n) => n,
                    Err(e) => {
                        log::debug!("CC read error: {}", e);
                        return Ok(());
                    }
                };

                // Append to line buffer
                let chunk = String::from_utf8_lossy(&read_buf[..n]);
                session.line_buffer.push_str(&chunk);

                // Process complete lines
                let lines = extract_lines(&mut session.line_buffer);
                for line in lines {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }

                    let response = process_single_command(&mut session, &line).await;
                    stream.write_all(response.as_bytes()).await?;
                    stream.flush().await?;
                }
            }

            Ok(CcItem::Notif(notif)) => {
                if let Some(notification_str) = translate_notification(&mut session, notif) {
                    match stream.write_all(notification_str.as_bytes()).await {
                        Ok(()) => {
                            if let Err(e) = stream.flush().await {
                                log::debug!("CC flush notification error: {}", e);
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            log::debug!("CC write notification error: {}", e);
                            return Ok(());
                        }
                    }
                }
            }

            Err(_) => {
                // Channel closed — session is ending
                log::trace!("CC notification channel closed");
                let exit = exit_notification(None);
                let _ = stream.write_all(exit.as_bytes()).await;
                let _ = stream.flush().await;
                return Ok(());
            }
        }
    }
}

/// Parse and dispatch a single command line, returning the formatted response.
async fn process_single_command(session: &mut TmuxCompatSession, line: &str) -> String {
    match parse_command(line) {
        Ok(cmd) => match dispatch_command(&mut session.ctx, cmd).await {
            Ok(body) => {
                if body.is_empty() {
                    session.writer.empty_success()
                } else {
                    session.writer.success(&body)
                }
            }
            Err(e) => session.writer.error(&e),
        },
        Err(e) => session.writer.error(&format!("{}", e)),
    }
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// Start the tmux CC compatibility listener on the given socket path.
///
/// Spawns a background thread that accepts connections.  Each connection is
/// handed off to the main async executor via `spawn_into_main_thread` +
/// `promise::spawn::spawn` (local, non-Send) since the async domain methods
/// used by `dispatch_command` return non-Send futures.
pub fn start_tmux_compat_listener(socket_path: &std::path::Path) -> anyhow::Result<()> {
    // Remove stale socket if it exists
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }

    let listener = wezterm_uds::UnixListener::bind(socket_path)?;
    log::info!(
        "tmux CC compat listener started on {}",
        socket_path.display()
    );

    let _thread = std::thread::Builder::new()
        .name("tmux-cc-listener".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        log::trace!("tmux CC: accepted new connection");
                        // Use spawn_into_main_thread to hop to the main
                        // thread, then spawn the non-Send connection future
                        // locally on that thread.
                        promise::spawn::spawn_into_main_thread(async move {
                            promise::spawn::spawn(async move {
                                if let Err(e) = process_cc_connection(stream).await {
                                    log::error!("tmux CC connection error: {}", e);
                                }
                            })
                            .detach();
                        })
                        .detach();
                    }
                    Err(e) => {
                        log::error!("tmux CC accept error: {}", e);
                    }
                }
            }
        })?;

    Ok(())
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
}
