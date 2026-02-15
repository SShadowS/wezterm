//! Minimal `env` shim for Windows.
//!
//! Emulates the Unix `env` command so that commands like
//! `env KEY=VAL command args...` work inside WezTerm's tmux compat layer
//! on Windows where no native `env` utility exists.

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut clear_env = false;
    let mut unset_vars: Vec<String> = Vec::new();
    let mut set_vars: Vec<(String, String)> = Vec::new();
    let mut cmd_start: Option<usize> = None;
    let mut i = 0;

    // Parse options first
    while i < args.len() {
        let arg = &args[i];

        if arg == "--" {
            // Stop option processing; next arg is the command
            i += 1;
            if i < args.len() {
                cmd_start = Some(i);
            }
            break;
        }

        if arg == "-i" || arg == "-" {
            clear_env = true;
            i += 1;
            continue;
        }

        if arg == "-u" {
            i += 1;
            if i < args.len() {
                unset_vars.push(args[i].clone());
                i += 1;
            } else {
                eprintln!("env: option '-u' requires an argument");
                return ExitCode::from(125);
            }
            continue;
        }

        // Check for -uVAR (combined form)
        if arg.starts_with("-u") {
            unset_vars.push(arg[2..].to_string());
            i += 1;
            continue;
        }

        // If it looks like an unknown option, error
        if arg.starts_with('-') && !arg.contains('=') {
            eprintln!("env: invalid option '{}'", arg);
            return ExitCode::from(125);
        }

        // Not an option -- fall through to KEY=VAL / command parsing
        break;
    }

    // Parse KEY=VAL pairs and find the command
    if cmd_start.is_none() {
        while i < args.len() {
            if let Some(eq) = args[i].find('=') {
                let key = &args[i][..eq];
                let val = &args[i][eq + 1..];
                set_vars.push((key.to_string(), val.to_string()));
                i += 1;
            } else {
                cmd_start = Some(i);
                break;
            }
        }
    }

    // If no command, print the environment
    let cmd_idx = match cmd_start {
        Some(idx) => idx,
        None => {
            // Apply modifications to our own env, then print
            if clear_env {
                // Print only the vars we set
                for (k, v) in &set_vars {
                    println!("{}={}", k, v);
                }
            } else {
                for (k, v) in &set_vars {
                    std::env::set_var(k, v);
                }
                for k in &unset_vars {
                    std::env::remove_var(k);
                }
                for (k, v) in std::env::vars() {
                    println!("{}={}", k, v);
                }
            }
            return ExitCode::SUCCESS;
        }
    };

    let program = &args[cmd_idx];
    let cmd_args = &args[cmd_idx + 1..];

    let mut cmd = Command::new(program);
    cmd.args(cmd_args);

    if clear_env {
        cmd.env_clear();
    }

    for k in &unset_vars {
        cmd.env_remove(k);
    }

    for (k, v) in &set_vars {
        cmd.env(k, v);
    }

    match cmd.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("env: '{}': {}", program, e);
            ExitCode::from(127)
        }
    }
}
