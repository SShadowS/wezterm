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
}

/// Expand a tmux format string, substituting `#{variable}` placeholders
/// and evaluating `#{?condition,true_value,false_value}` conditionals
/// using the provided context.
///
/// Unknown variables expand to the empty string.
pub fn expand_format(fmt: &str, ctx: &FormatContext) -> String {
    let mut output = String::with_capacity(fmt.len());
    let bytes = fmt.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len && bytes[i] == b'#' && bytes[i + 1] == b'{' {
            // Start of a format expression. Find the matching closing brace,
            // accounting for nested braces (tmux doesn't nest `#{}` but we
            // handle brace depth for robustness).
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
}
