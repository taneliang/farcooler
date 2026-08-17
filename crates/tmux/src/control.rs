//! tmux control-mode client and notification parser.
//!
//! One long-lived `tmux -CC attach-session` client attaches to the runner session.
//! Because tmux sends output from every pane in every window of the attached
//! session, this client drains and routes all terminal output by stable pane ID.
//!
//! The control client is a STREAMING failure domain, not a process-lifetime
//! failure domain. If it exits or its parser rejects malformed output, managed
//! commands pause, tmux processes keep running, and the daemon reconnects,
//! re-inventories, and emits visible gaps before accepting new input.

/// A parsed control-mode notification.
///
/// Only the variants that affect identity or output are modelled. Anything else
/// is `Other`, which is deliberately inert: an unrecognized notification must
/// never be mistaken for a structural change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    /// `%output %pane data` with tmux octal escapes already decoded.
    Output { pane_id: String, bytes: Vec<u8> },
    /// A window went away. Structural: the view must be refreshed.
    WindowClose { window_id: String },
    /// The pane's window was unlinked. Structural.
    UnlinkedWindowClose { window_id: String },
    /// The control client itself is going away.
    Exit { reason: Option<String> },
    /// Command reply framing.
    Begin,
    End,
    Error { message: String },
    /// Recognized but not structural.
    Other(String),
}

impl Notification {
    /// Structural notifications are the ones that must update the live view.
    pub fn is_structural(&self) -> bool {
        matches!(
            self,
            Notification::WindowClose { .. }
                | Notification::UnlinkedWindowClose { .. }
                | Notification::Exit { .. }
        )
    }
}

/// Parse one control-mode line.
pub fn parse_line(line: &str) -> Option<Notification> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if !line.starts_with('%') {
        return None;
    }

    let (tag, rest) = match line.split_once(' ') {
        Some((t, r)) => (t, r),
        None => (line, ""),
    };

    Some(match tag {
        "%output" => {
            // `%output %12 <escaped-bytes>`
            let (pane, data) = rest.split_once(' ').unwrap_or((rest, ""));
            Notification::Output {
                pane_id: pane.to_string(),
                bytes: decode_escapes(data),
            }
        }
        "%window-close" => Notification::WindowClose { window_id: rest.trim().to_string() },
        "%unlinked-window-close" => {
            Notification::UnlinkedWindowClose { window_id: rest.trim().to_string() }
        }
        "%exit" => Notification::Exit {
            reason: (!rest.trim().is_empty()).then(|| rest.trim().to_string()),
        },
        "%begin" => Notification::Begin,
        "%end" => Notification::End,
        "%error" => Notification::Error { message: rest.trim().to_string() },
        other => Notification::Other(other.to_string()),
    })
}

/// tmux escapes non-printable output bytes as `\ooo` octal triples.
///
/// Decoding must be byte-exact: output bytes arrive exactly as written,
/// including escape sequences split across reads and partial UTF-8.
pub fn decode_escapes(s: &str) -> Vec<u8> {
    let raw = s.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;

    while i < raw.len() {
        if raw[i] == b'\\' && i + 3 < raw.len() {
            let d = &raw[i + 1..i + 4];
            if d.iter().all(|c| (b'0'..=b'7').contains(c)) {
                let v = (d[0] - b'0') as u32 * 64 + (d[1] - b'0') as u32 * 8 + (d[2] - b'0') as u32;
                if v <= 0xff {
                    out.push(v as u8);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_output_with_pane_id() {
        let n = parse_line("%output %12 hello").unwrap();
        assert_eq!(
            n,
            Notification::Output { pane_id: "%12".into(), bytes: b"hello".to_vec() }
        );
    }

    #[test]
    fn decodes_octal_escapes_byte_exactly() {
        // ESC [ 3 1 m  -> tmux sends ESC as \033
        let n = parse_line("%output %1 \\033[31mred").unwrap();
        match n {
            Notification::Output { bytes, .. } => {
                assert_eq!(bytes, b"\x1b[31mred");
            }
            other => panic!("expected output, got {other:?}"),
        }
    }

    #[test]
    fn preserves_non_utf8_bytes() {
        // \377 is 0xFF, which is not valid UTF-8 on its own.
        assert_eq!(decode_escapes("\\377"), vec![0xff]);
    }

    #[test]
    fn leaves_a_lone_backslash_alone() {
        assert_eq!(decode_escapes("a\\b"), b"a\\b".to_vec());
        assert_eq!(decode_escapes("\\99"), b"\\99".to_vec());
    }

    #[test]
    fn structural_notifications_are_flagged() {
        assert!(parse_line("%window-close @3").unwrap().is_structural());
        assert!(parse_line("%unlinked-window-close @3").unwrap().is_structural());
        assert!(parse_line("%exit").unwrap().is_structural());
        assert!(!parse_line("%output %1 x").unwrap().is_structural());
        assert!(!parse_line("%begin 123 0 1").unwrap().is_structural());
    }

    #[test]
    fn window_close_carries_the_stable_window_id() {
        assert_eq!(
            parse_line("%window-close @7").unwrap(),
            Notification::WindowClose { window_id: "@7".into() }
        );
    }

    #[test]
    fn unrecognized_notification_is_inert() {
        let n = parse_line("%session-renamed foo").unwrap();
        assert!(!n.is_structural(), "an unknown notification must not imply a change");
    }

    #[test]
    fn non_notification_lines_are_not_parsed() {
        assert!(parse_line("plain command output").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn error_lines_are_surfaced() {
        assert_eq!(
            parse_line("%error no such window").unwrap(),
            Notification::Error { message: "no such window".into() }
        );
    }
}
