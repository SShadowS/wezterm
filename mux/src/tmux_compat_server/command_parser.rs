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
        print_and_format: Option<String>,
        cwd: Option<String>,
        env: Vec<String>,
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
        print_and_format: Option<String>,
        cwd: Option<String>,
        env: Vec<String>,
    },
    SelectWindow {
        target: Option<String>,
    },
    SelectPane {
        target: Option<String>,
        style: Option<String>,
        title: Option<String>,
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
        adjust_pane: Option<String>,
        subscription: Option<String>,
    },
    DisplayMessage {
        print: bool,
        verbose: bool,
        format: Option<String>,
        target: Option<String>,
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
        window_name: Option<String>,
        detached: bool,
        print_and_format: Option<String>,
        cwd: Option<String>,
        env: Vec<String>,
    },
    ShowOptions {
        global: bool,
        value_only: bool,
        quiet: bool,
        option_name: Option<String>,
    },
    ShowWindowOptions {
        global: bool,
        value_only: bool,
        quiet: bool,
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
    // Phase 11: clipboard / buffer commands
    ShowBuffer {
        buffer_name: Option<String>,
    },
    SetBuffer {
        buffer_name: Option<String>,
        data: Option<String>,
        append: bool,
    },
    DeleteBuffer {
        buffer_name: Option<String>,
    },
    ListBuffers {
        format: Option<String>,
    },
    PasteBuffer {
        buffer_name: Option<String>,
        target: Option<String>,
        delete_after: bool,
        bracketed: bool,
    },
    // Phase 12.3: move commands
    MovePane {
        src: Option<String>,
        dst: Option<String>,
        horizontal: bool,
        before: bool,
    },
    MoveWindow {
        src: Option<String>,
        dst: Option<String>,
    },
    // Phase 12.4: copy mode bridge
    CopyMode {
        quit: bool,
        target: Option<String>,
    },
    // Phase 13: Claude Code agent teams compatibility
    SetOption {
        target: Option<String>,
        option_name: Option<String>,
        value: Option<String>,
    },
    SelectLayout {
        target: Option<String>,
        layout_name: Option<String>,
    },
    BreakPane {
        detach: bool,
        source: Option<String>,
        target: Option<String>,
    },
    // Phase 17: missing commands for cleanup & orchestration
    KillServer,
    WaitFor {
        signal: bool,
        channel: String,
    },
    PipePane {
        target: Option<String>,
        command: Option<String>,
        output: bool,
        input: bool,
        toggle: bool,
    },
    DisplayPopup {
        target: Option<String>,
    },
    RunShell {
        background: bool,
        target: Option<String>,
        command: Option<String>,
        /// Delay in seconds (stored as string to keep Eq derivation).
        delay: Option<String>,
    },
    // Phase 19: diagnostic & debugging
    ServerInfo,
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
        "split-window" | "splitw" => parse_split_window(args),
        "send-keys" | "send" => parse_send_keys(args),
        "capture-pane" | "capturep" => parse_capture_pane(args),
        "list-panes" | "lsp" => parse_list_panes(args),
        "list-windows" | "lsw" => parse_list_windows(args),
        "list-sessions" | "ls" => parse_list_sessions(args),
        "new-window" | "neww" => parse_new_window(args),
        "select-window" | "selectw" => parse_select_window(args),
        "select-pane" | "selectp" => parse_select_pane(args),
        "kill-pane" | "killp" => parse_kill_pane(args),
        "resize-pane" | "resizep" => parse_resize_pane(args),
        "resize-window" | "resizew" => parse_resize_window(args),
        "refresh-client" | "refresh" => parse_refresh_client(args),
        "display-message" | "display" => parse_display_message(args),
        "has-session" | "has" => parse_has_session(args),
        "list-commands" | "lscm" => Ok(TmuxCliCommand::ListCommands),
        "kill-window" | "killw" => parse_kill_window(args),
        "kill-session" | "kills" => parse_kill_session(args),
        "rename-window" | "renamew" => parse_rename_window(args),
        "rename-session" | "rename" => parse_rename_session(args),
        "new-session" | "new" => parse_new_session(args),
        "show-options" | "show" | "show-option" => parse_show_options(args),
        "show-window-options" | "showw" | "show-window-option" => parse_show_window_options(args),
        "attach-session" | "attach" => parse_attach_session(args),
        "detach-client" | "detach" => parse_detach_client(args),
        "switch-client" | "switchc" => parse_switch_client(args),
        "list-clients" | "lsc" => parse_list_clients(args),
        "show-buffer" | "showb" => parse_show_buffer(args),
        "set-buffer" | "setb" => parse_set_buffer(args),
        "delete-buffer" | "deleteb" => parse_delete_buffer(args),
        "list-buffers" | "lsb" => parse_list_buffers(args),
        "paste-buffer" | "pasteb" => parse_paste_buffer(args),
        "move-pane" | "movep" | "join-pane" | "joinp" => parse_move_pane(args),
        "move-window" | "movew" => parse_move_window(args),
        "copy-mode" => parse_copy_mode(args),
        // Phase 13: Claude Code agent teams compatibility
        "set-option" | "set" => parse_set_option(args),
        "select-layout" | "selectl" => parse_select_layout(args),
        "break-pane" | "breakp" => parse_break_pane(args),
        // Phase 17: missing commands for cleanup & orchestration
        "kill-server" => Ok(TmuxCliCommand::KillServer),
        "wait-for" | "wait" => parse_wait_for(args),
        "pipe-pane" | "pipep" => parse_pipe_pane(args),
        "display-popup" | "popup" | "display-menu" | "menu" => parse_display_popup(args),
        "run-shell" | "run" => parse_run_shell(args),
        // Phase 19: diagnostic & debugging
        "server-info" | "info" => Ok(TmuxCliCommand::ServerInfo),
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
    let mut print_info = false;
    let mut format = None;
    let mut cwd = None;
    let mut env = Vec::new();

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-h" => horizontal = true,
            "-v" => vertical = true,
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-l" | "-p" => size = Some(take_flag_value(arg, &mut iter)?),
            "-P" => print_info = true,
            "-F" => format = Some(take_flag_value("-F", &mut iter)?),
            // Flags we accept but ignore: -d (detach), -b (before), -f (full-width/height)
            "-d" | "-b" | "-f" | "-Z" | "-I" => {}
            "-e" => env.push(take_flag_value("-e", &mut iter)?),
            "-c" => cwd = Some(take_flag_value("-c", &mut iter)?),
            other => bail!("split-window: unexpected argument: {other:?}"),
        }
    }

    let print_and_format = if print_info {
        Some(format.unwrap_or_else(|| "#{session_name}:#{window_index}.#{pane_index}".to_string()))
    } else {
        None
    };

    Ok(TmuxCliCommand::SplitWindow {
        horizontal,
        vertical,
        target,
        size,
        print_and_format,
        cwd,
        env,
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
    let mut print_info = false;
    let mut format = None;
    let mut cwd = None;
    let mut env = Vec::new();

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-n" => name = Some(take_flag_value("-n", &mut iter)?),
            "-P" => print_info = true,
            "-F" => format = Some(take_flag_value("-F", &mut iter)?),
            // Flags we accept but ignore
            "-d" | "-S" | "-a" | "-b" | "-k" => {}
            "-e" => env.push(take_flag_value("-e", &mut iter)?),
            "-c" => cwd = Some(take_flag_value("-c", &mut iter)?),
            other => bail!("new-window: unexpected argument: {other:?}"),
        }
    }

    let print_and_format = if print_info {
        Some(format.unwrap_or_else(|| "#{session_name}:#{window_index}.#{pane_index}".to_string()))
    } else {
        None
    };

    Ok(TmuxCliCommand::NewWindow {
        target,
        name,
        print_and_format,
        cwd,
        env,
    })
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
    let mut style = None;
    let mut title = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-T" => title = Some(take_flag_value("-T", &mut iter)?),
            "-P" => style = Some(take_flag_value("-P", &mut iter).unwrap_or_default()),
            // Flags we accept but ignore
            "-e" | "-d" | "-D" | "-l" | "-M" | "-m" | "-Z" | "-U" | "-R" | "-L" => {}
            other => bail!("select-pane: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::SelectPane {
        target,
        style,
        title,
    })
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
                // Strip trailing '%' — percentage sizes will be resolved by the handler
                let val_clean = val.strip_suffix('%').unwrap_or(&val);
                width = Some(
                    val_clean
                        .parse::<u64>()
                        .map_err(|_| anyhow::anyhow!("resize-pane -x: invalid number: {val:?}"))?,
                );
            }
            "-y" => {
                let val = take_flag_value("-y", &mut iter)?;
                let val_clean = val.strip_suffix('%').unwrap_or(&val);
                height = Some(
                    val_clean
                        .parse::<u64>()
                        .map_err(|_| anyhow::anyhow!("resize-pane -y: invalid number: {val:?}"))?,
                );
            }
            // Flags we accept but ignore: -D/-U/-L/-R (relative resize directions)
            "-D" | "-U" | "-L" | "-R" | "-M" => {}
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
    let mut adjust_pane = None;
    let mut subscription = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-C" => size = Some(take_flag_value("-C", &mut iter)?),
            "-f" => flags = Some(take_flag_value("-f", &mut iter)?),
            "-A" => adjust_pane = Some(take_flag_value("-A", &mut iter)?),
            "-B" => subscription = Some(take_flag_value("-B", &mut iter)?),
            other => bail!("refresh-client: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::RefreshClient {
        size,
        flags,
        adjust_pane,
        subscription,
    })
}

fn parse_display_message(args: &[String]) -> Result<TmuxCliCommand> {
    let mut print = false;
    let mut verbose = false;
    let mut format = None;
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-p" => print = true,
            "-v" => verbose = true,
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            // Flags we accept but ignore
            "-a" | "-I" | "-N" => {}
            "-c" => {
                let _ = take_flag_value("-c", &mut iter)?;
            }
            _ => {
                // The first non-flag argument is the format string.
                // tmux display-message takes at most one positional argument.
                format = Some(arg.to_string());
            }
        }
    }

    Ok(TmuxCliCommand::DisplayMessage {
        print,
        verbose,
        format,
        target,
    })
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
    let mut window_name = None;
    let mut detached = false;
    let mut print_info = false;
    let mut format = None;
    let mut cwd = None;
    let mut env = Vec::new();

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-s" => name = Some(take_flag_value("-s", &mut iter)?),
            "-n" => window_name = Some(take_flag_value("-n", &mut iter)?),
            "-d" => detached = true,
            "-P" => print_info = true,
            "-F" => format = Some(take_flag_value("-F", &mut iter)?),
            // Flags we accept but ignore
            "-A" | "-D" | "-E" | "-X" => {}
            "-t" => {
                let _ = take_flag_value("-t", &mut iter)?;
            }
            "-x" => {
                let _ = take_flag_value("-x", &mut iter)?;
            }
            "-y" => {
                let _ = take_flag_value("-y", &mut iter)?;
            }
            "-e" => env.push(take_flag_value("-e", &mut iter)?),
            "-c" => cwd = Some(take_flag_value("-c", &mut iter)?),
            "-f" => {
                let _ = take_flag_value("-f", &mut iter)?;
            }
            other => bail!("new-session: unexpected argument: {other:?}"),
        }
    }

    let print_and_format = if print_info {
        Some(format.unwrap_or_else(|| "#{session_name}:#{window_index}.#{pane_index}".to_string()))
    } else {
        None
    };

    Ok(TmuxCliCommand::NewSession {
        name,
        window_name,
        detached,
        print_and_format,
        cwd,
        env,
    })
}

fn parse_show_options(args: &[String]) -> Result<TmuxCliCommand> {
    let mut global = false;
    let mut value_only = false;
    let mut quiet = false;
    let mut option_name = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        // Parse combined flags like -gvq, -qgv, etc.
        if arg.starts_with('-') && arg.len() > 1 && arg.chars().skip(1).all(|c| "gvqs".contains(c))
        {
            for ch in arg.chars().skip(1) {
                match ch {
                    'g' => global = true,
                    'v' => value_only = true,
                    'q' => quiet = true,
                    's' => global = true, // -s (server) is equivalent to -g (global)
                    _ => {}
                }
            }
        } else {
            option_name = Some(arg.to_string());
        }
    }

    Ok(TmuxCliCommand::ShowOptions {
        global,
        value_only,
        quiet,
        option_name,
    })
}

fn parse_show_window_options(args: &[String]) -> Result<TmuxCliCommand> {
    let mut global = false;
    let mut value_only = false;
    let mut quiet = false;
    let mut option_name = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        // Parse combined flags like -gvq, -qgv, etc.
        if arg.starts_with('-') && arg.len() > 1 && arg.chars().skip(1).all(|c| "gvq".contains(c)) {
            for ch in arg.chars().skip(1) {
                match ch {
                    'g' => global = true,
                    'v' => value_only = true,
                    'q' => quiet = true,
                    _ => {}
                }
            }
        } else {
            option_name = Some(arg.to_string());
        }
    }

    Ok(TmuxCliCommand::ShowWindowOptions {
        global,
        value_only,
        quiet,
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

// ---------------------------------------------------------------------------
// Phase 11: clipboard / buffer command parsers
// ---------------------------------------------------------------------------

fn parse_show_buffer(args: &[String]) -> Result<TmuxCliCommand> {
    let mut buffer_name = None;
    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-b" => buffer_name = Some(take_flag_value("-b", &mut iter)?),
            other => bail!("show-buffer: unexpected argument: {other:?}"),
        }
    }
    Ok(TmuxCliCommand::ShowBuffer { buffer_name })
}

fn parse_set_buffer(args: &[String]) -> Result<TmuxCliCommand> {
    let mut buffer_name = None;
    let mut append = false;
    let mut data = None;
    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-b" => buffer_name = Some(take_flag_value("-b", &mut iter)?),
            "-a" => append = true,
            "-w" | "-n" | "-t" => {
                // Accept but ignore: -w (clipboard sync), -n (rename), -t (target client)
                let _ = take_flag_value(arg, &mut iter).ok();
            }
            // Positional: the data to set. Take the rest as a single string if `--` was used,
            // or treat this as the data argument.
            "--" => {
                let rest: Vec<&str> = iter.collect();
                if !rest.is_empty() {
                    data = Some(rest.join(" "));
                }
                break;
            }
            _ => {
                // First non-flag argument is the data.
                data = Some(arg.to_string());
                // Collect any remaining args as part of data (shouldn't happen
                // with shell_words splitting, but be safe).
                let rest: Vec<&str> = iter.collect();
                if !rest.is_empty() {
                    let mut d = data.unwrap();
                    for r in rest {
                        d.push(' ');
                        d.push_str(r);
                    }
                    data = Some(d);
                }
                break;
            }
        }
    }
    Ok(TmuxCliCommand::SetBuffer {
        buffer_name,
        data,
        append,
    })
}

fn parse_delete_buffer(args: &[String]) -> Result<TmuxCliCommand> {
    let mut buffer_name = None;
    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-b" => buffer_name = Some(take_flag_value("-b", &mut iter)?),
            other => bail!("delete-buffer: unexpected argument: {other:?}"),
        }
    }
    Ok(TmuxCliCommand::DeleteBuffer { buffer_name })
}

fn parse_list_buffers(args: &[String]) -> Result<TmuxCliCommand> {
    let mut format = None;
    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-F" => format = Some(take_flag_value("-F", &mut iter)?),
            "-f" | "-O" => {
                // Accept but ignore: -f (filter), -O (sort order)
                let _ = take_flag_value(arg, &mut iter).ok();
            }
            "-r" => {} // Accept but ignore reverse flag
            other => bail!("list-buffers: unexpected argument: {other:?}"),
        }
    }
    Ok(TmuxCliCommand::ListBuffers { format })
}

fn parse_paste_buffer(args: &[String]) -> Result<TmuxCliCommand> {
    let mut buffer_name = None;
    let mut target = None;
    let mut delete_after = false;
    let mut bracketed = false;
    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-b" => buffer_name = Some(take_flag_value("-b", &mut iter)?),
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-d" => delete_after = true,
            "-p" => bracketed = true,
            "-r" | "-s" => {
                // Accept but ignore: -r (LF separator), -s (custom separator)
                if arg == "-s" {
                    let _ = take_flag_value("-s", &mut iter).ok();
                }
            }
            other => bail!("paste-buffer: unexpected argument: {other:?}"),
        }
    }
    Ok(TmuxCliCommand::PasteBuffer {
        buffer_name,
        target,
        delete_after,
        bracketed,
    })
}

fn parse_move_pane(args: &[String]) -> Result<TmuxCliCommand> {
    let mut src = None;
    let mut dst = None;
    let mut horizontal = false;
    let mut before = false;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-s" => src = Some(take_flag_value("-s", &mut iter)?),
            "-t" => dst = Some(take_flag_value("-t", &mut iter)?),
            "-h" => horizontal = true,
            "-v" => {} // vertical is default, no-op
            "-b" => before = true,
            "-d" | "-f" => {} // accept but ignore: -d (don't focus), -f (full size)
            "-l" | "-p" => {
                // Accept but ignore: -l size, -p percentage
                let _ = take_flag_value(arg, &mut iter).ok();
            }
            other => bail!("move-pane: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::MovePane {
        src,
        dst,
        horizontal,
        before,
    })
}

fn parse_move_window(args: &[String]) -> Result<TmuxCliCommand> {
    let mut src = None;
    let mut dst = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-s" => src = Some(take_flag_value("-s", &mut iter)?),
            "-t" => dst = Some(take_flag_value("-t", &mut iter)?),
            "-a" | "-b" | "-d" | "-k" | "-r" => {} // accept but ignore
            other => bail!("move-window: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::MoveWindow { src, dst })
}

fn parse_copy_mode(args: &[String]) -> Result<TmuxCliCommand> {
    let mut quit = false;
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-q" => quit = true,
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-s" => {
                let _ = take_flag_value("-s", &mut iter)?;
            }
            "-d" | "-e" | "-H" | "-M" | "-S" | "-u" => {} // accept but ignore
            other => bail!("copy-mode: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::CopyMode { quit, target })
}

// Phase 13: Claude Code agent teams compatibility — new parsers

fn parse_set_option(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;
    let mut option_name = None;
    let mut value = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            // Scope flags — accept but ignore (we always treat as no-op)
            "-g" | "-s" | "-w" | "-p" | "-q" | "-o" | "-u" | "-U" | "-a" | "-F" => {}
            _ => {
                // First positional = option name, second = value
                if option_name.is_none() {
                    option_name = Some(arg.to_string());
                } else if value.is_none() {
                    value = Some(arg.to_string());
                }
                // Extra positionals are silently ignored
            }
        }
    }

    Ok(TmuxCliCommand::SetOption {
        target,
        option_name,
        value,
    })
}

fn parse_select_layout(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;
    let mut layout_name = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            // Flags we accept but ignore
            "-E" | "-n" | "-o" | "-p" => {}
            _ => {
                // Positional argument = layout name
                if layout_name.is_none() {
                    layout_name = Some(arg.to_string());
                }
            }
        }
    }

    Ok(TmuxCliCommand::SelectLayout {
        target,
        layout_name,
    })
}

fn parse_break_pane(args: &[String]) -> Result<TmuxCliCommand> {
    let mut detach = false;
    let mut source = None;
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-d" => detach = true,
            "-s" => source = Some(take_flag_value("-s", &mut iter)?),
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            // Flags we accept but ignore
            "-P" => {}
            "-F" => {
                let _ = take_flag_value("-F", &mut iter)?;
            }
            "-n" => {
                let _ = take_flag_value("-n", &mut iter)?;
            }
            other => bail!("break-pane: unexpected argument: {other:?}"),
        }
    }

    Ok(TmuxCliCommand::BreakPane {
        detach,
        source,
        target,
    })
}

// ---------------------------------------------------------------------------
// Phase 17: missing commands for cleanup & orchestration
// ---------------------------------------------------------------------------

fn parse_wait_for(args: &[String]) -> Result<TmuxCliCommand> {
    let mut signal = false;
    let mut channel = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-S" => signal = true,
            "-L" | "-U" => {
                // Lock/unlock — accept but treat like signal
            }
            _ => {
                channel = Some(arg.to_string());
            }
        }
    }

    let channel = channel.unwrap_or_default();
    Ok(TmuxCliCommand::WaitFor { signal, channel })
}

fn parse_pipe_pane(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;
    let mut command = None;
    let mut output = false;
    let mut input = false;
    let mut toggle = false;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-O" => output = true,
            "-I" => input = true,
            "-o" => toggle = true,
            _ => {
                command = Some(arg.to_string());
            }
        }
    }

    // Default: if neither -I nor -O specified, output mode is implied
    if !input && !output {
        output = true;
    }

    Ok(TmuxCliCommand::PipePane {
        target,
        command,
        output,
        input,
        toggle,
    })
}

fn parse_display_popup(args: &[String]) -> Result<TmuxCliCommand> {
    let mut target = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            // Flags that take a value (display-popup + display-menu union)
            "-b" | "-c" | "-d" | "-e" | "-h" | "-w" | "-x" | "-y" | "-s" | "-S" | "-T" | "-H" => {
                let _ = take_flag_value(arg, &mut iter)?;
            }
            // Boolean flags (no value)
            "-B" | "-C" | "-E" | "-k" | "-M" | "-N" | "-O" => {}
            _ => {
                // Remaining args are the popup/menu command — ignore
            }
        }
    }

    Ok(TmuxCliCommand::DisplayPopup { target })
}

fn parse_run_shell(args: &[String]) -> Result<TmuxCliCommand> {
    let mut background = false;
    let mut target = None;
    let mut command = None;
    let mut delay = None;

    let strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut iter = strs.iter().copied();
    while let Some(arg) = iter.next() {
        match arg {
            "-b" => background = true,
            "-C" => {} // tmux command mode — ignore
            "-t" => target = Some(take_flag_value("-t", &mut iter)?),
            "-d" => {
                delay = Some(take_flag_value("-d", &mut iter)?);
            }
            _ => {
                command = Some(arg.to_string());
            }
        }
    }

    Ok(TmuxCliCommand::RunShell {
        background,
        target,
        command,
        delay,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to reduce boilerplate in assertions.
    fn parse(s: &str) -> TmuxCliCommand {
        parse_command(s).unwrap_or_else(|e| panic!("parse_command({:?}) failed: {}", s, e))
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
                print_and_format: None,
                cwd: None,
                env: vec![],
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
                print_and_format: None,
                cwd: None,
                env: vec![],
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
                print_and_format: None,
                cwd: None,
                env: vec![],
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
                print_and_format: None,
                cwd: None,
                env: vec![],
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
                print_and_format: None,
                cwd: None,
                env: vec![],
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
                print_and_format: None,
                cwd: None,
                env: vec![],
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
                print_and_format: None,
                cwd: None,
                env: vec![],
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
                style: None,
                title: None,
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
                adjust_pane: None,
                subscription: None,
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
                adjust_pane: None,
                subscription: None,
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
                adjust_pane: None,
                subscription: None,
            }
        );
    }

    #[test]
    fn refresh_client_pause_after_flag() {
        assert_eq!(
            parse("refresh-client -f pause-after=5,wait-exit"),
            TmuxCliCommand::RefreshClient {
                size: None,
                flags: Some("pause-after=5,wait-exit".into()),
                adjust_pane: None,
                subscription: None,
            }
        );
    }

    #[test]
    fn refresh_client_adjust_pane() {
        assert_eq!(
            parse("refresh-client -A %0:continue"),
            TmuxCliCommand::RefreshClient {
                size: None,
                flags: None,
                adjust_pane: Some("%0:continue".into()),
                subscription: None,
            }
        );
    }

    #[test]
    fn refresh_client_disable_pause() {
        assert_eq!(
            parse("refresh-client -f !pause-after"),
            TmuxCliCommand::RefreshClient {
                size: None,
                flags: Some("!pause-after".into()),
                adjust_pane: None,
                subscription: None,
            }
        );
    }

    #[test]
    fn refresh_client_subscription() {
        assert_eq!(
            parse("refresh-client -B my-sub:%0:#{pane_id}"),
            TmuxCliCommand::RefreshClient {
                size: None,
                flags: None,
                adjust_pane: None,
                subscription: Some("my-sub:%0:#{pane_id}".into()),
            }
        );
    }

    #[test]
    fn refresh_client_unsubscribe() {
        assert_eq!(
            parse("refresh-client -B my-sub"),
            TmuxCliCommand::RefreshClient {
                size: None,
                flags: None,
                adjust_pane: None,
                subscription: Some("my-sub".into()),
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
                verbose: false,
                format: Some("#{session_id}".into()),
                target: None,
            }
        );
    }

    #[test]
    fn display_message_no_args() {
        assert_eq!(
            parse("display-message"),
            TmuxCliCommand::DisplayMessage {
                print: false,
                verbose: false,
                format: None,
                target: None,
            }
        );
    }

    #[test]
    fn display_message_format_only() {
        assert_eq!(
            parse("display-message '#{window_id}'"),
            TmuxCliCommand::DisplayMessage {
                print: false,
                verbose: false,
                format: Some("#{window_id}".into()),
                target: None,
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
                window_name: None,
                detached: false,
                print_and_format: None,
                cwd: None,
                env: vec![],
            }
        );
    }

    #[test]
    fn new_session_no_args() {
        assert_eq!(
            parse("new-session"),
            TmuxCliCommand::NewSession {
                name: None,
                window_name: None,
                detached: false,
                print_and_format: None,
                cwd: None,
                env: vec![],
            }
        );
    }

    #[test]
    fn new_session_alias_new() {
        assert_eq!(
            parse("new -s dev"),
            TmuxCliCommand::NewSession {
                name: Some("dev".into()),
                window_name: None,
                detached: false,
                print_and_format: None,
                cwd: None,
                env: vec![],
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
                quiet: false,
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
                quiet: false,
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
                quiet: false,
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
                quiet: false,
                option_name: Some("set-clipboard".into()),
            }
        );
    }

    #[test]
    fn show_options_quiet_flag() {
        assert_eq!(
            parse("show-options -gqv nonexistent"),
            TmuxCliCommand::ShowOptions {
                global: true,
                value_only: true,
                quiet: true,
                option_name: Some("nonexistent".into()),
            }
        );
    }

    #[test]
    fn show_options_server_flag() {
        // -s is equivalent to -g (server scope = global)
        assert_eq!(
            parse("show-options -sv default-terminal"),
            TmuxCliCommand::ShowOptions {
                global: true,
                value_only: true,
                quiet: false,
                option_name: Some("default-terminal".into()),
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
                quiet: false,
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
                quiet: false,
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
                quiet: false,
                option_name: None,
            }
        );
    }

    #[test]
    fn show_window_options_quiet_flag() {
        assert_eq!(
            parse("showw -gqv nonexistent"),
            TmuxCliCommand::ShowWindowOptions {
                global: true,
                value_only: true,
                quiet: true,
                option_name: Some("nonexistent".into()),
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

    // ---------------------------------------------------------------
    // move-pane / join-pane
    // ---------------------------------------------------------------

    #[test]
    fn move_pane_basic() {
        assert_eq!(
            parse("move-pane -s %0 -t %1"),
            TmuxCliCommand::MovePane {
                src: Some("%0".into()),
                dst: Some("%1".into()),
                horizontal: false,
                before: false,
            }
        );
    }

    #[test]
    fn move_pane_horizontal_before() {
        assert_eq!(
            parse("move-pane -s %0 -t %1 -h -b"),
            TmuxCliCommand::MovePane {
                src: Some("%0".into()),
                dst: Some("%1".into()),
                horizontal: true,
                before: true,
            }
        );
    }

    #[test]
    fn join_pane_alias() {
        assert_eq!(
            parse("join-pane -s %3 -t %5 -v"),
            TmuxCliCommand::MovePane {
                src: Some("%3".into()),
                dst: Some("%5".into()),
                horizontal: false,
                before: false,
            }
        );
    }

    #[test]
    fn movep_alias() {
        assert_eq!(
            parse("movep -s %1 -t %2 -h"),
            TmuxCliCommand::MovePane {
                src: Some("%1".into()),
                dst: Some("%2".into()),
                horizontal: true,
                before: false,
            }
        );
    }

    #[test]
    fn move_pane_iterm2_style() {
        // iTerm2 sends: move-pane -s "%0" -t "%1" -h
        assert_eq!(
            parse("move-pane -s \"%0\" -t \"%1\" -h"),
            TmuxCliCommand::MovePane {
                src: Some("%0".into()),
                dst: Some("%1".into()),
                horizontal: true,
                before: false,
            }
        );
    }

    // ---------------------------------------------------------------
    // move-window
    // ---------------------------------------------------------------

    #[test]
    fn move_window_basic() {
        assert_eq!(
            parse("move-window -s $0:@1 -t $1:+"),
            TmuxCliCommand::MoveWindow {
                src: Some("$0:@1".into()),
                dst: Some("$1:+".into()),
            }
        );
    }

    #[test]
    fn movew_alias() {
        assert_eq!(
            parse("movew -s @0 -t @1"),
            TmuxCliCommand::MoveWindow {
                src: Some("@0".into()),
                dst: Some("@1".into()),
            }
        );
    }

    #[test]
    fn move_window_with_ignored_flags() {
        assert_eq!(
            parse("move-window -s @0 -t $1:+ -a -d -k -r"),
            TmuxCliCommand::MoveWindow {
                src: Some("@0".into()),
                dst: Some("$1:+".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // copy-mode
    // ---------------------------------------------------------------

    #[test]
    fn copy_mode_quit() {
        assert_eq!(
            parse("copy-mode -q"),
            TmuxCliCommand::CopyMode {
                quit: true,
                target: None,
            }
        );
    }

    #[test]
    fn copy_mode_with_target() {
        assert_eq!(
            parse("copy-mode -t %0"),
            TmuxCliCommand::CopyMode {
                quit: false,
                target: Some("%0".into()),
            }
        );
    }

    #[test]
    fn copy_mode_quit_with_target() {
        assert_eq!(
            parse("copy-mode -q -t %3"),
            TmuxCliCommand::CopyMode {
                quit: true,
                target: Some("%3".into()),
            }
        );
    }

    #[test]
    fn copy_mode_all_flags() {
        // All valid tmux copy-mode flags: -d, -e, -H, -M, -q, -S, -s (value), -t (value), -u
        assert_eq!(
            parse("copy-mode -d -s %3 -S -e -H -M -u -q -t %5"),
            TmuxCliCommand::CopyMode {
                quit: true,
                target: Some("%5".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // Phase 13: Claude Code agent teams compatibility
    // ---------------------------------------------------------------

    #[test]
    fn split_window_print_format() {
        assert_eq!(
            parse("split-window -t %5 -h -l 70% -P -F '#{pane_id}'"),
            TmuxCliCommand::SplitWindow {
                horizontal: true,
                vertical: false,
                target: Some("%5".into()),
                size: Some("70%".into()),
                print_and_format: Some("#{pane_id}".into()),
                cwd: None,
                env: vec![],
            }
        );
    }

    #[test]
    fn split_window_print_default_format() {
        assert_eq!(
            parse("split-window -h -P"),
            TmuxCliCommand::SplitWindow {
                horizontal: true,
                vertical: false,
                target: None,
                size: None,
                print_and_format: Some("#{session_name}:#{window_index}.#{pane_index}".into()),
                cwd: None,
                env: vec![],
            }
        );
    }

    #[test]
    fn split_window_extra_flags() {
        // Claude Code sends -d, -b, -f, -c, -e flags; -c/-e are now wired through
        assert_eq!(
            parse("split-window -h -d -c /tmp -e FOO=bar"),
            TmuxCliCommand::SplitWindow {
                horizontal: true,
                vertical: false,
                target: None,
                size: None,
                print_and_format: None,
                cwd: Some("/tmp".into()),
                env: vec!["FOO=bar".into()],
            }
        );
    }

    #[test]
    fn new_window_print_format() {
        assert_eq!(
            parse("new-window -t main -n editor -P -F '#{pane_id}'"),
            TmuxCliCommand::NewWindow {
                target: Some("main".into()),
                name: Some("editor".into()),
                print_and_format: Some("#{pane_id}".into()),
                cwd: None,
                env: vec![],
            }
        );
    }

    #[test]
    fn new_session_full_claude_code() {
        assert_eq!(
            parse("new-session -d -s myswarm -n main -P -F '#{pane_id}'"),
            TmuxCliCommand::NewSession {
                name: Some("myswarm".into()),
                window_name: Some("main".into()),
                detached: true,
                print_and_format: Some("#{pane_id}".into()),
                cwd: None,
                env: vec![],
            }
        );
    }

    #[test]
    fn select_pane_with_style() {
        assert_eq!(
            parse("select-pane -t %5 -P bg=default,fg=blue"),
            TmuxCliCommand::SelectPane {
                target: Some("%5".into()),
                style: Some("bg=default,fg=blue".into()),
                title: None,
            }
        );
    }

    #[test]
    fn select_pane_with_title() {
        assert_eq!(
            parse("select-pane -t %5 -T myagent"),
            TmuxCliCommand::SelectPane {
                target: Some("%5".into()),
                style: None,
                title: Some("myagent".into()),
            }
        );
    }

    #[test]
    fn select_pane_ignored_flags() {
        // -e, -d, -D, -l, -M, -m, -Z, -U, -R, -L should all be accepted
        assert_eq!(
            parse("select-pane -t %5 -e -Z"),
            TmuxCliCommand::SelectPane {
                target: Some("%5".into()),
                style: None,
                title: None,
            }
        );
    }

    #[test]
    fn display_message_with_target() {
        assert_eq!(
            parse("display-message -t %5 -p '#{session_name}:#{window_index}'"),
            TmuxCliCommand::DisplayMessage {
                print: true,
                verbose: false,
                format: Some("#{session_name}:#{window_index}".into()),
                target: Some("%5".into()),
            }
        );
    }

    #[test]
    fn set_option_pane_style() {
        assert_eq!(
            parse("set-option -p -t %5 pane-border-style fg=blue"),
            TmuxCliCommand::SetOption {
                target: Some("%5".into()),
                option_name: Some("pane-border-style".into()),
                value: Some("fg=blue".into()),
            }
        );
    }

    #[test]
    fn set_option_alias_set() {
        assert_eq!(
            parse("set -g status off"),
            TmuxCliCommand::SetOption {
                target: None,
                option_name: Some("status".into()),
                value: Some("off".into()),
            }
        );
    }

    #[test]
    fn select_layout_main_vertical() {
        assert_eq!(
            parse("select-layout -t @0 main-vertical"),
            TmuxCliCommand::SelectLayout {
                target: Some("@0".into()),
                layout_name: Some("main-vertical".into()),
            }
        );
    }

    #[test]
    fn select_layout_tiled() {
        assert_eq!(
            parse("select-layout tiled"),
            TmuxCliCommand::SelectLayout {
                target: None,
                layout_name: Some("tiled".into()),
            }
        );
    }

    #[test]
    fn select_layout_alias_selectl() {
        assert_eq!(
            parse("selectl -t @0 even-horizontal"),
            TmuxCliCommand::SelectLayout {
                target: Some("@0".into()),
                layout_name: Some("even-horizontal".into()),
            }
        );
    }

    #[test]
    fn break_pane_claude_code() {
        assert_eq!(
            parse("break-pane -d -s %5 -t main:"),
            TmuxCliCommand::BreakPane {
                detach: true,
                source: Some("%5".into()),
                target: Some("main:".into()),
            }
        );
    }

    #[test]
    fn break_pane_alias_breakp() {
        assert_eq!(
            parse("breakp -d -s %3"),
            TmuxCliCommand::BreakPane {
                detach: true,
                source: Some("%3".into()),
                target: None,
            }
        );
    }

    #[test]
    fn resize_pane_percentage() {
        // Claude Code sends percentage values — they should parse correctly
        assert_eq!(
            parse("resize-pane -t %5 -x 30%"),
            TmuxCliCommand::ResizePane {
                target: Some("%5".into()),
                width: Some(30),
                height: None,
                zoom: false,
            }
        );
    }

    #[test]
    fn resize_pane_percentage_y() {
        assert_eq!(
            parse("resize-pane -t %5 -y 50%"),
            TmuxCliCommand::ResizePane {
                target: Some("%5".into()),
                width: None,
                height: Some(50),
                zoom: false,
            }
        );
    }

    // ---------------------------------------------------------------
    // Phase 16: command alias tests
    // ---------------------------------------------------------------

    #[test]
    fn alias_ls_for_list_sessions() {
        assert_eq!(parse("ls"), TmuxCliCommand::ListSessions { format: None });
    }

    #[test]
    fn alias_ls_with_format() {
        assert_eq!(
            parse("ls -F '#{session_name}'"),
            TmuxCliCommand::ListSessions {
                format: Some("#{session_name}".into()),
            }
        );
    }

    #[test]
    fn alias_lsp_for_list_panes() {
        assert_eq!(
            parse("lsp"),
            TmuxCliCommand::ListPanes {
                all: false,
                session: false,
                format: None,
                target: None,
            }
        );
    }

    #[test]
    fn alias_lsw_for_list_windows() {
        assert_eq!(
            parse("lsw"),
            TmuxCliCommand::ListWindows {
                all: false,
                format: None,
                target: None,
            }
        );
    }

    #[test]
    fn alias_splitw_for_split_window() {
        assert_eq!(
            parse("splitw -h"),
            TmuxCliCommand::SplitWindow {
                horizontal: true,
                vertical: false,
                target: None,
                size: None,
                print_and_format: None,
                cwd: None,
                env: vec![],
            }
        );
    }

    #[test]
    fn alias_neww_for_new_window() {
        assert_eq!(
            parse("neww -n test"),
            TmuxCliCommand::NewWindow {
                target: None,
                name: Some("test".into()),
                print_and_format: None,
                cwd: None,
                env: vec![],
            }
        );
    }

    #[test]
    fn alias_selectw_for_select_window() {
        assert_eq!(
            parse("selectw -t @1"),
            TmuxCliCommand::SelectWindow {
                target: Some("@1".into()),
            }
        );
    }

    #[test]
    fn alias_selectp_for_select_pane() {
        assert_eq!(
            parse("selectp -t %0"),
            TmuxCliCommand::SelectPane {
                target: Some("%0".into()),
                style: None,
                title: None,
            }
        );
    }

    #[test]
    fn alias_killp_for_kill_pane() {
        assert_eq!(
            parse("killp -t %1"),
            TmuxCliCommand::KillPane {
                target: Some("%1".into()),
            }
        );
    }

    #[test]
    fn alias_capturep_for_capture_pane() {
        assert_eq!(
            parse("capturep -p -t %0"),
            TmuxCliCommand::CapturePane {
                print: true,
                target: Some("%0".into()),
                escape: false,
                octal_escape: false,
                start_line: None,
                end_line: None,
            }
        );
    }

    #[test]
    fn alias_send_for_send_keys() {
        assert_eq!(
            parse("send -t %0 -l hello"),
            TmuxCliCommand::SendKeys {
                target: Some("%0".into()),
                literal: true,
                hex: false,
                keys: vec!["hello".into()],
            }
        );
    }

    #[test]
    fn alias_display_for_display_message() {
        assert_eq!(
            parse("display -p '#{pane_id}'"),
            TmuxCliCommand::DisplayMessage {
                print: true,
                verbose: false,
                format: Some("#{pane_id}".into()),
                target: None,
            }
        );
    }

    #[test]
    fn alias_has_for_has_session() {
        assert_eq!(
            parse("has -t main"),
            TmuxCliCommand::HasSession {
                target: Some("main".into()),
            }
        );
    }

    #[test]
    fn alias_kills_for_kill_session() {
        assert_eq!(
            parse("kills -t old"),
            TmuxCliCommand::KillSession {
                target: Some("old".into()),
            }
        );
    }

    #[test]
    fn alias_resizew_for_resize_window() {
        assert_eq!(
            parse("resizew -t @0 -x 120 -y 40"),
            TmuxCliCommand::ResizeWindow {
                target: Some("@0".into()),
                width: Some(120),
                height: Some(40),
            }
        );
    }

    #[test]
    fn alias_refresh_for_refresh_client() {
        // "refresh" is alias for "refresh-client"
        assert_eq!(
            parse("refresh -C 200x50"),
            TmuxCliCommand::RefreshClient {
                size: Some("200x50".into()),
                flags: None,
                adjust_pane: None,
                subscription: None,
            }
        );
    }

    // ---------------------------------------------------------------
    // Phase 17: parser tests for missing commands
    // ---------------------------------------------------------------

    #[test]
    fn phase17_kill_server() {
        assert_eq!(parse("kill-server"), TmuxCliCommand::KillServer);
    }

    #[test]
    fn phase17_wait_for_signal() {
        assert_eq!(
            parse("wait-for -S mychannel"),
            TmuxCliCommand::WaitFor {
                signal: true,
                channel: "mychannel".into(),
            }
        );
    }

    #[test]
    fn phase17_wait_for_lock() {
        assert_eq!(
            parse("wait-for -L lockname"),
            TmuxCliCommand::WaitFor {
                signal: false,
                channel: "lockname".into(),
            }
        );
    }

    #[test]
    fn phase17_wait_alias() {
        assert_eq!(
            parse("wait -S done"),
            TmuxCliCommand::WaitFor {
                signal: true,
                channel: "done".into(),
            }
        );
    }

    #[test]
    fn phase17_pipe_pane_with_command() {
        assert_eq!(
            parse("pipe-pane -t %3 \"cat >> /tmp/log\""),
            TmuxCliCommand::PipePane {
                target: Some("%3".into()),
                command: Some("cat >> /tmp/log".into()),
                output: true,
                input: false,
                toggle: false,
            }
        );
    }

    #[test]
    fn phase17_pipe_pane_no_args() {
        assert_eq!(
            parse("pipe-pane"),
            TmuxCliCommand::PipePane {
                target: None,
                command: None,
                output: true,
                input: false,
                toggle: false,
            }
        );
    }

    #[test]
    fn phase17_pipep_alias() {
        assert_eq!(
            parse("pipep -t %1"),
            TmuxCliCommand::PipePane {
                target: Some("%1".into()),
                command: None,
                output: true,
                input: false,
                toggle: false,
            }
        );
    }

    #[test]
    fn phase17_pipe_pane_input_output() {
        assert_eq!(
            parse("pipe-pane -I -O -t %2 \"tee /tmp/log\""),
            TmuxCliCommand::PipePane {
                target: Some("%2".into()),
                command: Some("tee /tmp/log".into()),
                output: true,
                input: true,
                toggle: false,
            }
        );
    }

    #[test]
    fn phase17_pipe_pane_toggle() {
        assert_eq!(
            parse("pipe-pane -o -t %0 \"cat >> /tmp/log\""),
            TmuxCliCommand::PipePane {
                target: Some("%0".into()),
                command: Some("cat >> /tmp/log".into()),
                output: true,
                input: false,
                toggle: true,
            }
        );
    }

    #[test]
    fn phase17_display_popup_basic() {
        assert_eq!(
            parse("display-popup -t %0"),
            TmuxCliCommand::DisplayPopup {
                target: Some("%0".into()),
            }
        );
    }

    #[test]
    fn phase17_popup_alias() {
        assert_eq!(
            parse("popup -E ls"),
            TmuxCliCommand::DisplayPopup { target: None }
        );
    }

    #[test]
    fn phase17_display_popup_with_flags() {
        assert_eq!(
            parse("display-popup -w 80 -h 24 -d /tmp -t %2 echo hello"),
            TmuxCliCommand::DisplayPopup {
                target: Some("%2".into()),
            }
        );
    }

    #[test]
    fn phase17_display_menu_basic() {
        assert_eq!(
            parse("display-menu -t %1 -T title -x 10 -y 5"),
            TmuxCliCommand::DisplayPopup {
                target: Some("%1".into()),
            }
        );
    }

    #[test]
    fn phase17_menu_alias() {
        assert_eq!(
            parse("menu -T test -x 0 -y 0"),
            TmuxCliCommand::DisplayPopup { target: None }
        );
    }

    #[test]
    fn phase17_display_popup_boolean_b_flag() {
        // -B should be boolean (no value), not consume the next arg
        assert_eq!(
            parse("display-popup -B -t %0"),
            TmuxCliCommand::DisplayPopup {
                target: Some("%0".into()),
            }
        );
    }

    #[test]
    fn phase17_run_shell_basic() {
        assert_eq!(
            parse("run-shell \"echo hello\""),
            TmuxCliCommand::RunShell {
                background: false,
                target: None,
                command: Some("echo hello".into()),
                delay: None,
            }
        );
    }

    #[test]
    fn phase17_run_shell_background() {
        assert_eq!(
            parse("run-shell -b \"sleep 1\""),
            TmuxCliCommand::RunShell {
                background: true,
                target: None,
                command: Some("sleep 1".into()),
                delay: None,
            }
        );
    }

    #[test]
    fn phase17_run_shell_with_target() {
        assert_eq!(
            parse("run-shell -t %5 \"echo hi\""),
            TmuxCliCommand::RunShell {
                background: false,
                target: Some("%5".into()),
                command: Some("echo hi".into()),
                delay: None,
            }
        );
    }

    #[test]
    fn phase17_run_alias() {
        assert_eq!(
            parse("run \"date\""),
            TmuxCliCommand::RunShell {
                background: false,
                target: None,
                command: Some("date".into()),
                delay: None,
            }
        );
    }

    #[test]
    fn phase17_run_shell_no_command() {
        assert_eq!(
            parse("run-shell"),
            TmuxCliCommand::RunShell {
                background: false,
                target: None,
                command: None,
                delay: None,
            }
        );
    }

    #[test]
    fn phase17_run_shell_with_delay() {
        assert_eq!(
            parse("run-shell -d 2.5 \"echo delayed\""),
            TmuxCliCommand::RunShell {
                background: false,
                target: None,
                command: Some("echo delayed".into()),
                delay: Some("2.5".into()),
            }
        );
    }

    // ---------------------------------------------------------------
    // Phase 18: robustness & edge case tests
    // ---------------------------------------------------------------

    // 18.2: send-keys with special characters

    #[test]
    fn phase18_send_keys_literal_with_special_chars() {
        // Quotes, backslashes, dollar signs in literal mode
        // shell_words::split unquotes: "echo $HOME" -> echo $HOME
        assert_eq!(
            parse(r#"send-keys -t %3 -l "echo $HOME done""#),
            TmuxCliCommand::SendKeys {
                target: Some("%3".into()),
                literal: true,
                hex: false,
                keys: vec!["echo $HOME done".into()],
            }
        );
    }

    #[test]
    fn phase18_send_keys_env_syntax() {
        // env VAR=value command syntax used by Claude Code
        assert_eq!(
            parse("send-keys -t %5 \"cd /path && env CLAUDECODE=1 claude --agent\" C-m"),
            TmuxCliCommand::SendKeys {
                target: Some("%5".into()),
                literal: false,
                hex: false,
                keys: vec![
                    "cd /path && env CLAUDECODE=1 claude --agent".into(),
                    "C-m".into(),
                ],
            }
        );
    }

    #[test]
    fn phase18_send_keys_named_control_keys() {
        // C-c, C-d, C-z named keys
        assert_eq!(
            parse("send-keys -t %0 C-c"),
            TmuxCliCommand::SendKeys {
                target: Some("%0".into()),
                literal: false,
                hex: false,
                keys: vec!["C-c".into()],
            }
        );
    }

    #[test]
    fn phase18_send_keys_multiple_named_keys() {
        assert_eq!(
            parse("send-keys -t %1 Escape \"[A\""),
            TmuxCliCommand::SendKeys {
                target: Some("%1".into()),
                literal: false,
                hex: false,
                keys: vec!["Escape".into(), "[A".into()],
            }
        );
    }

    // 18.3: unknown command returns parse error

    #[test]
    fn phase18_unknown_command_error() {
        let result = parse_command("frobnicate --everything");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown"),
            "error should mention 'unknown': {}",
            err
        );
    }

    #[test]
    fn phase18_empty_command_error() {
        assert!(parse_command("").is_err());
        assert!(parse_command("   ").is_err());
    }

    // 18.4: -c flag stored in spawn commands

    #[test]
    fn phase18_split_window_cwd() {
        assert_eq!(
            parse("split-window -h -c /home/user/project"),
            TmuxCliCommand::SplitWindow {
                horizontal: true,
                vertical: false,
                target: None,
                size: None,
                print_and_format: None,
                cwd: Some("/home/user/project".into()),
                env: vec![],
            }
        );
    }

    #[test]
    fn phase18_new_window_cwd() {
        assert_eq!(
            parse("new-window -c /tmp -n build"),
            TmuxCliCommand::NewWindow {
                target: None,
                name: Some("build".into()),
                print_and_format: None,
                cwd: Some("/tmp".into()),
                env: vec![],
            }
        );
    }

    #[test]
    fn phase18_new_session_cwd() {
        assert_eq!(
            parse("new-session -s work -c /home/user"),
            TmuxCliCommand::NewSession {
                name: Some("work".into()),
                window_name: None,
                detached: false,
                print_and_format: None,
                cwd: Some("/home/user".into()),
                env: vec![],
            }
        );
    }

    // 18.5: -e flag stored in spawn commands

    #[test]
    fn phase18_split_window_env() {
        assert_eq!(
            parse("split-window -h -e CLAUDECODE=1 -e TERM=xterm"),
            TmuxCliCommand::SplitWindow {
                horizontal: true,
                vertical: false,
                target: None,
                size: None,
                print_and_format: None,
                cwd: None,
                env: vec!["CLAUDECODE=1".into(), "TERM=xterm".into()],
            }
        );
    }

    #[test]
    fn phase18_new_window_env() {
        assert_eq!(
            parse("new-window -e FOO=bar"),
            TmuxCliCommand::NewWindow {
                target: None,
                name: None,
                print_and_format: None,
                cwd: None,
                env: vec!["FOO=bar".into()],
            }
        );
    }

    #[test]
    fn phase18_split_window_cwd_and_env() {
        // Combined -c and -e flags
        assert_eq!(
            parse("split-window -h -c /tmp -e KEY=val -P -F '#{pane_id}'"),
            TmuxCliCommand::SplitWindow {
                horizontal: true,
                vertical: false,
                target: None,
                size: None,
                print_and_format: Some("#{pane_id}".into()),
                cwd: Some("/tmp".into()),
                env: vec!["KEY=val".into()],
            }
        );
    }

    // ---------------------------------------------------------------
    // Phase 19: diagnostic & debugging tests
    // ---------------------------------------------------------------

    #[test]
    fn phase19_server_info() {
        assert_eq!(parse("server-info"), TmuxCliCommand::ServerInfo);
    }

    #[test]
    fn phase19_info_alias() {
        assert_eq!(parse("info"), TmuxCliCommand::ServerInfo);
    }

    #[test]
    fn phase19_display_message_verbose() {
        assert_eq!(
            parse("display-message -v -p '#{pane_id}'"),
            TmuxCliCommand::DisplayMessage {
                print: true,
                verbose: true,
                format: Some("#{pane_id}".into()),
                target: None,
            }
        );
    }

    #[test]
    fn phase19_display_message_no_verbose() {
        assert_eq!(
            parse("display-message -p '#{session_name}'"),
            TmuxCliCommand::DisplayMessage {
                print: true,
                verbose: false,
                format: Some("#{session_name}".into()),
                target: None,
            }
        );
    }
}
