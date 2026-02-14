//! tmux-compatible CLI shim for WezTerm.
//!
//! This binary is named `tmux` and placed on `$PATH` ahead of the real tmux
//! when WezTerm's tmux-compat mode is enabled.  It connects to the CC protocol
//! server (Phase 3) over a Unix domain socket, sends one command, reads the
//! `%begin`/`%end` response, prints the body to stdout, and exits.

// std::io::{Read, Write} used via UFCS in run_cc_exchange and raw_read_line.

// ---------------------------------------------------------------------------
// Command modes
// ---------------------------------------------------------------------------

/// What the shim should do after parsing CLI args.
enum Action {
    /// Print version string and exit 0.
    Version,
    /// Session management command (new-session, attach-session) — no-op, exit 0.
    SessionNoOp,
    /// Forward a one-shot command to the CC server.
    Command(String),
}

// ---------------------------------------------------------------------------
// Arg parsing
// ---------------------------------------------------------------------------

/// Parse the process's command-line arguments into an [`Action`].
///
/// The binary is invoked as `tmux [flags] [command] [args...]`.  We strip
/// connection-mode flags (`-C`, `-CC`, `-L`, `-S`, `-f`) and detect special
/// cases (version, session commands).  Everything else is reconstructed into
/// the command text that the CC server's `command_parser::parse_command()`
/// expects.
fn parse_args(args: &[String]) -> Action {
    // Skip argv[0] (the binary name).
    let args = if !args.is_empty() { &args[1..] } else { &[] };

    if args.is_empty() {
        // Bare `tmux` with no arguments — treat like `new-session`.
        return Action::SessionNoOp;
    }

    // Strip global flags that precede the command name.
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-V" => return Action::Version,
            // Connection-mode flags — skip.
            "-C" | "-CC" => {
                i += 1;
            }
            // Flags that consume the next argument — skip both.
            "-L" | "-S" | "-f" => {
                i += 2;
            }
            _ => break,
        }
    }

    let rest = &args[i..];
    if rest.is_empty() {
        return Action::SessionNoOp;
    }

    // Detect session management commands.
    match rest[0].as_str() {
        "new-session" | "new" | "attach-session" | "attach" | "a" => {
            return Action::SessionNoOp;
        }
        _ => {}
    }

    // Everything else: reconstruct the command text.
    // We need to re-quote arguments that contain spaces so the server's
    // shell_words-based parser can split them correctly.
    let command_text = rest
        .iter()
        .map(|a| {
            if a.contains(' ') || a.contains('"') || a.contains('\'') || a.is_empty() {
                // Shell-quote: wrap in single quotes, escaping existing single quotes.
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    Action::Command(command_text)
}

// ---------------------------------------------------------------------------
// CC protocol client
// ---------------------------------------------------------------------------

/// Outcome of reading a CC response block.
struct CcResponse {
    /// The text between `%begin` and `%end`/`%error`.
    body: String,
    /// `true` if the closing line was `%error`.
    is_error: bool,
}

#[cfg(test)]
/// Skip the initial handshake the CC server sends when a client connects.
///
/// The server sends:
/// 1. A greeting `%begin`/`%end` block (counter = 1)
/// 2. `%session-changed ...`
/// 3. One or more `%window-add ...`
///
/// We consume lines until we've seen the greeting `%end` and then drain any
/// `%`-prefixed notification lines that follow.
fn skip_handshake(reader: &mut impl std::io::BufRead) -> anyhow::Result<()> {
    let mut line = String::new();

    // Phase 1: read until we see the greeting %end.
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            anyhow::bail!("connection closed during handshake");
        }
        if line.trim().starts_with("%end ") {
            break;
        }
    }

    // Phase 2: drain any %-prefixed notification lines that follow.
    // We peek by reading into the internal buffer and checking without
    // consuming if the next content starts with '%'.
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            // EOF — handshake is done, nothing more to drain.
            break;
        }
        // Check if the next data starts with '%'.
        if available[0] != b'%' {
            break;
        }
        // It's a notification line — consume it.
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
/// Read a single `%begin`/`%end` (or `%begin`/`%error`) response block.
fn read_response(reader: &mut impl std::io::BufRead) -> anyhow::Result<CcResponse> {
    let mut line = String::new();
    let mut body = String::new();
    let mut in_block = false;

    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            if in_block {
                anyhow::bail!("connection closed while reading response");
            } else {
                anyhow::bail!("connection closed before response received");
            }
        }

        let trimmed = line.trim();

        if !in_block {
            // Skip any notification lines that arrive before our response.
            if trimmed.starts_with('%') && !trimmed.starts_with("%begin ") {
                continue;
            }
            if trimmed.starts_with("%begin ") {
                in_block = true;
                continue;
            }
            // Non-%-prefixed line before %begin — skip (shouldn't happen).
            continue;
        }

        // Inside a block.
        if trimmed.starts_with("%end ") {
            return Ok(CcResponse {
                body,
                is_error: false,
            });
        }
        if trimmed.starts_with("%error ") {
            return Ok(CcResponse {
                body,
                is_error: true,
            });
        }

        // Body line — accumulate.
        body.push_str(&line);
    }
}

/// Read a single line from the stream using raw reads.
/// Returns the line INCLUDING the trailing newline.
fn raw_read_line(
    stream: &mut impl std::io::Read,
    accum: &mut String,
    buf: &mut [u8],
) -> anyhow::Result<String> {
    loop {
        if let Some(pos) = accum.find('\n') {
            let line = accum[..=pos].to_string();
            accum.drain(..=pos);
            return Ok(line);
        }
        let n = stream.read(buf)?;
        if n == 0 {
            anyhow::bail!("connection closed");
        }
        accum.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
}

/// Run the CC protocol exchange on an already-connected stream.
///
/// Uses raw reads with manual line buffering — `BufReader::read_line()`
/// blocks indefinitely on Windows TCP sockets even when data is available.
fn run_cc_exchange(
    mut stream: impl std::io::Read + std::io::Write,
    command: &str,
    verbose: bool,
) -> anyhow::Result<CcResponse> {
    let mut accum = String::new();
    let mut buf = [0u8; 4096];

    if verbose {
        eprintln!("[tmux-shim] waiting for handshake...");
    }
    // Skip handshake: read until we see %end, then drain %-prefixed lines.
    loop {
        let line = raw_read_line(&mut stream, &mut accum, &mut buf)?;
        if verbose {
            eprintln!("[tmux-shim] hs line: {:?}", line.trim());
        }
        if line.trim().starts_with("%end ") {
            break;
        }
    }
    // Drain %-prefixed notification lines.
    // Peek at accumulated data to see if more %-lines follow.
    loop {
        // If there's a complete line in the accumulator, check it.
        if let Some(pos) = accum.find('\n') {
            let peek = accum[..pos].trim_start();
            if peek.starts_with('%') {
                let _notification = accum[..=pos].to_string();
                accum.drain(..=pos);
                if verbose {
                    eprintln!("[tmux-shim] hs notif: {:?}", _notification.trim());
                }
                continue;
            }
            // Non-% line — handshake is done.
            break;
        }
        // No complete line buffered — try a non-blocking-ish read.
        // We can't easily do non-blocking, so just break and let the
        // command/response flow handle any remaining notifications.
        break;
    }

    if verbose {
        eprintln!("[tmux-shim] handshake done, sending command: {}", command);
    }

    std::io::Write::write_all(&mut stream, command.as_bytes())?;
    std::io::Write::write_all(&mut stream, b"\n")?;
    std::io::Write::flush(&mut stream)?;

    if verbose {
        eprintln!("[tmux-shim] waiting for response...");
    }

    // Read response using raw reads.
    let mut body = String::new();
    let mut in_block = false;
    loop {
        let line = raw_read_line(&mut stream, &mut accum, &mut buf)?;
        let trimmed = line.trim();

        if !in_block {
            if trimmed.starts_with("%begin ") {
                in_block = true;
                continue;
            }
            // Skip notification lines before response.
            continue;
        }
        if trimmed.starts_with("%end ") {
            return Ok(CcResponse {
                body,
                is_error: false,
            });
        }
        if trimmed.starts_with("%error ") {
            return Ok(CcResponse {
                body,
                is_error: true,
            });
        }
        body.push_str(&line);
    }
}

/// Connect to the CC server, send a command, and return the response.
///
/// Supports two address formats in `WEZTERM_TMUX_CC`:
/// - `tcp:HOST:PORT` — connect via TCP (used on Windows)
/// - anything else — treat as a Unix domain socket path
fn execute_command(socket_path: &str, command: &str) -> anyhow::Result<CcResponse> {
    let verbose = std::env::var("WEZTERM_TMUX_CC_VERBOSE").is_ok();


    if let Some(addr) = socket_path.strip_prefix("tcp:") {
        let stream = std::net::TcpStream::connect(addr).map_err(|e| {
            anyhow::anyhow!("failed to connect to WezTerm CC server at {}: {}", addr, e)
        })?;
        // Disable Nagle to avoid delayed-ACK stalls on Windows localhost.
        let _ = stream.set_nodelay(true);
        if verbose {
            eprintln!("[tmux-shim] connected via TCP (nodelay)");
        }
        run_cc_exchange(stream, command, verbose)
    } else {
        let stream = wezterm_uds::UnixStream::connect(socket_path).map_err(|e| {
            anyhow::anyhow!(
                "failed to connect to WezTerm CC server at {}: {}",
                socket_path,
                e
            )
        })?;
        if verbose {
            eprintln!("[tmux-shim] connected via UDS");
        }
        run_cc_exchange(stream, command, verbose)
    }
}

// ---------------------------------------------------------------------------
// Fallthrough to real tmux
// ---------------------------------------------------------------------------

/// Attempt to find and exec the real `tmux` binary (skipping ourselves).
///
/// Returns an error if no real tmux is found.
fn exec_real_tmux(args: &[String]) -> anyhow::Result<()> {
    // Get our own executable path so we can skip it.
    let our_exe = std::env::current_exe().unwrap_or_default();

    // Search PATH for a `tmux` that isn't us.
    if let Ok(path_var) = std::env::var("PATH") {
        #[cfg(windows)]
        let sep = ';';
        #[cfg(not(windows))]
        let sep = ':';

        for dir in path_var.split(sep) {
            #[cfg(windows)]
            let candidate = std::path::Path::new(dir).join("tmux.exe");
            #[cfg(not(windows))]
            let candidate = std::path::Path::new(dir).join("tmux");

            if candidate.exists() && candidate != our_exe {
                // Found real tmux — exec it.
                let status = std::process::Command::new(&candidate)
                    .args(&args[1..])
                    .status()?;
                std::process::exit(status.code().unwrap_or(1));
            }
        }
    }

    anyhow::bail!("WEZTERM_TMUX_CC is not set and no real tmux binary found on PATH")
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if let Err(e) = run(&args) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> anyhow::Result<()> {
    let action = parse_args(args);

    match action {
        Action::Version => {
            println!("tmux 3.3a (wezterm-compat)");
            Ok(())
        }

        Action::SessionNoOp => {
            // Session management commands are no-ops — we're already "in" a session.
            Ok(())
        }

        Action::Command(command_text) => {
            // Find the CC server socket.
            let socket_path = match std::env::var("WEZTERM_TMUX_CC") {
                Ok(p) if !p.is_empty() => p,
                _ => {
                    // No CC socket — try to fall through to real tmux.
                    return exec_real_tmux(args);
                }
            };

            let response = execute_command(&socket_path, &command_text)?;

            if response.is_error {
                // Print error body to stderr (strip trailing newline for cleaner output).
                let msg = response.body.trim_end();
                if !msg.is_empty() {
                    eprintln!("{}", msg);
                }
                std::process::exit(1);
            }

            // Print success body to stdout verbatim.
            // The body already has appropriate newlines from the server.
            print!("{}", response.body);
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parse_version() {
        match parse_args(&args(&["tmux", "-V"])) {
            Action::Version => {}
            _ => panic!("expected Version"),
        }
    }

    #[test]
    fn parse_session_new() {
        match parse_args(&args(&["tmux", "-CC", "new-session", "-t", "main"])) {
            Action::SessionNoOp => {}
            _ => panic!("expected SessionNoOp"),
        }
    }

    #[test]
    fn parse_session_attach() {
        match parse_args(&args(&["tmux", "attach-session"])) {
            Action::SessionNoOp => {}
            _ => panic!("expected SessionNoOp"),
        }
    }

    #[test]
    fn parse_bare_tmux() {
        match parse_args(&args(&["tmux"])) {
            Action::SessionNoOp => {}
            _ => panic!("expected SessionNoOp"),
        }
    }

    #[test]
    fn parse_split_window() {
        match parse_args(&args(&["tmux", "split-window", "-h", "-t", "%3"])) {
            Action::Command(cmd) => assert_eq!(cmd, "split-window -h -t %3"),
            _ => panic!("expected Command"),
        }
    }

    #[test]
    fn parse_send_keys_with_spaces() {
        match parse_args(&args(&[
            "tmux",
            "send-keys",
            "-t",
            "%5",
            "echo hello",
            "Enter",
        ])) {
            Action::Command(cmd) => assert_eq!(cmd, "send-keys -t %5 'echo hello' Enter"),
            _ => panic!("expected Command"),
        }
    }

    #[test]
    fn parse_strips_connection_flags() {
        match parse_args(&args(&["tmux", "-C", "-L", "main", "list-panes", "-a"])) {
            Action::Command(cmd) => assert_eq!(cmd, "list-panes -a"),
            _ => panic!("expected Command"),
        }
    }

    #[test]
    fn parse_capture_pane() {
        match parse_args(&args(&[
            "tmux",
            "capture-pane",
            "-p",
            "-t",
            "%1",
            "-S",
            "-50",
        ])) {
            Action::Command(cmd) => assert_eq!(cmd, "capture-pane -p -t %1 -S -50"),
            _ => panic!("expected Command"),
        }
    }

    #[test]
    fn extract_response_success() {
        let data = "%begin 1234567890 1 1\nhello world\n%end 1234567890 1 1\n";
        let mut reader = std::io::BufReader::new(data.as_bytes());
        let resp = read_response(&mut reader).unwrap();
        assert!(!resp.is_error);
        assert_eq!(resp.body, "hello world\n");
    }

    #[test]
    fn extract_response_error() {
        let data = "%begin 1234567890 1 1\nno such pane\n%error 1234567890 1 1\n";
        let mut reader = std::io::BufReader::new(data.as_bytes());
        let resp = read_response(&mut reader).unwrap();
        assert!(resp.is_error);
        assert_eq!(resp.body, "no such pane\n");
    }

    #[test]
    fn extract_empty_response() {
        let data = "%begin 1234567890 1 1\n%end 1234567890 1 1\n";
        let mut reader = std::io::BufReader::new(data.as_bytes());
        let resp = read_response(&mut reader).unwrap();
        assert!(!resp.is_error);
        assert_eq!(resp.body, "");
    }

    #[test]
    fn extract_response_skips_notifications() {
        let data = "%window-pane-changed @0 %1\n%begin 1234567890 2 1\nok\n%end 1234567890 2 1\n";
        let mut reader = std::io::BufReader::new(data.as_bytes());
        let resp = read_response(&mut reader).unwrap();
        assert!(!resp.is_error);
        assert_eq!(resp.body, "ok\n");
    }

    #[test]
    fn skip_handshake_basic() {
        let data = "\
%begin 1700000000 1 1\n\
%end 1700000000 1 1\n\
%session-changed $0 default\n\
%window-add @0\n\
%window-add @1\n";
        let mut reader = std::io::BufReader::new(data.as_bytes());
        skip_handshake(&mut reader).unwrap();
        // After skipping, reader should be at EOF (all lines consumed).
        let mut remaining = String::new();
        reader.read_line(&mut remaining).unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn skip_handshake_with_body_in_greeting() {
        // The greeting block is always empty, but test resilience.
        let data = "\
%begin 1700000000 1 1\n\
some unexpected body\n\
%end 1700000000 1 1\n\
%session-changed $0 default\n";
        let mut reader = std::io::BufReader::new(data.as_bytes());
        skip_handshake(&mut reader).unwrap();
    }

    #[test]
    fn multiline_response_body() {
        let data = "%begin 1234567890 1 1\nline1\nline2\nline3\n%end 1234567890 1 1\n";
        let mut reader = std::io::BufReader::new(data.as_bytes());
        let resp = read_response(&mut reader).unwrap();
        assert!(!resp.is_error);
        assert_eq!(resp.body, "line1\nline2\nline3\n");
    }
}
