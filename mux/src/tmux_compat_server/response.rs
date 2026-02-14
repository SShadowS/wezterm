//! Generates tmux control mode (CC) wire-format responses and notifications.
//!
//! Every command response is wrapped in `%begin` / `%end` (or `%error`) guard
//! lines.  Asynchronous notifications are single `%`-prefixed lines.
//!
//! See `tmux(1)` *CONTROL MODE* for the full specification.

use std::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// vis encoding
// ---------------------------------------------------------------------------

/// Encode a byte slice using the tmux `vis`-style encoding used in `%output`
/// notifications.
///
/// Characters with ASCII value < 0x20 (space) and backslash (0x5C) are
/// replaced by a backslash followed by exactly three octal digits.  All other
/// bytes pass through unchanged.
pub fn vis_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len());
    for &b in data {
        if b < 0x20 || b == b'\\' {
            write!(out, "\\{:03o}", b).unwrap();
        } else {
            out.push(b as char);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// ResponseWriter — command response blocks
// ---------------------------------------------------------------------------

/// Writes tmux control-mode response blocks (`%begin`/`%end`/`%error`).
///
/// Each call to [`success`](Self::success), [`empty_success`](Self::empty_success),
/// or [`error`](Self::error) increments an internal counter so that every
/// response block carries a unique, monotonically increasing command number.
pub struct ResponseWriter {
    counter: u64,
}

impl ResponseWriter {
    pub fn new() -> Self {
        ResponseWriter { counter: 0 }
    }

    /// Return the next command number, advancing the internal counter.
    fn next_counter(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    /// Return the current Unix epoch timestamp in seconds.
    fn timestamp() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    /// Format a complete guard block.
    ///
    /// If `is_error` is `false` the closing line is `%end`; otherwise it is
    /// `%error`.  The timestamp is captured once and reused for both the
    /// opening and closing guard lines.
    fn format_guard_block(&mut self, body: &str, is_error: bool) -> String {
        let ts = Self::timestamp();
        let n = self.next_counter();
        self.format_guard_block_with(ts, n, body, is_error)
    }

    /// Inner helper that accepts an explicit timestamp and counter, making it
    /// easy to test without worrying about wall-clock time.
    fn format_guard_block_with(&self, ts: i64, n: u64, body: &str, is_error: bool) -> String {
        let end_tag = if is_error { "%error" } else { "%end" };

        let mut out = String::new();
        write!(out, "%begin {} {} 1\n", ts, n).unwrap();

        if !body.is_empty() {
            out.push_str(body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
        }

        write!(out, "{} {} {} 1\n", end_tag, ts, n).unwrap();
        out
    }

    /// Generate a successful response wrapping the given `output`.
    ///
    /// If `output` is empty the result is the same as [`empty_success`](Self::empty_success).
    pub fn success(&mut self, output: &str) -> String {
        self.format_guard_block(output, false)
    }

    /// Generate an empty success response (for commands that produce no
    /// output).
    pub fn empty_success(&mut self) -> String {
        self.format_guard_block("", false)
    }

    /// Generate an error response containing `message`.
    pub fn error(&mut self, message: &str) -> String {
        self.format_guard_block(message, true)
    }
}

// ---------------------------------------------------------------------------
// Notifications — free functions
// ---------------------------------------------------------------------------

/// `%output %<pane_id> <vis_encoded_data>`
pub fn output_notification(pane_id: u64, data: &[u8]) -> String {
    format!("%output %{} {}\n", pane_id, vis_encode(data))
}

/// `%layout-change @<window_id> <layout_string>`
pub fn layout_change_notification(window_id: u64, layout: &str) -> String {
    format!("%layout-change @{} {}\n", window_id, layout)
}

/// `%window-add @<window_id>`
pub fn window_add_notification(window_id: u64) -> String {
    format!("%window-add @{}\n", window_id)
}

/// `%window-close @<window_id>`
pub fn window_close_notification(window_id: u64) -> String {
    format!("%window-close @{}\n", window_id)
}

/// `%window-renamed @<window_id> <name>`
pub fn window_renamed_notification(window_id: u64, name: &str) -> String {
    format!("%window-renamed @{} {}\n", window_id, name)
}

/// `%window-pane-changed @<window_id> %<pane_id>`
pub fn window_pane_changed_notification(window_id: u64, pane_id: u64) -> String {
    format!("%window-pane-changed @{} %{}\n", window_id, pane_id)
}

/// `%session-changed $<session_id> <name>`
pub fn session_changed_notification(session_id: u64, name: &str) -> String {
    format!("%session-changed ${} {}\n", session_id, name)
}

/// `%session-renamed $<session_id> <name>`
pub fn session_renamed_notification(session_id: u64, name: &str) -> String {
    format!("%session-renamed ${} {}\n", session_id, name)
}

/// `%sessions-changed`
pub fn sessions_changed_notification() -> String {
    "%sessions-changed\n".to_string()
}

/// `%paste-buffer-changed <buffer_name>`
pub fn paste_buffer_changed_notification(buffer_name: &str) -> String {
    format!("%paste-buffer-changed {}\n", buffer_name)
}

/// `%paste-buffer-deleted <buffer_name>`
pub fn paste_buffer_deleted_notification(buffer_name: &str) -> String {
    format!("%paste-buffer-deleted {}\n", buffer_name)
}

/// `%session-window-changed $<session_id> @<window_id>`
pub fn session_window_changed_notification(session_id: u64, window_id: u64) -> String {
    format!("%session-window-changed ${} @{}\n", session_id, window_id)
}

/// `%exit` or `%exit <reason>`
pub fn exit_notification(reason: Option<&str>) -> String {
    match reason {
        Some(r) => format!("%exit {}\n", r),
        None => "%exit\n".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- vis_encode ---------------------------------------------------------

    #[test]
    fn vis_encode_plain_ascii() {
        assert_eq!(vis_encode(b"hello"), "hello");
    }

    #[test]
    fn vis_encode_crlf() {
        assert_eq!(vis_encode(b"hello\r\n"), "hello\\015\\012");
    }

    #[test]
    fn vis_encode_escape_sequence() {
        assert_eq!(vis_encode(b"\x1b[1mtest"), "\\033[1mtest");
    }

    #[test]
    fn vis_encode_backslash() {
        assert_eq!(vis_encode(b"back\\slash"), "back\\134slash");
    }

    #[test]
    fn vis_encode_tab_and_nul() {
        assert_eq!(vis_encode(b"\t\0"), "\\011\\000");
    }

    #[test]
    fn vis_encode_printable_unchanged() {
        assert_eq!(vis_encode(b"normal text 123!@#"), "normal text 123!@#");
    }

    #[test]
    fn vis_encode_empty() {
        assert_eq!(vis_encode(b""), "");
    }

    #[test]
    fn vis_encode_all_control_chars() {
        // Every byte from 0x00..0x1F should be encoded.
        for b in 0u8..0x20 {
            let encoded = vis_encode(&[b]);
            assert_eq!(encoded.len(), 4, "byte {:#04x} should produce 4 chars", b);
            assert!(
                encoded.starts_with('\\'),
                "byte {:#04x} should start with \\",
                b
            );
        }
    }

    #[test]
    fn vis_encode_space_not_encoded() {
        // 0x20 is the first byte that should NOT be encoded.
        assert_eq!(vis_encode(b" "), " ");
    }

    // -- ResponseWriter helpers ---------------------------------------------

    /// Parse a guard-block string and return `(timestamp, counter, body, is_error)`.
    fn parse_guard_block(s: &str) -> (i64, u64, String, bool) {
        let lines: Vec<&str> = s.split('\n').collect();
        // Last element after trailing newline is empty.
        assert!(
            lines.last() == Some(&""),
            "block should end with newline, got: {s:?}"
        );

        let begin_line = lines[0];
        let end_line = lines[lines.len() - 2];

        // Parse %begin
        let begin_parts: Vec<&str> = begin_line.splitn(4, ' ').collect();
        assert_eq!(begin_parts[0], "%begin");
        let ts: i64 = begin_parts[1].parse().unwrap();
        let counter: u64 = begin_parts[2].parse().unwrap();
        assert_eq!(begin_parts[3], "1");

        // Parse %end or %error
        let end_parts: Vec<&str> = end_line.splitn(4, ' ').collect();
        let is_error = end_parts[0] == "%error";
        assert!(
            end_parts[0] == "%end" || end_parts[0] == "%error",
            "unexpected closing tag: {}",
            end_parts[0]
        );
        let end_ts: i64 = end_parts[1].parse().unwrap();
        let end_counter: u64 = end_parts[2].parse().unwrap();
        assert_eq!(end_parts[3], "1");

        // Timestamps and counters must match.
        assert_eq!(ts, end_ts, "timestamps must match");
        assert_eq!(counter, end_counter, "counters must match");

        // Body is everything between the first and last meaningful lines.
        let body_lines = &lines[1..lines.len() - 2];
        let body = if body_lines.is_empty() {
            String::new()
        } else {
            body_lines.join("\n") + "\n"
        };

        (ts, counter, body, is_error)
    }

    // -- ResponseWriter: success --------------------------------------------

    #[test]
    fn empty_success_structure() {
        let mut w = ResponseWriter::new();
        let resp = w.empty_success();
        let (_ts, counter, body, is_error) = parse_guard_block(&resp);
        assert_eq!(counter, 1);
        assert_eq!(body, "");
        assert!(!is_error);
    }

    #[test]
    fn success_with_content() {
        let mut w = ResponseWriter::new();
        let resp = w.success("0 %0\n1 %1\n");
        let (_ts, counter, body, is_error) = parse_guard_block(&resp);
        assert_eq!(counter, 1);
        assert_eq!(body, "0 %0\n1 %1\n");
        assert!(!is_error);
    }

    #[test]
    fn success_appends_newline_when_missing() {
        let mut w = ResponseWriter::new();
        let resp = w.success("no trailing newline");
        let (_ts, _counter, body, _is_error) = parse_guard_block(&resp);
        assert_eq!(body, "no trailing newline\n");
    }

    #[test]
    fn success_empty_string_same_as_empty_success() {
        let mut w = ResponseWriter::new();
        let a = w.success("");
        let mut w2 = ResponseWriter::new();
        let b = w2.empty_success();
        // They may differ in timestamp, but structurally should be the same.
        let (_, _, body_a, err_a) = parse_guard_block(&a);
        let (_, _, body_b, err_b) = parse_guard_block(&b);
        assert_eq!(body_a, body_b);
        assert_eq!(err_a, err_b);
    }

    // -- ResponseWriter: error ----------------------------------------------

    #[test]
    fn error_structure() {
        let mut w = ResponseWriter::new();
        let resp = w.error("session not found");
        let (_ts, counter, body, is_error) = parse_guard_block(&resp);
        assert_eq!(counter, 1);
        assert_eq!(body, "session not found\n");
        assert!(is_error);
    }

    // -- Counter increments -------------------------------------------------

    #[test]
    fn counter_increments() {
        let mut w = ResponseWriter::new();

        let r1 = w.empty_success();
        let (_, c1, _, _) = parse_guard_block(&r1);
        assert_eq!(c1, 1);

        let r2 = w.success("data\n");
        let (_, c2, _, _) = parse_guard_block(&r2);
        assert_eq!(c2, 2);

        let r3 = w.error("oops");
        let (_, c3, _, _) = parse_guard_block(&r3);
        assert_eq!(c3, 3);
    }

    // -- format_guard_block_with (deterministic) ----------------------------

    #[test]
    fn format_guard_block_with_deterministic() {
        let w = ResponseWriter::new();
        let block = w.format_guard_block_with(1700000000, 42, "hello\n", false);
        assert_eq!(
            block,
            "%begin 1700000000 42 1\nhello\n%end 1700000000 42 1\n"
        );
    }

    #[test]
    fn format_guard_block_with_error_deterministic() {
        let w = ResponseWriter::new();
        let block = w.format_guard_block_with(1700000000, 7, "bad command", true);
        assert_eq!(
            block,
            "%begin 1700000000 7 1\nbad command\n%error 1700000000 7 1\n"
        );
    }

    #[test]
    fn format_guard_block_with_empty_body() {
        let w = ResponseWriter::new();
        let block = w.format_guard_block_with(1234567890, 1, "", false);
        assert_eq!(block, "%begin 1234567890 1 1\n%end 1234567890 1 1\n");
    }

    // -- Notifications ------------------------------------------------------

    #[test]
    fn output_notification_basic() {
        assert_eq!(
            output_notification(1, b"hello\r\n"),
            "%output %1 hello\\015\\012\n"
        );
    }

    #[test]
    fn output_notification_empty() {
        assert_eq!(output_notification(0, b""), "%output %0 \n");
    }

    #[test]
    fn layout_change_notification_basic() {
        assert_eq!(
            layout_change_notification(0, "b25f,80x24,0,0,2"),
            "%layout-change @0 b25f,80x24,0,0,2\n"
        );
    }

    #[test]
    fn window_add_notification_basic() {
        assert_eq!(window_add_notification(1), "%window-add @1\n");
    }

    #[test]
    fn window_close_notification_basic() {
        assert_eq!(window_close_notification(3), "%window-close @3\n");
    }

    #[test]
    fn window_renamed_notification_basic() {
        assert_eq!(
            window_renamed_notification(2, "editor"),
            "%window-renamed @2 editor\n"
        );
    }

    #[test]
    fn window_pane_changed_notification_basic() {
        assert_eq!(
            window_pane_changed_notification(1, 5),
            "%window-pane-changed @1 %5\n"
        );
    }

    #[test]
    fn session_changed_notification_basic() {
        assert_eq!(
            session_changed_notification(0, "main"),
            "%session-changed $0 main\n"
        );
    }

    #[test]
    fn session_renamed_notification_basic() {
        assert_eq!(
            session_renamed_notification(0, "newname"),
            "%session-renamed $0 newname\n"
        );
    }

    #[test]
    fn sessions_changed_notification_basic() {
        assert_eq!(sessions_changed_notification(), "%sessions-changed\n");
    }

    #[test]
    fn exit_notification_no_reason() {
        assert_eq!(exit_notification(None), "%exit\n");
    }

    #[test]
    fn exit_notification_with_reason() {
        assert_eq!(exit_notification(Some("detached")), "%exit detached\n");
    }

    #[test]
    fn paste_buffer_changed_notification_basic() {
        assert_eq!(
            paste_buffer_changed_notification("buffer0"),
            "%paste-buffer-changed buffer0\n"
        );
    }

    #[test]
    fn paste_buffer_changed_notification_numbered() {
        assert_eq!(
            paste_buffer_changed_notification("buffer5"),
            "%paste-buffer-changed buffer5\n"
        );
    }

    #[test]
    fn session_window_changed_notification_basic() {
        assert_eq!(
            session_window_changed_notification(0, 2),
            "%session-window-changed $0 @2\n"
        );
    }

    #[test]
    fn session_window_changed_notification_large_ids() {
        assert_eq!(
            session_window_changed_notification(3, 15),
            "%session-window-changed $3 @15\n"
        );
    }
}
