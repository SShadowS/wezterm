//! Parser for tmux CLI commands as sent over the control mode protocol.
//!
//! In tmux control mode (CC), clients send standard tmux commands as text lines.
//! This module parses those command strings into structured [`TmuxCliCommand`]
//! variants so the compatibility server can dispatch them.

use anyhow::{bail, Result};

/// A parsed tmux command with its flags and arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxCliCommand {
    SplitWindow {
        horizontal: bool,
        vertical: bool,
        target: Option<String>,
        size: Option<String>,
    },
    SendKeys {
        target: Option<String>,
        literal: bool,
        hex: bool,
        keys: Vec<String>,
    },
    CapturePane {
        print: bool,
        target: Option<String>,
        escape: bool,
        octal_escape: bool,
        start_line: Option<i64>,
        end_line: Option<i64>,
    },
    ListPanes {
        all: bool,
        session: bool,
        format: Option<String>,
        target: Option<String>,
    },
    ListWindows {
        all: bool,
        format: Option<String>,
        target: Option<String>,
    },
    ListSessions {
        format: Option<String>,
    },
    NewWindow {
        target: Option<String>,
        name: Option<String>,
    },
    SelectWindow {
        target: Option<String>,
    },
    SelectPane {
        target: Option<String>,
    },
    KillPane {
        target: Option<String>,
    },
    ResizePane {
        target: Option<String>,
        width: Option<u64>,
        height: Option<u64>,
        zoom: bool,
    },
    ResizeWindow {
        target: Option<String>,
        width: Option<u64>,
        height: Option<u64>,
    },
    RefreshClient {
        size: Option<String>,
        flags: Option<String>,
    },
    DisplayMessage {
        print: bool,
        format: Option<String>,
    },
    HasSession {
        target: Option<String>,
    },
    ListCommands,
    KillWindow {
        target: Option<String>,
    },
    KillSession {
        target: Option<String>,
    },
    RenameWindow {
        target: Option<String>,
        name: String,
    },
    RenameSession {
        target: Option<String>,
        name: String,
    },
    NewSession {
        name: Option<String>,
    },
    ShowOptions {
        global: bool,
        value_only: bool,
        option_name: Option<String>,
    },
    ShowWindowOptions {
        global: bool,
        value_only: bool,
        option_name: Option<String>,
    },
    AttachSession {
        target: Option<String>,
    },
    DetachClient,
    SwitchClient {
        target: Option<String>,
    },
    ListClients {
        format: Option<String>,
        target: Option<String>,
    },
}

/// Parse a tmux command line into a structured [`TmuxCliCommand`].
///
/// The input `line` is the raw text sent by the client, e.g.
/// `"send-keys -t %5 \"echo hello\" Enter"`.
///
/// Uses `shell_words::split` for proper handling of quoted arguments.
pub fn parse_command(line: &str) -> Result<TmuxCliCommand> {
    let line = line.trim();
    if line.is_empty() {
        bail!("empty command");
    }

    let words = shell_words::split(line)?;
    if words.is_empty() {
        bail!("empty command after splitting");
    }

    let command_name = &words[0];
    let args = &words[1..];

    match command_name.as_str() {
        "split-window" => parse_split_window(args),
        "send-keys" => parse_send_keys(args),
        "capture-pane" => parse_capture_pane(args),
        "list-panes" => parse_list_panes(args),
        "list-windows" => parse_list_windows(args),
        "list-sessions" => parse_list_sessions(args),
        "new-window" => parse_new_window(args),
        "select-window" => parse_select_window(args),
        "select-pane" => parse_select_pane(args),
        "kill-pane" => parse_kill_pane(args),
        "resize-pane" | "resizep" => parse_resize_pane(args),
        "resize-window" => parse_resize_window(args),
        "refresh-client" => parse_refresh_client(args),
        "display-message" => parse_display_message(args),
        "has-session" => parse_has_session(args),
        "list-commands" | "lscm" => Ok(TmuxCliCommand::ListCommands),
        "kill-window" | "killw" => parse_kill_window(args),
        "kill-session" => parse_kill_session(args),
        "rename-window" | "renamew" => parse_rename_window(args),
        "rename-session" | "rename" => parse_rename_session(args),
        "new-session" | "new" => parse_new_session(args),
        "show-options" | "show" | "show-option" => parse_show_options(args),
        "show-window-options" | "showw" | "show-window-option" => parse_show_window_options(args),
        "attach-session" | "attach" => parse_attach_session(args),
        "detach-client" | "detach" => parse_detach_client(args),
        "switch-client" | "switchc" => parse_switch_client(args),
        "list-clients" | "lsc" => parse_list_clients(args),
        other => bail!("unknown tmux command: {other:?}"),
    }
}

/// Helper: consume a flag's required value from the argument iterator.
///
/// Returns an error if the iterator is exhausted (the flag was provided
/// without a value).
fn take_flag_value<'a>(flag: &str, iter: &mut impl Iterator<Item = &'a str>) -> Result<String> {
    match iter.next() {
        Some(val) => Ok(val.to_string()),
        None => bail!("flag {flag} requires a value"),
    }
}

fn parse_split_window(args: &[String]) -> Result<TmuxCliCommand> {
    let mut horizontal = false;
    let mut vertical = false;
    let mut target = None;
    let mut size = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-h" => horizontal = true,
            "-v" => vertical = true,
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-l" => size = Some(take_flag_value("-l", &mut iter)?),
            other => bail!("split-window: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::SplitWindow {
        horizontal,
        vertical,
        target,
        size,
    })
}

fn parse_send_keys(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;
    let mut literal = false;
    let mut hex = false;
    let mut keys = Vec::new();

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-l" => literal = true,
            "-H" => hex = true,
            _ => {
                // First non-flag argument: this and everything remaining are keys.
                keys.push(arg.to_string());
                for rest in iter.by_ref() {
                    keys.push(rest.to_string());
                }
            }
        }
    }

    Ok(TmuxCliCommand::SendKeys {
        target,
        literal,
        hex,
        keys,
    })
}

fn parse_capture_pane(args: &[String]) -> Result<TmuxCliCommand> {
    let mut print = false;
    let mut target = None;
    let mut escape = false;
    let mut octal_escape = false;
    let mut start_line = None;
    let mut end_line = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-p" => print = true,
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-e" => escape = true,
            "-C" => octal_escape = true,
            "-S" => {
                let val = take_flag_value("-S", &mut iter)?;
                start_line =
                    Some(val.parse::<i64>().map_err(|_| {
                        anyhow::anyhow!("capture-pane -S: invalid number: {val:?}")
                    })?);
            }
            "-E" => {
                let val = take_flag_value("-E", &mut iter)?;
                end_line =
                    Some(val.parse::<i64>().map_err(|_| {
                        anyhow::anyhow!("capture-pane -E: invalid number: {val:?}")
                    })?);
            }
            other => bail!("capture-pane: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::CapturePane {
        print,
        target,
        escape,
        octal_escape,
        start_line,
        end_line,
    })
}

fn parse_list_panes(args: &[String]) -> Result<TmuxCliCommand> {
    let mut all = false;
    let mut session = false;
    let mut format = None;
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-a" => all = true,
            "-s" => session = true,
            "-F" => format = Some(take_flag_value("-F", &mut iter)?),
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("list-panes: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::ListPanes {
        all,
        session,
        format,
        target,
    })
}

fn parse_list_windows(args: &[String]) -> Result<TmuxCliCommand> {
    let mut all = false;
    let mut format = None;
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-a" => all = true,
            "-F" => format = Some(take_flag_value("-F", &mut iter)?),
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("list-windows: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::ListWindows {
        all,
        format,
        target,
    })
}

fn parse_list_sessions(args: &[String]) -> Result<TmuxCliCommand> {
    let mut format = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-F" => format = Some(take_flag_value("-F", &mut iter)?),
            other => bail!("list-sessions: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::ListSessions { format })
}

fn parse_new_window(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;
    let mut name = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-n" => name = Some(take_flag_value("-n", &mut iter)?),
            other => bail!("new-window: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::NewWindow { target, name })
}

fn parse_select_window(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("select-window: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::SelectWindow { target })
}

fn parse_select_pane(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("select-pane: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::SelectPane { target })
}

fn parse_kill_pane(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("kill-pane: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::KillPane { target })
}

fn parse_resize_pane(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;
    let mut width = None;
    let mut height = None;
    let mut zoom = false;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-Z" => zoom = true,
            "-x" => {
                let val = take_flag_value("-x", &mut iter)?;
                width = Some(
                    val.parse::<u64>()
                        .map_err(|_| anyhow::anyhow!("resize-pane -x: invalid number: {val:?}"))?,
                );
            }
            "-y" => {
                let val = take_flag_value("-y", &mut iter)?;
                height = Some(
                    val.parse::<u64>()
                        .map_err(|_| anyhow::anyhow!("resize-pane -y: invalid number: {val:?}"))?,
                );
            }
            other => bail!("resize-pane: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::ResizePane {
        target,
        width,
        height,
        zoom,
    })
}

fn parse_resize_window(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;
    let mut width = None;
    let mut height = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-x" => {
                let val = take_flag_value("-x", &mut iter)?;
                width =
                    Some(val.parse::<u64>().map_err(|_| {
                        anyhow::anyhow!("resize-window -x: invalid number: {val:?}")
                    })?);
            }
            "-y" => {
                let val = take_flag_value("-y", &mut iter)?;
                height =
                    Some(val.parse::<u64>().map_err(|_| {
                        anyhow::anyhow!("resize-window -y: invalid number: {val:?}")
                    })?);
            }
            other => bail!("resize-window: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::ResizeWindow {
        target,
        width,
        height,
    })
}

fn parse_refresh_client(args: &[String]) -> Result<TmuxCliCommand> {
    let mut size = None;
    let mut flags = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-C" => size = Some(take_flag_value("-C", &mut iter)?),
            "-f" => flags = Some(take_flag_value("-f", &mut iter)?),
            other => bail!("refresh-client: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::RefreshClient { size, flags })
}

fn parse_display_message(args: &[String]) -> Result<TmuxCliCommand> {
    let mut print = false;
    let mut format = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-p" => print = true,
            _ => {
                // The first non-flag argument is the format string.
                // tmux display-message takes at most one positional argument.
                format = Some(arg.to_string());
            }
        }
    }

    Ok(TmuxCliCommand::DisplayMessage { print, format })
}

fn parse_has_session(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("has-session: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::HasSession { target })
}

fn parse_kill_window(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("kill-window: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::KillWindow { target })
}

fn parse_kill_session(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("kill-session: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::KillSession { target })
}

fn parse_rename_window(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;
    let mut name = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            _ => {
                // Positional argument: the new name
                name = Some(arg.to_string());
            }
        }
    }

    let name = name.ok_or_else(|| anyhow::anyhow!("rename-window: missing new name"))?;
    Ok(TmuxCliCommand::RenameWindow { target, name })
}

fn parse_rename_session(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;
    let mut name = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            _ => {
                // Positional argument: the new name
                name = Some(arg.to_string());
            }
        }
    }

    let name = name.ok_or_else(|| anyhow::anyhow!("rename-session: missing new name"))?;
    Ok(TmuxCliCommand::RenameSession { target, name })
}

fn parse_new_session(args: &[String]) -> Result<TmuxCliCommand> {
    let mut name = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-s" => name = Some(take_flag_value("-s", &mut iter)?),
            other => bail!("new-session: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::NewSession { name })
}

fn parse_show_options(args: &[String]) -> Result<TmuxCliCommand> {
    let mut global = false;
    let mut value_only = false;
    let mut option_name = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-g" => global = true,
            "-v" => value_only = true,
            "-gv" | "-vg" => {
                global = true;
                value_only = true;
            }
            _ => {
                option_name = Some(arg.to_string());
            }
        }
    }

    Ok(TmuxCliCommand::ShowOptions {
        global,
        value_only,
        option_name,
    })
}

fn parse_show_window_options(args: &[String]) -> Result<TmuxCliCommand> {
    let mut global = false;
    let mut value_only = false;
    let mut option_name = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-g" => global = true,
            "-v" => value_only = true,
            "-gv" | "-vg" => {
                global = true;
                value_only = true;
            }
            _ => {
                option_name = Some(arg.to_string());
            }
        }
    }

    Ok(TmuxCliCommand::ShowWindowOptions {
        global,
        value_only,
        option_name,
    })
}

fn parse_attach_session(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("attach-session: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::AttachSession { target })
}

fn parse_detach_client(args: &[String]) -> Result<TmuxCliCommand> {
    // detach-client accepts -t (target client) and -s (target session) but
    // for CC mode we only need the bare command — ignore flags gracefully.
    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" | "-s" => {
                // Consume and ignore the value
                let _ = take_flag_value(arg, &mut iter)?;
            }
            other => bail!("detach-client: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::DetachClient)
}

fn parse_switch_client(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("switch-client: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::SwitchClient { target })
}

fn parse_list_clients(args: &[String]) -> Result<TmuxCliCommand> {
    let mut format = None;
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-F" => format = Some(take_flag_value("-F", &mut iter)?),
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            other => bail!("list-clients: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::ListClients { format, target })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to reduce boilerplate in assertions.
    fn parse(s: &str) -> TmuxCliCommand {
        parse_command(s).unwrap_or_else(|e| panic!("parse_command({s:?}) failed: {e}"))
    }

    // ---------------------------------------------------------------
    // split-window
    // ---------------------------------------------------------------

    #[test]
    fn split_window_horizontal() {
        assert_eq!(
            parse("split-window -h"),
            TmuxCliCommand::SplitWindow {
                horizontal: true,
                vertical: false,
                target: None,
                size: None,
            }
        );
    }

    #[test]
    fn split_window_vertical_with_target() {
        assert_eq!(
            parse("split-window -v -t %3"),
            TmuxCliCommand::SplitWindow {
                horizontal: false,
                vertical: true,
                target: Some("%3".into()),
                size: None,
            }
        );
    }

    #[test]
    fn split_window_default() {
        assert_eq!(
            parse("split-window"),
            TmuxCliCommand::SplitWindow {
                horizontal: false,
                vertical: false,
                target: None,
                size: None,
            }
        );
    }

    #[test]
    fn split_window_with_size() {
        assert_eq!(
            parse("split-window -h -l 50%"),
            TmuxCliCommand::SplitWindow {
                horizontal: true,
                vertical: false,
                target: None,
                size: Some("50%".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // send-keys
    // ---------------------------------------------------------------

    #[test]
    fn send_keys_with_target_and_quoted_keys() {
        assert_eq!(
            parse(r#"send-keys -t $0:@0.%1 "echo hello" Enter"#),
            TmuxCliCommand::SendKeys {
                target: Some("$0:@0.%1".into()),
                literal: false,
                hex: false,
                keys: vec!["echo hello".into(), "Enter".into()],
            }
        );
    }

    #[test]
    fn send_keys_hex_values() {
        assert_eq!(
            parse("send-keys -t %5 0x68 0x69 0xA"),
            TmuxCliCommand::SendKeys {
                target: Some("%5".into()),
                literal: false,
                hex: false,
                keys: vec!["0x68".into(), "0x69".into(), "0xA".into()],
            }
        );
    }

    #[test]
    fn send_keys_hex_flag() {
        assert_eq!(
            parse("send-keys -H -t %5 68 69 0A"),
            TmuxCliCommand::SendKeys {
                target: Some("%5".into()),
                literal: false,
                hex: true,
                keys: vec!["68".into(), "69".into(), "0A".into()],
            }
        );
    }

    #[test]
    fn send_keys_literal_flag() {
        assert_eq!(
            parse("send-keys -l -t %1 hello"),
            TmuxCliCommand::SendKeys {
                target: Some("%1".into()),
                literal: true,
                hex: false,
                keys: vec!["hello".into()],
            }
        );
    }

    #[test]
    fn send_keys_no_target() {
        assert_eq!(
            parse("send-keys Enter"),
            TmuxCliCommand::SendKeys {
                target: None,
                literal: false,
                hex: false,
                keys: vec!["Enter".into()],
            }
        );
    }

    // ---------------------------------------------------------------
    // capture-pane
    // ---------------------------------------------------------------

    #[test]
    fn capture_pane_print_with_target() {
        assert_eq!(
            parse("capture-pane -p -t %1"),
            TmuxCliCommand::CapturePane {
                print: true,
                target: Some("%1".into()),
                escape: false,
                octal_escape: false,
                start_line: None,
                end_line: None,
            }
        );
    }

    #[test]
    fn capture_pane_all_flags() {
        assert_eq!(
            parse("capture-pane -p -t %1 -e -C -S -32768"),
            TmuxCliCommand::CapturePane {
                print: true,
                target: Some("%1".into()),
                escape: true,
                octal_escape: true,
                start_line: Some(-32768),
                end_line: None,
            }
        );
    }

    #[test]
    fn capture_pane_start_and_end_lines() {
        assert_eq!(
            parse("capture-pane -p -S 0 -E 100"),
            TmuxCliCommand::CapturePane {
                print: true,
                target: None,
                escape: false,
                octal_escape: false,
                start_line: Some(0),
                end_line: Some(100),
            }
        );
    }

    // ---------------------------------------------------------------
    // list-panes
    // ---------------------------------------------------------------

    #[test]
    fn list_panes_all_with_format() {
        assert_eq!(
            parse("list-panes -a -F '#{pane_index} #{pane_id}'"),
            TmuxCliCommand::ListPanes {
                all: true,
                session: false,
                format: Some("#{pane_index} #{pane_id}".into()),
                target: None,
            }
        );
    }

    #[test]
    fn list_panes_session_with_target() {
        assert_eq!(
            parse("list-panes -s -t $0"),
            TmuxCliCommand::ListPanes {
                all: false,
                session: true,
                format: None,
                target: Some("$0".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // list-windows
    // ---------------------------------------------------------------

    #[test]
    fn list_windows_with_format() {
        assert_eq!(
            parse("list-windows -F '#{window_id} #{window_name}'"),
            TmuxCliCommand::ListWindows {
                all: false,
                format: Some("#{window_id} #{window_name}".into()),
                target: None,
            }
        );
    }

    #[test]
    fn list_windows_all() {
        assert_eq!(
            parse("list-windows -a"),
            TmuxCliCommand::ListWindows {
                all: true,
                format: None,
                target: None,
            }
        );
    }

    // ---------------------------------------------------------------
    // list-sessions
    // ---------------------------------------------------------------

    #[test]
    fn list_sessions_no_args() {
        assert_eq!(
            parse("list-sessions"),
            TmuxCliCommand::ListSessions { format: None }
        );
    }

    #[test]
    fn list_sessions_with_format() {
        assert_eq!(
            parse("list-sessions -F '#{session_id}'"),
            TmuxCliCommand::ListSessions {
                format: Some("#{session_id}".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // new-window
    // ---------------------------------------------------------------

    #[test]
    fn new_window_no_args() {
        assert_eq!(
            parse("new-window"),
            TmuxCliCommand::NewWindow {
                target: None,
                name: None,
            }
        );
    }

    #[test]
    fn new_window_with_name() {
        assert_eq!(
            parse("new-window -n mywin"),
            TmuxCliCommand::NewWindow {
                target: None,
                name: Some("mywin".into()),
            }
        );
    }

    #[test]
    fn new_window_with_target_and_name() {
        assert_eq!(
            parse("new-window -t $0 -n editor"),
            TmuxCliCommand::NewWindow {
                target: Some("$0".into()),
                name: Some("editor".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // select-window
    // ---------------------------------------------------------------

    #[test]
    fn select_window_with_target() {
        assert_eq!(
            parse("select-window -t @3"),
            TmuxCliCommand::SelectWindow {
                target: Some("@3".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // select-pane
    // ---------------------------------------------------------------

    #[test]
    fn select_pane_with_target() {
        assert_eq!(
            parse("select-pane -t %2"),
            TmuxCliCommand::SelectPane {
                target: Some("%2".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // kill-pane
    // ---------------------------------------------------------------

    #[test]
    fn kill_pane_with_target() {
        assert_eq!(
            parse("kill-pane -t %3"),
            TmuxCliCommand::KillPane {
                target: Some("%3".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // resize-pane
    // ---------------------------------------------------------------

    #[test]
    fn resize_pane_width_and_height() {
        assert_eq!(
            parse("resize-pane -t %1 -x 80 -y 24"),
            TmuxCliCommand::ResizePane {
                target: Some("%1".into()),
                width: Some(80),
                height: Some(24),
                zoom: false,
            }
        );
    }

    #[test]
    fn resize_pane_width_only() {
        assert_eq!(
            parse("resize-pane -x 120"),
            TmuxCliCommand::ResizePane {
                target: None,
                width: Some(120),
                height: None,
                zoom: false,
            }
        );
    }

    // ---------------------------------------------------------------
    // resize-window
    // ---------------------------------------------------------------

    #[test]
    fn resize_window_with_dimensions() {
        assert_eq!(
            parse("resize-window -t @1 -x 200 -y 50"),
            TmuxCliCommand::ResizeWindow {
                target: Some("@1".into()),
                width: Some(200),
                height: Some(50),
            }
        );
    }

    // ---------------------------------------------------------------
    // refresh-client
    // ---------------------------------------------------------------

    #[test]
    fn refresh_client_with_size() {
        assert_eq!(
            parse("refresh-client -C 160x40"),
            TmuxCliCommand::RefreshClient {
                size: Some("160x40".into()),
                flags: None,
            }
        );
    }

    #[test]
    fn refresh_client_with_flags() {
        assert_eq!(
            parse("refresh-client -f no-output"),
            TmuxCliCommand::RefreshClient {
                size: None,
                flags: Some("no-output".into()),
            }
        );
    }

    #[test]
    fn refresh_client_with_size_and_flags() {
        assert_eq!(
            parse("refresh-client -C 80x24 -f no-output"),
            TmuxCliCommand::RefreshClient {
                size: Some("80x24".into()),
                flags: Some("no-output".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // display-message
    // ---------------------------------------------------------------

    #[test]
    fn display_message_print_with_format() {
        assert_eq!(
            parse("display-message -p '#{session_id}'"),
            TmuxCliCommand::DisplayMessage {
                print: true,
                format: Some("#{session_id}".into()),
            }
        );
    }

    #[test]
    fn display_message_no_args() {
        assert_eq!(
            parse("display-message"),
            TmuxCliCommand::DisplayMessage {
                print: false,
                format: None,
            }
        );
    }

    #[test]
    fn display_message_format_only() {
        assert_eq!(
            parse("display-message '#{window_id}'"),
            TmuxCliCommand::DisplayMessage {
                print: false,
                format: Some("#{window_id}".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // has-session
    // ---------------------------------------------------------------

    #[test]
    fn has_session_with_target() {
        assert_eq!(
            parse("has-session -t mysession"),
            TmuxCliCommand::HasSession {
                target: Some("mysession".into()),
            }
        );
    }

    #[test]
    fn has_session_no_args() {
        assert_eq!(
            parse("has-session"),
            TmuxCliCommand::HasSession { target: None }
        );
    }

    // ---------------------------------------------------------------
    // list-commands
    // ---------------------------------------------------------------

    #[test]
    fn list_commands() {
        assert_eq!(parse("list-commands"), TmuxCliCommand::ListCommands);
    }

    // ---------------------------------------------------------------
    // Error cases
    // ---------------------------------------------------------------

    #[test]
    fn empty_command_is_error() {
        assert!(parse_command("").is_err());
    }

    #[test]
    fn whitespace_only_is_error() {
        assert!(parse_command("   ").is_err());
    }

    #[test]
    fn unknown_command_is_error() {
        assert!(parse_command("foobar").is_err());
    }

    #[test]
    fn missing_flag_value_is_error() {
        assert!(parse_command("split-window -t").is_err());
    }

    #[test]
    fn invalid_number_is_error() {
        assert!(parse_command("resize-pane -x notanumber").is_err());
    }

    #[test]
    fn capture_pane_invalid_start_line() {
        assert!(parse_command("capture-pane -S notanumber").is_err());
    }

    // ---------------------------------------------------------------
    // Whitespace / quoting edge cases
    // ---------------------------------------------------------------

    #[test]
    fn leading_and_trailing_whitespace() {
        assert_eq!(parse("  list-commands  "), TmuxCliCommand::ListCommands,);
    }

    #[test]
    fn double_quoted_format_string() {
        assert_eq!(
            parse(r##"list-panes -F "#{pane_id}""##),
            TmuxCliCommand::ListPanes {
                all: false,
                session: false,
                format: Some("#{pane_id}".into()),
                target: None,
            }
        );
    }

    #[test]
    fn send_keys_multiple_words_quoted() {
        assert_eq!(
            parse(r#"send-keys "ls -la" Enter"#),
            TmuxCliCommand::SendKeys {
                target: None,
                literal: false,
                hex: false,
                keys: vec!["ls -la".into(), "Enter".into()],
            }
        );
    }

    // ---------------------------------------------------------------
    // resize-pane -Z (zoom)
    // ---------------------------------------------------------------

    #[test]
    fn resize_pane_zoom() {
        assert_eq!(
            parse("resize-pane -Z -t %1"),
            TmuxCliCommand::ResizePane {
                target: Some("%1".into()),
                width: None,
                height: None,
                zoom: true,
            }
        );
    }

    #[test]
    fn resize_pane_zoom_no_target() {
        assert_eq!(
            parse("resize-pane -Z"),
            TmuxCliCommand::ResizePane {
                target: None,
                width: None,
                height: None,
                zoom: true,
            }
        );
    }

    #[test]
    fn resize_pane_alias_resizep() {
        assert_eq!(
            parse("resizep -Z"),
            TmuxCliCommand::ResizePane {
                target: None,
                width: None,
                height: None,
                zoom: true,
            }
        );
    }

    // ---------------------------------------------------------------
    // kill-window
    // ---------------------------------------------------------------

    #[test]
    fn kill_window_with_target() {
        assert_eq!(
            parse("kill-window -t @1"),
            TmuxCliCommand::KillWindow {
                target: Some("@1".into()),
            }
        );
    }

    #[test]
    fn kill_window_no_args() {
        assert_eq!(
            parse("kill-window"),
            TmuxCliCommand::KillWindow { target: None }
        );
    }

    #[test]
    fn kill_window_alias_killw() {
        assert_eq!(
            parse("killw -t @2"),
            TmuxCliCommand::KillWindow {
                target: Some("@2".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // kill-session
    // ---------------------------------------------------------------

    #[test]
    fn kill_session_with_target() {
        assert_eq!(
            parse("kill-session -t mysession"),
            TmuxCliCommand::KillSession {
                target: Some("mysession".into()),
            }
        );
    }

    #[test]
    fn kill_session_no_args() {
        assert_eq!(
            parse("kill-session"),
            TmuxCliCommand::KillSession { target: None }
        );
    }

    // ---------------------------------------------------------------
    // rename-window
    // ---------------------------------------------------------------

    #[test]
    fn rename_window_with_target() {
        assert_eq!(
            parse("rename-window -t @0 editor"),
            TmuxCliCommand::RenameWindow {
                target: Some("@0".into()),
                name: "editor".into(),
            }
        );
    }

    #[test]
    fn rename_window_name_only() {
        assert_eq!(
            parse("rename-window mywin"),
            TmuxCliCommand::RenameWindow {
                target: None,
                name: "mywin".into(),
            }
        );
    }

    #[test]
    fn rename_window_alias_renamew() {
        assert_eq!(
            parse("renamew newname"),
            TmuxCliCommand::RenameWindow {
                target: None,
                name: "newname".into(),
            }
        );
    }

    #[test]
    fn rename_window_missing_name_is_error() {
        assert!(parse_command("rename-window").is_err());
    }

    // ---------------------------------------------------------------
    // rename-session
    // ---------------------------------------------------------------

    #[test]
    fn rename_session_with_target() {
        assert_eq!(
            parse("rename-session -t $0 newname"),
            TmuxCliCommand::RenameSession {
                target: Some("$0".into()),
                name: "newname".into(),
            }
        );
    }

    #[test]
    fn rename_session_name_only() {
        assert_eq!(
            parse("rename-session work"),
            TmuxCliCommand::RenameSession {
                target: None,
                name: "work".into(),
            }
        );
    }

    #[test]
    fn rename_session_alias_rename() {
        assert_eq!(
            parse("rename newname"),
            TmuxCliCommand::RenameSession {
                target: None,
                name: "newname".into(),
            }
        );
    }

    #[test]
    fn rename_session_missing_name_is_error() {
        assert!(parse_command("rename-session").is_err());
    }

    // ---------------------------------------------------------------
    // new-session
    // ---------------------------------------------------------------

    #[test]
    fn new_session_with_name() {
        assert_eq!(
            parse("new-session -s work"),
            TmuxCliCommand::NewSession {
                name: Some("work".into()),
            }
        );
    }

    #[test]
    fn new_session_no_args() {
        assert_eq!(
            parse("new-session"),
            TmuxCliCommand::NewSession { name: None }
        );
    }

    #[test]
    fn new_session_alias_new() {
        assert_eq!(
            parse("new -s dev"),
            TmuxCliCommand::NewSession {
                name: Some("dev".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // show-options
    // ---------------------------------------------------------------

    #[test]
    fn show_options_global_value() {
        assert_eq!(
            parse("show-options -gv default-terminal"),
            TmuxCliCommand::ShowOptions {
                global: true,
                value_only: true,
                option_name: Some("default-terminal".into()),
            }
        );
    }

    #[test]
    fn show_options_global_only() {
        assert_eq!(
            parse("show-options -g"),
            TmuxCliCommand::ShowOptions {
                global: true,
                value_only: false,
                option_name: None,
            }
        );
    }

    #[test]
    fn show_options_alias_show() {
        assert_eq!(
            parse("show -gv escape-time"),
            TmuxCliCommand::ShowOptions {
                global: true,
                value_only: true,
                option_name: Some("escape-time".into()),
            }
        );
    }

    #[test]
    fn show_options_separate_flags() {
        assert_eq!(
            parse("show-options -g -v set-clipboard"),
            TmuxCliCommand::ShowOptions {
                global: true,
                value_only: true,
                option_name: Some("set-clipboard".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // show-window-options
    // ---------------------------------------------------------------

    #[test]
    fn show_window_options_global_value() {
        assert_eq!(
            parse("show-window-options -gv aggressive-resize"),
            TmuxCliCommand::ShowWindowOptions {
                global: true,
                value_only: true,
                option_name: Some("aggressive-resize".into()),
            }
        );
    }

    #[test]
    fn show_window_options_alias_showw() {
        assert_eq!(
            parse("showw -gv aggressive-resize"),
            TmuxCliCommand::ShowWindowOptions {
                global: true,
                value_only: true,
                option_name: Some("aggressive-resize".into()),
            }
        );
    }

    #[test]
    fn show_window_options_no_args() {
        assert_eq!(
            parse("show-window-options"),
            TmuxCliCommand::ShowWindowOptions {
                global: false,
                value_only: false,
                option_name: None,
            }
        );
    }

    // ---------------------------------------------------------------
    // attach-session
    // ---------------------------------------------------------------

    #[test]
    fn attach_session_with_target() {
        assert_eq!(
            parse("attach-session -t work"),
            TmuxCliCommand::AttachSession {
                target: Some("work".into()),
            }
        );
    }

    #[test]
    fn attach_session_no_args() {
        assert_eq!(
            parse("attach-session"),
            TmuxCliCommand::AttachSession { target: None }
        );
    }

    #[test]
    fn attach_session_alias_attach() {
        assert_eq!(
            parse("attach -t $1"),
            TmuxCliCommand::AttachSession {
                target: Some("$1".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // detach-client
    // ---------------------------------------------------------------

    #[test]
    fn detach_client_no_args() {
        assert_eq!(parse("detach-client"), TmuxCliCommand::DetachClient);
    }

    #[test]
    fn detach_client_alias_detach() {
        assert_eq!(parse("detach"), TmuxCliCommand::DetachClient);
    }

    #[test]
    fn detach_client_with_ignored_flags() {
        // -t and -s are accepted but ignored in CC mode
        assert_eq!(
            parse("detach-client -t myclient"),
            TmuxCliCommand::DetachClient
        );
        assert_eq!(
            parse("detach-client -s mysession"),
            TmuxCliCommand::DetachClient
        );
    }

    // ---------------------------------------------------------------
    // switch-client
    // ---------------------------------------------------------------

    #[test]
    fn switch_client_with_target() {
        assert_eq!(
            parse("switch-client -t work"),
            TmuxCliCommand::SwitchClient {
                target: Some("work".into()),
            }
        );
    }

    #[test]
    fn switch_client_no_args() {
        assert_eq!(
            parse("switch-client"),
            TmuxCliCommand::SwitchClient { target: None }
        );
    }

    #[test]
    fn switch_client_alias_switchc() {
        assert_eq!(
            parse("switchc -t $0"),
            TmuxCliCommand::SwitchClient {
                target: Some("$0".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // list-clients
    // ---------------------------------------------------------------

    #[test]
    fn list_clients_no_args() {
        assert_eq!(
            parse("list-clients"),
            TmuxCliCommand::ListClients {
                format: None,
                target: None,
            }
        );
    }

    #[test]
    fn list_clients_with_format_and_target() {
        assert_eq!(
            parse("list-clients -t $0 -F '#{client_name}'"),
            TmuxCliCommand::ListClients {
                format: Some("#{client_name}".into()),
                target: Some("$0".into()),
            }
        );
    }

    #[test]
    fn list_clients_alias_lsc() {
        assert_eq!(
            parse("lsc -F '#{client_name}\t#{client_control_mode}'"),
            TmuxCliCommand::ListClients {
                format: Some("#{client_name}\t#{client_control_mode}".into()),
                target: None,
            }
        );
    }

    #[test]
    fn list_clients_iterm2_style() {
        // iTerm2 sends: list-clients -t '$0' -F '#{client_name}\t#{client_control_mode}'
        assert_eq!(
            parse("list-clients -t '$0' -F '#{client_name}\t#{client_control_mode}'"),
            TmuxCliCommand::ListClients {
                format: Some("#{client_name}\t#{client_control_mode}".into()),
                target: Some("$0".into()),
            }
        );
    }
}
