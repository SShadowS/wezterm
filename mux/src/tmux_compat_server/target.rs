//! Parser for tmux `-t TARGET` strings.
//!
//! The tmux target format is `SESSION:WINDOW.PANE` where each component is
//! optional. Individual components can be specified by ID (`$N`, `@N`, `%N`),
//! by name/index, or omitted entirely.
//!
//! Examples:
//! - `%5`           -> pane ID 5
//! - `$0:@1.%2`    -> session ID 0, window ID 1, pane ID 2
//! - `mysession:0.1`-> session "mysession", window index 0, pane index 1
//! - `:0.0`        -> current session, window index 0, pane index 0
//! - `@3`          -> window ID 3
//! - `$2`          -> session ID 2
//! - (empty)       -> all current context

use anyhow::{bail, Result};

/// Reference to a tmux session, either by numeric ID or by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRef {
    Id(u64),
    Name(String),
}

/// Reference to a tmux window, either by ID (`@N`), numeric index, or name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowRef {
    Id(u64),
    Index(u64),
    Name(String),
}

/// Reference to a tmux pane, either by ID (`%N`) or numeric index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneRef {
    Id(u64),
    Index(u64),
}

/// A parsed tmux target. Each component is `None` when not specified,
/// meaning "use the current/default" for that level.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TmuxTarget {
    pub session: Option<SessionRef>,
    pub window: Option<WindowRef>,
    pub pane: Option<PaneRef>,
}

/// Parse a tmux target string into its constituent session, window, and pane
/// references.
///
/// The general format is `SESSION:WINDOW.PANE`, but there are several special
/// cases for bare tokens:
///
/// - A bare `%N` is always a pane ID.
/// - A bare `@N` is always a window ID.
/// - A bare `$N` is always a session ID.
/// - A bare number or name (no `:` or `.`) is treated as a session name/reference.
pub fn parse_target(target: &str) -> Result<TmuxTarget> {
    if target.is_empty() {
        return Ok(TmuxTarget::default());
    }

    // Handle bare pane ID: `%N` with no colon or dot.
    if target.starts_with('%') && !target.contains(':') && !target.contains('.') {
        let id = parse_id_number(&target[1..])?;
        return Ok(TmuxTarget {
            pane: Some(PaneRef::Id(id)),
            ..Default::default()
        });
    }

    // Handle bare window ID: `@N` with no colon or dot.
    if target.starts_with('@') && !target.contains(':') && !target.contains('.') {
        let id = parse_id_number(&target[1..])?;
        return Ok(TmuxTarget {
            window: Some(WindowRef::Id(id)),
            ..Default::default()
        });
    }

    // Handle bare session ID: `$N` with no colon or dot.
    if target.starts_with('$') && !target.contains(':') && !target.contains('.') {
        let id = parse_id_number(&target[1..])?;
        return Ok(TmuxTarget {
            session: Some(SessionRef::Id(id)),
            ..Default::default()
        });
    }

    // Split on `:` to separate the session part from the window.pane part.
    let (session_part, window_pane_part) = if let Some(colon_pos) = target.find(':') {
        (
            &target[..colon_pos],
            Some(&target[colon_pos + 1..]),
        )
    } else {
        // No colon: the entire string is the window.pane portion
        // (no session specified).
        ("", Some(target.as_ref()))
    };

    let session = parse_session_ref(session_part)?;

    let (window, pane) = match window_pane_part {
        Some(wp) if !wp.is_empty() => parse_window_pane(wp)?,
        _ => (None, None),
    };

    Ok(TmuxTarget {
        session,
        window,
        pane,
    })
}

/// Parse a session reference from the text before the first `:`.
fn parse_session_ref(s: &str) -> Result<Option<SessionRef>> {
    if s.is_empty() {
        return Ok(None);
    }

    if let Some(rest) = s.strip_prefix('$') {
        let id = parse_id_number(rest)?;
        Ok(Some(SessionRef::Id(id)))
    } else {
        Ok(Some(SessionRef::Name(s.to_string())))
    }
}

/// Parse the `WINDOW.PANE` portion of a target string.
///
/// The dot separates window from pane. Either part may be absent if the dot is
/// at the edge, and the dot itself may be absent (window only).
fn parse_window_pane(s: &str) -> Result<(Option<WindowRef>, Option<PaneRef>)> {
    if s.is_empty() {
        return Ok((None, None));
    }

    let (window_part, pane_part) = if let Some(dot_pos) = find_pane_separator(s) {
        (&s[..dot_pos], Some(&s[dot_pos + 1..]))
    } else {
        (s, None)
    };

    let window = parse_window_ref(window_part)?;

    let pane = match pane_part {
        Some(p) if !p.is_empty() => Some(parse_pane_ref(p)?),
        _ => None,
    };

    Ok((window, pane))
}

/// Find the position of the `.` that separates the window part from the pane
/// part.
///
/// We need to be careful: the dot must not appear inside a window name that
/// would be ambiguous. The rule is straightforward since window names that
/// contain dots are unusual in tmux targets. We find the last `.` that could
/// be a separator. However, to keep things simple and match tmux's behavior,
/// we use the *first* `.` as the separator.
fn find_pane_separator(s: &str) -> Option<usize> {
    s.find('.')
}

/// Parse a window reference token.
fn parse_window_ref(s: &str) -> Result<Option<WindowRef>> {
    if s.is_empty() {
        return Ok(None);
    }

    if let Some(rest) = s.strip_prefix('@') {
        let id = parse_id_number(rest)?;
        Ok(Some(WindowRef::Id(id)))
    } else if let Ok(index) = s.parse::<u64>() {
        Ok(Some(WindowRef::Index(index)))
    } else {
        Ok(Some(WindowRef::Name(s.to_string())))
    }
}

/// Parse a pane reference token.
fn parse_pane_ref(s: &str) -> Result<PaneRef> {
    if let Some(rest) = s.strip_prefix('%') {
        let id = parse_id_number(rest)?;
        Ok(PaneRef::Id(id))
    } else if let Ok(index) = s.parse::<u64>() {
        Ok(PaneRef::Index(index))
    } else {
        bail!("invalid pane reference: {s:?}");
    }
}

/// Parse the numeric portion after a sigil (`$`, `@`, or `%`).
fn parse_id_number(s: &str) -> Result<u64> {
    if s.is_empty() {
        bail!("expected a number after sigil");
    }
    s.parse::<u64>()
        .map_err(|_| anyhow::anyhow!("invalid numeric id: {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to reduce boilerplate in assertions.
    fn parse(s: &str) -> TmuxTarget {
        parse_target(s).unwrap_or_else(|e| panic!("parse_target({s:?}) failed: {e}", s = s, e = e))
    }

    #[test]
    fn empty_target() {
        assert_eq!(
            parse(""),
            TmuxTarget {
                session: None,
                window: None,
                pane: None,
            }
        );
    }

    #[test]
    fn bare_pane_id() {
        assert_eq!(
            parse("%5"),
            TmuxTarget {
                session: None,
                window: None,
                pane: Some(PaneRef::Id(5)),
            }
        );
    }

    #[test]
    fn bare_window_id() {
        assert_eq!(
            parse("@3"),
            TmuxTarget {
                session: None,
                window: Some(WindowRef::Id(3)),
                pane: None,
            }
        );
    }

    #[test]
    fn bare_session_id() {
        assert_eq!(
            parse("$2"),
            TmuxTarget {
                session: Some(SessionRef::Id(2)),
                window: None,
                pane: None,
            }
        );
    }

    #[test]
    fn full_target_with_ids() {
        assert_eq!(
            parse("$0:@1.%2"),
            TmuxTarget {
                session: Some(SessionRef::Id(0)),
                window: Some(WindowRef::Id(1)),
                pane: Some(PaneRef::Id(2)),
            }
        );
    }

    #[test]
    fn session_name_with_indices() {
        assert_eq!(
            parse("mysession:0.1"),
            TmuxTarget {
                session: Some(SessionRef::Name("mysession".to_string())),
                window: Some(WindowRef::Index(0)),
                pane: Some(PaneRef::Index(1)),
            }
        );
    }

    #[test]
    fn no_session_with_window_and_pane_indices() {
        assert_eq!(
            parse(":0.0"),
            TmuxTarget {
                session: None,
                window: Some(WindowRef::Index(0)),
                pane: Some(PaneRef::Index(0)),
            }
        );
    }

    #[test]
    fn window_name_with_pane_id() {
        assert_eq!(
            parse("mywin.%3"),
            TmuxTarget {
                session: None,
                window: Some(WindowRef::Name("mywin".to_string())),
                pane: Some(PaneRef::Id(3)),
            }
        );
    }

    #[test]
    fn session_id_and_window_id_no_pane() {
        assert_eq!(
            parse("$1:@2"),
            TmuxTarget {
                session: Some(SessionRef::Id(1)),
                window: Some(WindowRef::Id(2)),
                pane: None,
            }
        );
    }

    #[test]
    fn session_name_only_with_colon() {
        // `mysession:` means session name with no window/pane.
        assert_eq!(
            parse("mysession:"),
            TmuxTarget {
                session: Some(SessionRef::Name("mysession".to_string())),
                window: None,
                pane: None,
            }
        );
    }

    #[test]
    fn window_index_only() {
        // No colon, no sigil, just a number -> in the no-colon path this is
        // treated as window.pane portion, and a bare number is a window index.
        assert_eq!(
            parse(":3"),
            TmuxTarget {
                session: None,
                window: Some(WindowRef::Index(3)),
                pane: None,
            }
        );
    }

    #[test]
    fn session_id_with_window_index_and_pane_index() {
        assert_eq!(
            parse("$10:2.7"),
            TmuxTarget {
                session: Some(SessionRef::Id(10)),
                window: Some(WindowRef::Index(2)),
                pane: Some(PaneRef::Index(7)),
            }
        );
    }

    #[test]
    fn large_ids() {
        assert_eq!(
            parse("$999:@1000.%2000"),
            TmuxTarget {
                session: Some(SessionRef::Id(999)),
                window: Some(WindowRef::Id(1000)),
                pane: Some(PaneRef::Id(2000)),
            }
        );
    }

    #[test]
    fn pane_index_after_dot() {
        assert_eq!(
            parse(":@1.3"),
            TmuxTarget {
                session: None,
                window: Some(WindowRef::Id(1)),
                pane: Some(PaneRef::Index(3)),
            }
        );
    }

    #[test]
    fn session_name_and_window_name() {
        assert_eq!(
            parse("sess:win"),
            TmuxTarget {
                session: Some(SessionRef::Name("sess".to_string())),
                window: Some(WindowRef::Name("win".to_string())),
                pane: None,
            }
        );
    }

    #[test]
    fn colon_only() {
        // `:` means no session, no window, no pane.
        assert_eq!(
            parse(":"),
            TmuxTarget {
                session: None,
                window: None,
                pane: None,
            }
        );
    }

    #[test]
    fn invalid_pane_ref() {
        assert!(parse_target(":0.abc").is_err());
    }

    #[test]
    fn invalid_session_id() {
        assert!(parse_target("$abc").is_err());
    }

    #[test]
    fn invalid_window_id() {
        assert!(parse_target(":@abc").is_err());
    }

    #[test]
    fn invalid_pane_id() {
        assert!(parse_target("%xyz").is_err());
    }

    #[test]
    fn bare_pane_zero() {
        assert_eq!(
            parse("%0"),
            TmuxTarget {
                session: None,
                window: None,
                pane: Some(PaneRef::Id(0)),
            }
        );
    }

    #[test]
    fn window_dot_pane_no_session() {
        // `0.0` with no colon: treated as window.pane in the no-colon path.
        assert_eq!(
            parse("0.0"),
            TmuxTarget {
                session: None,
                window: Some(WindowRef::Index(0)),
                pane: Some(PaneRef::Index(0)),
            }
        );
    }
}
