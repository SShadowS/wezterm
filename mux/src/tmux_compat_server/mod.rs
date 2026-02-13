//! Tmux Control Mode Compatibility Server
//!
//! Makes WezTerm act as a tmux-compatible server so tools like Claude Code's
//! Agent Teams can use their existing tmux integration (split-window, send-keys,
//! capture-pane, list-panes) natively in WezTerm.

pub mod command_parser;
pub mod format;
pub mod handlers;
pub mod id_map;
pub mod layout;
pub mod response;
pub mod server;
pub mod target;
