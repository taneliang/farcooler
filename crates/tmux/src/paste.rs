//! Typing text into a pane as literal characters (ov-632).
//!
//! `send_bytes_hex` hands tmux one argument per byte (`send-keys -H`), the
//! real input path for a terminal client. ov-627 saw cursor-agent 2026.09.26
//! drop a paste sent that way; ov-632 could not reproduce the drop (2026.09.23,
//! .26 and 2026.10.16, idle and after a turn, under CPU load). Cursor's paste
//! is delivered here as `send-keys -l` of the bracketed text instead because
//! that is the form measured to land, as a defensive choice. The caller
//! chooses by the agent in the pane (`farcooler_daemon::pastes::plan_paste`);
//! nothing here knows one agent from another.
//!
//! `-l` passes each character through tmux's key encoder where `-H` writes
//! raw bytes, so a pane that turned on extended keys could see the bracket's
//! ESC encoded differently. That fails closed (the text arrives unbracketed
//! and the read-back does not match); re-measure both methods on each new
//! cursor version.

use farcooler_core::{DomainError, Result};

use crate::server::TmuxServer;

/// The most bytes of text one `send-keys -l` carries: well under the 16 KB
/// tmux 3.4 accepts of a command's arguments (ov-349), and one argument, so
/// 3.7's limit of 1,000 never applies.
pub const LITERAL_RUN: usize = 8_000;

/// `text` in runs of at most `LITERAL_RUN` bytes, each ending on a character
/// boundary so no character is cut in two, and no run but the last ending on
/// `;`: tmux reads an argument's trailing `;` as a command separator and
/// drops it, which would lose a character of the text. A run of nothing but
/// `;` is lengthened to include the next character that isn't one.
fn runs(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(LITERAL_RUN);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        if end < rest.len() {
            while end > 1 && rest[..end].ends_with(';') {
                end -= 1;
            }
            if rest[..end].ends_with(';') {
                end = match rest[end..].find(|c| c != ';') {
                    Some(at) => end + at + rest[end + at..].chars().next().map_or(0, char::len_utf8),
                    None => rest.len(),
                };
            }
        }
        let (run, tail) = rest.split_at(end);
        out.push(run);
        rest = tail;
    }
    out
}

impl TmuxServer {
    /// Type `text`, escape bytes and all, into a pane as `send-keys -l`, one
    /// command per run of `LITERAL_RUN` bytes, in order. `--` ends the
    /// options, so a run that starts with `-` is text.
    pub async fn send_literal(&self, pane_id: &str, text: &str) -> Result<()> {
        for run in runs(text) {
            let out = self.run(&["send-keys", "-t", pane_id, "-l", "--", run]).await?;
            if !out.ok() {
                tracing::warn!(stderr = %out.stderr, "send-keys -l failed");
                return Err(DomainError::TmuxUnavailable);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use uuid::Uuid;

    use super::*;

    /// A server whose tmux is a script that records each call's arguments,
    /// NUL separated, each call ended by a SOH (arguments hold newlines).
    fn recorder() -> (TmuxServer, std::path::PathBuf) {
        let dir = farcooler_core::scratch::dir("send-literal");
        let log = dir.join("calls");
        let program = dir.join("tmux");
        std::fs::write(&program, format!("#!/bin/sh\nprintf '%s\\0' \"$@\" >> '{}'\nprintf '\\001' >> '{0}'\n", log.display())).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        (TmuxServer::in_dir(&format!("lit-{}", Uuid::now_v7().simple()), Uuid::now_v7(), &dir).with_program(program), log)
    }

    /// Each recorded call without the `-L <socket> -f <config>` the server
    /// puts first.
    fn calls(log: &std::path::Path) -> Vec<Vec<String>> {
        std::fs::read_to_string(log)
            .unwrap()
            .split('\u{1}')
            .filter(|call| !call.is_empty())
            .map(|l| l.split('\0').skip(4).filter(|a| !a.is_empty()).map(str::to_string).collect())
            .collect()
    }

    /// The paste is `send-keys -l`, with the bracketed text as one argument
    /// and never a `-H`: the form cursor-agent drops (ov-632).
    #[tokio::test]
    async fn a_literal_paste_is_one_send_keys_l_and_never_h() {
        let (server, log) = recorder();
        let text = "\x1b[200~fix the\nflaky test\x1b[201~";
        server.send_literal("%3", text).await.unwrap();
        assert_eq!(calls(&log), vec![vec!["send-keys", "-t", "%3", "-l", "--", text]]);
    }

    /// A long text goes in several commands, in order, none cut inside a
    /// character, and together exactly the text.
    #[tokio::test]
    async fn a_long_literal_paste_is_split_on_character_boundaries() {
        let (server, log) = recorder();
        let text = format!("-{}", "é".repeat(LITERAL_RUN));
        server.send_literal("%3", &text).await.unwrap();
        let sent = calls(&log);
        assert!(sent.len() >= 2, "{} calls", sent.len());
        assert!(sent.iter().all(|c| c[..5] == ["send-keys", "-t", "%3", "-l", "--"] && c[5].len() <= LITERAL_RUN));
        assert_eq!(sent.iter().map(|c| c[5].as_str()).collect::<String>(), text);
    }

    /// A run that would end on `;` ends before it instead, so tmux's
    /// trailing-`;` rule drops nothing (review 1, L1).
    #[test]
    fn no_run_but_the_last_ends_on_a_semicolon() {
        let text = format!("{}{}tail", "a".repeat(LITERAL_RUN - 1), ";;;");
        let r = runs(&text);
        assert_eq!(r.concat(), text);
        assert!(r[..r.len() - 1].iter().all(|run| !run.ends_with(';')), "{:?}", r.iter().map(|x| x.len()).collect::<Vec<_>>());
        let all = ";".repeat(LITERAL_RUN * 2) + "x";
        let r = runs(&all);
        assert_eq!(r.concat(), all);
        assert!(r[..r.len() - 1].iter().all(|run| !run.ends_with(';')));
    }

    #[test]
    fn runs_cover_the_text_whole() {
        assert!(runs("").is_empty());
        let text = "日本語".repeat(LITERAL_RUN);
        assert_eq!(runs(&text).concat(), text);
        assert!(runs(&text).iter().all(|r| r.len() <= LITERAL_RUN));
    }
}
