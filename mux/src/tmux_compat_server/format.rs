//! Tmux format string expander.
//!
//! Expands tmux-style format strings such as `#{pane_id}`, `#{window_id}`,
//! and conditional expressions like `#{?pane_active,active,}`.

use std::fmt::Write;

/// Holds all the state needed to resolve tmux format variables.
#[derive(Debug, Clone, Default)]
pub struct FormatContext {
    pub pane_id: u64,
    pub pane_index: u64,
    pub pane_width: u64,
    pub pane_height: u64,
    pub pane_active: bool,
    pub pane_left: u64,
    pub pane_top: u64,
    pub pane_dead: bool,
    pub window_id: u64,
    pub window_index: u64,
    pub window_name: String,
    pub window_active: bool,
    pub window_width: u64,
    pub window_height: u64,
    pub session_id: u64,
    pub session_name: String,
    pub cursor_x: u64,
    pub cursor_y: u64,
    pub history_limit: u64,
    pub history_size: u64,
    // Phase 10: additional format variables
    pub pane_title: String,
    pub pane_current_command: String,
    pub pane_current_path: String,
    pub pane_pid: u64,
    pub pane_mode: String,
    pub window_flags: String,
    pub window_panes: u64,
    pub session_windows: u64,
    pub session_attached: u64,
    pub client_name: String,
    pub socket_path: String,
    pub server_pid: u64,
    // Phase 11: buffer format variables (used by list-buffers)
    pub buffer_name: String,
    pub buffer_size: u64,
    pub buffer_sample: String,
}

impl FormatContext {
    /// Set the window as active and prepend `*` to window_flags.
    pub fn set_window_active(&mut self, active: bool) {
        self.window_active = active;
        if active && !self.window_flags.contains('*') {
            self.window_flags.insert(0, '*');
        }
    }
}

/// Map a single-character tmux short-form alias to the equivalent long-form
/// variable name. Returns `None` if the character is not a recognized alias.
///
/// These match tmux's `format_table[]` in `format.c`.
fn short_alias_to_variable(ch: u8) -> Option<&'static str> {
    match ch {
        b'D' => Some("pane_id"),
        b'F' => Some("window_flags"),
        b'I' => Some("window_index"),
        b'P' => Some("pane_index"),
        b'S' => Some("session_name"),
        b'T' => Some("pane_title"),
        b'W' => Some("window_name"),
        _ => None,
    }
}

/// Expand a tmux format string, substituting `#{variable}` placeholders,
/// single-character `#X` short-form aliases, and evaluating
/// `#{?condition,true_value,false_value}` conditionals using the provided
/// context.
///
/// Short-form aliases (from tmux `format_table[]`):
///   `#D` → `#{pane_id}`, `#F` → `#{window_flags}`,
///   `#I` → `#{window_index}`, `#P` → `#{pane_index}`,
///   `#S` → `#{session_name}`, `#T` → `#{pane_title}`,
///   `#W` → `#{window_name}`
///
/// `##` expands to a literal `#`.
///
/// Unknown variables expand to the empty string.
pub fn expand_format(fmt: &str, ctx: &FormatContext) -> String {
    let mut output = String::with_capacity(fmt.len());
    let bytes = fmt.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len && bytes[i] == b'#' {
            let next = bytes[i + 1];
            if next == b'{' {
                // Long-form expression: #{variable} or #{?cond,t,f}
                let start = i + 2;
                if let Some(end) = find_matching_brace(bytes, start) {
                    let expr = &fmt[start..end];
                    expand_expr(expr, ctx, &mut output);
                    i = end + 1;
                } else {
                    // No matching `}` found — emit the `#{` literally and move on.
                    output.push_str("#{");
                    i += 2;
                }
            } else if next == b'#' {
                // `##` → literal `#`
                output.push('#');
                i += 2;
            } else if let Some(var_name) = short_alias_to_variable(next) {
                // Short-form alias: #D, #F, #I, #P, #S, #T, #W
                resolve_variable(var_name, ctx, &mut output);
                i += 2;
            } else {
                // Unrecognized `#X` — emit literally.
                output.push('#');
                i += 1;
            }
        } else {
            output.push(fmt[i..].chars().next().unwrap());
            i += fmt[i..].chars().next().unwrap().len_utf8();
        }
    }

    output
}

/// Find the index of the `}` that closes the brace opened at `start`,
/// respecting nested brace pairs.
fn find_matching_brace(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth: usize = 1;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Expand a single expression (the content between `#{` and `}`).
///
/// This handles both plain variable names like `pane_id` and conditional
/// expressions like `?pane_active,active,inactive`.
fn expand_expr(expr: &str, ctx: &FormatContext, output: &mut String) {
    if let Some(rest) = expr.strip_prefix('?') {
        expand_conditional(rest, ctx, output);
    } else {
        resolve_variable(expr, ctx, output);
    }
}

/// Expand a conditional expression of the form `condition,true_str,false_str`.
///
/// The condition is resolved as a variable. If the resolved value is non-empty
/// and not `"0"`, the true branch is used; otherwise the false branch.
///
/// The commas that separate the three parts are found at the top level only
/// (i.e., commas inside nested `#{}` expressions are not treated as
/// separators).
fn expand_conditional(rest: &str, ctx: &FormatContext, output: &mut String) {
    // Split into exactly three parts: condition, true_str, false_str.
    // We need to split on top-level commas only (not inside braces).
    let parts = split_conditional_parts(rest);

    let (condition, true_str, false_str) = match parts.len() {
        3 => (&parts[0], &parts[1], &parts[2]),
        2 => (&parts[0], &parts[1], &"".to_string()),
        _ => {
            // Malformed conditional — emit nothing.
            return;
        }
    };

    // Resolve the condition variable.
    let mut cond_value = String::new();
    resolve_variable(condition, ctx, &mut cond_value);

    let is_true = !cond_value.is_empty() && cond_value != "0";

    let branch = if is_true { true_str } else { false_str };

    // The branch itself may contain `#{}` expressions, so expand it.
    output.push_str(&expand_format(branch, ctx));
}

/// Split the conditional body on top-level commas (those not nested inside
/// `#{}` expressions). Returns a `Vec` of the parts as `String`s.
fn split_conditional_parts(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut depth: usize = 0;
    let mut start = 0;

    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b',' if depth == 0 => {
                parts.push(s[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    // Push the final segment.
    parts.push(s[start..].to_string());
    parts
}

/// Resolve a single variable name and write the result into `output`.
///
/// Variable names are matched exactly. Unknown names produce no output.
fn resolve_variable(name: &str, ctx: &FormatContext, output: &mut String) {
    match name {
        "pane_id" => {
            let _ = write!(output, "%{}", ctx.pane_id);
        }
        "window_id" => {
            let _ = write!(output, "@{}", ctx.window_id);
        }
        "session_id" => {
            let _ = write!(output, "${}", ctx.session_id);
        }
        "pane_index" => {
            let _ = write!(output, "{}", ctx.pane_index);
        }
        "pane_width" => {
            let _ = write!(output, "{}", ctx.pane_width);
        }
        "pane_height" => {
            let _ = write!(output, "{}", ctx.pane_height);
        }
        "pane_active" => {
            output.push(if ctx.pane_active { '1' } else { '0' });
        }
        "pane_left" => {
            let _ = write!(output, "{}", ctx.pane_left);
        }
        "pane_top" => {
            let _ = write!(output, "{}", ctx.pane_top);
        }
        "pane_dead" => {
            output.push(if ctx.pane_dead { '1' } else { '0' });
        }
        "window_index" => {
            let _ = write!(output, "{}", ctx.window_index);
        }
        "window_name" => {
            output.push_str(&ctx.window_name);
        }
        "window_active" => {
            output.push(if ctx.window_active { '1' } else { '0' });
        }
        "window_width" => {
            let _ = write!(output, "{}", ctx.window_width);
        }
        "window_height" => {
            let _ = write!(output, "{}", ctx.window_height);
        }
        "session_name" => {
            output.push_str(&ctx.session_name);
        }
        "cursor_x" => {
            let _ = write!(output, "{}", ctx.cursor_x);
        }
        "cursor_y" => {
            let _ = write!(output, "{}", ctx.cursor_y);
        }
        "history_limit" => {
            let _ = write!(output, "{}", ctx.history_limit);
        }
        "history_size" => {
            let _ = write!(output, "{}", ctx.history_size);
        }
        // Phase 10: additional format variables
        "pane_title" => {
            output.push_str(&ctx.pane_title);
        }
        "pane_current_command" => {
            output.push_str(&ctx.pane_current_command);
        }
        "pane_current_path" => {
            output.push_str(&ctx.pane_current_path);
        }
        "pane_pid" => {
            let _ = write!(output, "{}", ctx.pane_pid);
        }
        "pane_mode" => {
            output.push_str(&ctx.pane_mode);
        }
        "window_flags" => {
            output.push_str(&ctx.window_flags);
        }
        "window_panes" => {
            let _ = write!(output, "{}", ctx.window_panes);
        }
        "session_windows" => {
            let _ = write!(output, "{}", ctx.session_windows);
        }
        "session_attached" => {
            let _ = write!(output, "{}", ctx.session_attached);
        }
        "client_name" => {
            output.push_str(&ctx.client_name);
        }
        "socket_path" => {
            output.push_str(&ctx.socket_path);
        }
        "version" => {
            output.push_str("3.3a");
        }
        "pid" => {
            let _ = write!(output, "{}", ctx.server_pid);
        }
        // Phase 11: buffer format variables
        "buffer_name" => {
            output.push_str(&ctx.buffer_name);
        }
        "buffer_size" => {
            let _ = write!(output, "{}", ctx.buffer_size);
        }
        "buffer_sample" => {
            output.push_str(&ctx.buffer_sample);
        }
        _ => {
            // Unknown variable — expand to empty string.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a context with common defaults for testing.
    fn test_ctx() -> FormatContext {
        FormatContext {
            pane_id: 5,
            pane_index: 0,
            pane_width: 80,
            pane_height: 24,
            pane_active: true,
            pane_left: 0,
            pane_top: 0,
            pane_dead: false,
            window_id: 1,
            window_index: 0,
            window_name: "bash".to_string(),
            window_active: true,
            window_width: 80,
            window_height: 24,
            session_id: 0,
            session_name: "main".to_string(),
            cursor_x: 3,
            cursor_y: 7,
            history_limit: 2000,
            history_size: 150,
            pane_title: "~/project".to_string(),
            pane_current_command: "vim".to_string(),
            pane_current_path: "/home/user/project".to_string(),
            pane_pid: 12345,
            pane_mode: String::new(),
            window_flags: "*".to_string(),
            window_panes: 2,
            session_windows: 3,
            session_attached: 1,
            client_name: "/dev/pts/0".to_string(),
            socket_path: "/tmp/tmux-1000/default".to_string(),
            server_pid: 9999,
            buffer_name: String::new(),
            buffer_size: 0,
            buffer_sample: String::new(),
        }
    }

    #[test]
    fn pane_id() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{pane_id}", &ctx), "%5");
    }

    #[test]
    fn window_id() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{window_id}", &ctx), "@1");
    }

    #[test]
    fn session_id() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{session_id}", &ctx), "$0");
    }

    #[test]
    fn pane_index_and_pane_id() {
        let ctx = FormatContext {
            pane_index: 0,
            pane_id: 3,
            ..Default::default()
        };
        assert_eq!(expand_format("#{pane_index} #{pane_id}", &ctx), "0 %3");
    }

    #[test]
    fn dimensions() {
        let ctx = FormatContext {
            pane_width: 80,
            pane_height: 24,
            ..Default::default()
        };
        assert_eq!(expand_format("#{pane_width}x#{pane_height}", &ctx), "80x24");
    }

    #[test]
    fn conditional_true() {
        let ctx = FormatContext {
            pane_active: true,
            ..Default::default()
        };
        assert_eq!(expand_format("#{?pane_active,active,}", &ctx), "active");
    }

    #[test]
    fn conditional_false() {
        let ctx = FormatContext {
            pane_active: false,
            ..Default::default()
        };
        assert_eq!(expand_format("#{?pane_active,active,}", &ctx), "");
    }

    #[test]
    fn conditional_true_with_spaces() {
        let ctx = FormatContext {
            pane_active: true,
            ..Default::default()
        };
        assert_eq!(
            expand_format("#{?pane_active, (active),}", &ctx),
            " (active)"
        );
    }

    #[test]
    fn conditional_false_branch_nonempty() {
        let ctx = FormatContext {
            pane_active: false,
            ..Default::default()
        };
        assert_eq!(expand_format("#{?pane_active,yes,no}", &ctx), "no");
    }

    #[test]
    fn plain_text_no_expansion() {
        let ctx = test_ctx();
        assert_eq!(expand_format("plain text", &ctx), "plain text");
    }

    #[test]
    fn unknown_variable() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{unknown_var}", &ctx), "");
    }

    #[test]
    fn empty_format_string() {
        let ctx = test_ctx();
        assert_eq!(expand_format("", &ctx), "");
    }

    #[test]
    fn list_panes_default_format() {
        let ctx = FormatContext {
            pane_index: 0,
            pane_width: 80,
            pane_height: 24,
            pane_id: 5,
            pane_active: true,
            ..Default::default()
        };
        let fmt =
            "#{pane_index}: [#{pane_width}x#{pane_height}] #{pane_id}#{?pane_active, (active),}";
        assert_eq!(expand_format(fmt, &ctx), "0: [80x24] %5 (active)");
    }

    #[test]
    fn list_panes_inactive() {
        let ctx = FormatContext {
            pane_index: 1,
            pane_width: 40,
            pane_height: 24,
            pane_id: 6,
            pane_active: false,
            ..Default::default()
        };
        let fmt =
            "#{pane_index}: [#{pane_width}x#{pane_height}] #{pane_id}#{?pane_active, (active),}";
        assert_eq!(expand_format(fmt, &ctx), "1: [40x24] %6");
    }

    #[test]
    fn all_simple_variables() {
        let ctx = test_ctx();

        assert_eq!(expand_format("#{pane_id}", &ctx), "%5");
        assert_eq!(expand_format("#{pane_index}", &ctx), "0");
        assert_eq!(expand_format("#{pane_width}", &ctx), "80");
        assert_eq!(expand_format("#{pane_height}", &ctx), "24");
        assert_eq!(expand_format("#{pane_active}", &ctx), "1");
        assert_eq!(expand_format("#{pane_left}", &ctx), "0");
        assert_eq!(expand_format("#{pane_top}", &ctx), "0");
        assert_eq!(expand_format("#{pane_dead}", &ctx), "0");
        assert_eq!(expand_format("#{window_id}", &ctx), "@1");
        assert_eq!(expand_format("#{window_index}", &ctx), "0");
        assert_eq!(expand_format("#{window_name}", &ctx), "bash");
        assert_eq!(expand_format("#{window_active}", &ctx), "1");
        assert_eq!(expand_format("#{window_width}", &ctx), "80");
        assert_eq!(expand_format("#{window_height}", &ctx), "24");
        assert_eq!(expand_format("#{session_id}", &ctx), "$0");
        assert_eq!(expand_format("#{session_name}", &ctx), "main");
        assert_eq!(expand_format("#{cursor_x}", &ctx), "3");
        assert_eq!(expand_format("#{cursor_y}", &ctx), "7");
        assert_eq!(expand_format("#{history_limit}", &ctx), "2000");
        assert_eq!(expand_format("#{history_size}", &ctx), "150");
    }

    #[test]
    fn boolean_variables_false() {
        let ctx = FormatContext {
            pane_active: false,
            window_active: false,
            pane_dead: true,
            ..Default::default()
        };
        assert_eq!(expand_format("#{pane_active}", &ctx), "0");
        assert_eq!(expand_format("#{window_active}", &ctx), "0");
        assert_eq!(expand_format("#{pane_dead}", &ctx), "1");
    }

    #[test]
    fn literal_hash_not_followed_by_brace() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#not_a_var", &ctx), "#not_a_var");
    }

    #[test]
    fn unclosed_brace() {
        let ctx = test_ctx();
        // `#{` with no closing `}` — emit literally and continue.
        assert_eq!(expand_format("#{pane_id", &ctx), "#{pane_id");
    }

    #[test]
    fn multiple_variables_inline() {
        let ctx = FormatContext {
            session_name: "dev".to_string(),
            window_index: 2,
            pane_index: 1,
            ..Default::default()
        };
        assert_eq!(
            expand_format("#{session_name}:#{window_index}.#{pane_index}", &ctx),
            "dev:2.1"
        );
    }

    #[test]
    fn conditional_on_window_active() {
        let active_ctx = FormatContext {
            window_active: true,
            ..Default::default()
        };
        let inactive_ctx = FormatContext {
            window_active: false,
            ..Default::default()
        };
        assert_eq!(expand_format("#{?window_active,*,-}", &active_ctx), "*");
        assert_eq!(expand_format("#{?window_active,*,-}", &inactive_ctx), "-");
    }

    #[test]
    fn conditional_on_pane_dead() {
        let dead_ctx = FormatContext {
            pane_dead: true,
            ..Default::default()
        };
        let alive_ctx = FormatContext {
            pane_dead: false,
            ..Default::default()
        };
        assert_eq!(expand_format("#{?pane_dead,DEAD,ALIVE}", &dead_ctx), "DEAD");
        assert_eq!(
            expand_format("#{?pane_dead,DEAD,ALIVE}", &alive_ctx),
            "ALIVE"
        );
    }

    #[test]
    fn adjacent_expansions() {
        let ctx = FormatContext {
            pane_id: 10,
            window_id: 3,
            ..Default::default()
        };
        assert_eq!(expand_format("#{pane_id}#{window_id}", &ctx), "%10@3");
    }

    #[test]
    fn conditional_unknown_variable_is_falsy() {
        let ctx = test_ctx();
        // Unknown variable resolves to "" which is falsy.
        assert_eq!(expand_format("#{?nonexistent,yes,no}", &ctx), "no");
    }

    // --- Phase 10: new format variable tests ---

    #[test]
    fn phase10_version() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{version}", &ctx), "3.3a");
    }

    #[test]
    fn phase10_pid() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{pid}", &ctx), "9999");
    }

    #[test]
    fn phase10_client_name() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{client_name}", &ctx), "/dev/pts/0");
    }

    #[test]
    fn phase10_socket_path() {
        let ctx = test_ctx();
        assert_eq!(
            expand_format("#{socket_path}", &ctx),
            "/tmp/tmux-1000/default"
        );
    }

    #[test]
    fn phase10_pane_title() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{pane_title}", &ctx), "~/project");
    }

    #[test]
    fn phase10_pane_current_command() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{pane_current_command}", &ctx), "vim");
    }

    #[test]
    fn phase10_pane_current_path() {
        let ctx = test_ctx();
        assert_eq!(
            expand_format("#{pane_current_path}", &ctx),
            "/home/user/project"
        );
    }

    #[test]
    fn phase10_pane_pid() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{pane_pid}", &ctx), "12345");
    }

    #[test]
    fn phase10_pane_mode_empty() {
        let ctx = test_ctx();
        // No mode infrastructure — always empty
        assert_eq!(expand_format("#{pane_mode}", &ctx), "");
    }

    #[test]
    fn phase10_window_flags() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{window_flags}", &ctx), "*");
    }

    #[test]
    fn phase10_window_panes() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{window_panes}", &ctx), "2");
    }

    #[test]
    fn phase10_session_windows() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{session_windows}", &ctx), "3");
    }

    #[test]
    fn phase10_session_attached() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#{session_attached}", &ctx), "1");
    }

    #[test]
    fn phase10_pane_mode_conditional() {
        // pane_mode is empty → falsy in conditional
        let ctx = test_ctx();
        assert_eq!(
            expand_format("#{?pane_mode,in-mode,normal}", &ctx),
            "normal"
        );
    }

    #[test]
    fn phase10_iterm2_version_detection() {
        // iTerm2 sends: display-message -p "#{version}"
        let ctx = test_ctx();
        assert_eq!(expand_format("#{version}", &ctx), "3.3a");
    }

    #[test]
    fn phase10_iterm2_window_listing_format() {
        // Subset of the format iTerm2 uses in list-windows
        let ctx = FormatContext {
            window_id: 2,
            window_name: "editor".to_string(),
            window_flags: "*Z".to_string(),
            window_panes: 3,
            window_active: true,
            ..Default::default()
        };
        let fmt = "#{window_id} #{window_name}#{window_flags} (#{window_panes} panes)";
        assert_eq!(expand_format(fmt, &ctx), "@2 editor*Z (3 panes)");
    }

    // --- Phase 14: short-form format alias tests ---

    #[test]
    fn phase14_short_alias_d_pane_id() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#D", &ctx), "%5");
    }

    #[test]
    fn phase14_short_alias_f_window_flags() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#F", &ctx), "*");
    }

    #[test]
    fn phase14_short_alias_i_window_index() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#I", &ctx), "0");
    }

    #[test]
    fn phase14_short_alias_p_pane_index() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#P", &ctx), "0");
    }

    #[test]
    fn phase14_short_alias_s_session_name() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#S", &ctx), "main");
    }

    #[test]
    fn phase14_short_alias_t_pane_title() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#T", &ctx), "~/project");
    }

    #[test]
    fn phase14_short_alias_w_window_name() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#W", &ctx), "bash");
    }

    #[test]
    fn phase14_double_hash_literal() {
        let ctx = test_ctx();
        assert_eq!(expand_format("##", &ctx), "#");
    }

    #[test]
    fn phase14_double_hash_in_text() {
        let ctx = test_ctx();
        assert_eq!(expand_format("foo ## bar", &ctx), "foo # bar");
    }

    #[test]
    fn phase14_mixed_short_and_long_form() {
        let ctx = FormatContext {
            session_name: "dev".to_string(),
            window_index: 2,
            pane_index: 1,
            ..Default::default()
        };
        // Mix of #S (short) and #{window_index} (long) and #P (short)
        assert_eq!(expand_format("#S:#{window_index}.#P", &ctx), "dev:2.1");
    }

    #[test]
    fn phase14_short_form_display_message_pattern() {
        // Claude Code uses: display-message -p '#S:#I.#P'
        let ctx = FormatContext {
            session_name: "main".to_string(),
            window_index: 0,
            pane_index: 3,
            ..Default::default()
        };
        assert_eq!(expand_format("#S:#I.#P", &ctx), "main:0.3");
    }

    #[test]
    fn phase14_short_form_list_panes_pattern() {
        // Claude Code uses: list-panes -F '#D #P'
        let ctx = FormatContext {
            pane_id: 7,
            pane_index: 2,
            ..Default::default()
        };
        assert_eq!(expand_format("#D #P", &ctx), "%7 2");
    }

    #[test]
    fn phase14_unrecognized_short_form_literal() {
        let ctx = test_ctx();
        // #X where X is not a known alias — emit '#' literally, then 'X'
        assert_eq!(expand_format("#Z", &ctx), "#Z");
    }

    #[test]
    fn phase14_hash_at_end_of_string() {
        let ctx = test_ctx();
        // Lone '#' at end — emit literally
        assert_eq!(expand_format("test#", &ctx), "test#");
    }

    #[test]
    fn phase14_all_short_aliases_match_long_form() {
        let ctx = test_ctx();
        assert_eq!(expand_format("#D", &ctx), expand_format("#{pane_id}", &ctx));
        assert_eq!(
            expand_format("#F", &ctx),
            expand_format("#{window_flags}", &ctx)
        );
        assert_eq!(
            expand_format("#I", &ctx),
            expand_format("#{window_index}", &ctx)
        );
        assert_eq!(
            expand_format("#P", &ctx),
            expand_format("#{pane_index}", &ctx)
        );
        assert_eq!(
            expand_format("#S", &ctx),
            expand_format("#{session_name}", &ctx)
        );
        assert_eq!(
            expand_format("#T", &ctx),
            expand_format("#{pane_title}", &ctx)
        );
        assert_eq!(
            expand_format("#W", &ctx),
            expand_format("#{window_name}", &ctx)
        );
    }
}
