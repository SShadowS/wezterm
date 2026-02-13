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
        "resize-pane" => parse_resize_pane(args),
        "resize-window" => parse_resize_window(args),
        "refresh-client" => parse_refresh_client(args),
        "display-message" => parse_display_message(args),
        "has-session" => parse_has_session(args),
        "list-commands" => Ok(TmuxCliCommand::ListCommands),
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

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
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
}
