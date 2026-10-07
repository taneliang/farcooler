//! One `list-panes` per tick.
//!
//! The daemon samples once a second, and every tmux command is a process. This
//! is the one read that answers everything the inventory and the sweep for
//! unfinished opens need, so that neither pays for a spawn of its own: the
//! tagged panes with their geometry, title and `ScreenStamp`, and the panes an
//! open started and never tagged.
//!
//! It also spends nothing while there is no tmux server at all. tmux names the
//! socket it could not reach in its own error, either missing or left behind by
//! a server that exited (tmux does not unlink it). Once a read has seen that,
//! later reads stat the socket instead of spawning, and the first one that
//! finds it missing, or a different file, spawns again: a new server binds a
//! new socket. The path is tmux's, read back from its message; this does not
//! guess where tmux keeps sockets.

use std::path::PathBuf;

use farcooler_core::{DomainError, Result, inventory::TaggedPane, tags};

use crate::server::{SESSION_NAME, TmuxServer};
use crate::windows::{OPENING_MARK, UnfinishedOpen, parse_pane_line};

/// Where the pane's title sits in a line: after every fixed-width field, because
/// a title is user text and may hold a tab.
pub(crate) const TITLE_FIELD: usize = 25;

/// What one `list-panes` found.
#[derive(Debug, Default)]
pub struct PaneRead {
    /// The panes carrying this install's exact tags.
    pub panes: Vec<TaggedPane>,
    /// Panes an open started and never tagged. See `TmuxServer::unfinished_opens`.
    pub unfinished: Vec<UnfinishedOpen>,
}

/// The format of the one read, a field per `parse_pane_line` index.
///
/// Fields 0 to 17 are what the inventory has always read. 18 to 22 are the
/// `ScreenStamp` (`window_activity`, `history_size`, the cursor, the pid). 23 and
/// 24 are for the unfinished-open sweep: the session, and whether the pane's
/// start command carries `OPENING_MARK`, asked of tmux as a pattern match so
/// that the command itself, which may hold a tab, never travels. `##` is how a
/// format spells a literal `#`.
fn list_format() -> String {
    format!(
        "#{{pane_id}}\t#{{window_id}}\t#{{pane_width}}\t#{{pane_height}}\t#{{{}}}\t#{{{}}}\t#{{{}}}\t#{{{}}}\t#{{pane_dead}}\t#{{pane_dead_status}}\t#{{pane_current_command}}\t#{{pane_left}}\t#{{pane_top}}\t#{{window_active}}\t#{{pane_active}}\t#{{window_zoomed_flag}}\t#{{pane_tty}}\t#{{pane_dead_signal}}\t#{{window_activity}}\t#{{history_size}}\t#{{cursor_x}}\t#{{cursor_y}}\t#{{pane_pid}}\t#{{session_name}}\t#{{m:*{}*,#{{pane_start_command}}}}\t#{{pane_title}}",
        tags::DAEMON_ID,
        tags::WORKTREE_ID,
        tags::TERMINAL_ID,
        tags::SCHEMA_VERSION,
        OPENING_MARK.replace('#', "##"),
    )
}

/// The unfinished open on `line`, if it is one: in `SESSION_NAME`, no terminal
/// id, and the opening mark in its start command.
fn parse_unfinished(line: &str) -> Option<UnfinishedOpen> {
    let f: Vec<&str> = line.split('\t').collect();
    let (pane, tag, pid) = (f.first()?, f.get(6)?, f.get(22)?);
    let ours = f.get(23)? == &SESSION_NAME && pane.starts_with('%');
    let marked = f.get(24)?.trim() == "1";
    (ours && tag.trim().is_empty() && marked).then(|| UnfinishedOpen {
        pane_id: pane.trim().to_string(),
        pid: pid.trim().parse().unwrap_or(0),
    })
}

/// A socket tmux found nothing listening on, and what it was when it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Silent {
    path: PathBuf,
    /// The file there then: `None` for no file at all, else enough of it to tell
    /// it from the file a new server would bind in its place.
    file: Option<FileIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    inode: u64,
    changed: (i64, i64),
    modified: (i64, i64),
}

fn identity_of(path: &std::path::Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some(FileIdentity {
        inode: meta.ino(),
        changed: (meta.ctime(), meta.ctime_nsec()),
        modified: (meta.mtime(), meta.mtime_nsec()),
    })
}

/// The socket tmux says nothing answers on, from its own message:
/// `error connecting to /tmp/tmux-501/x (No such file or directory)` for a
/// socket that is not there, and `no server running on /tmp/tmux-501/x` (or the
/// same `error connecting` with `Connection refused`) for one a server left
/// behind. `None` for any other failure.
fn silent_socket(stderr: &str) -> Option<Silent> {
    let line = stderr.lines().next()?.trim_end();
    let path = if let Some(rest) = line.strip_prefix("no server running on ") {
        rest
    } else {
        let rest = line.strip_prefix("error connecting to ")?;
        rest.strip_suffix(" (No such file or directory)").or_else(|| rest.strip_suffix(" (Connection refused)"))?
    };
    let path = std::path::Path::new(path.trim());
    path.is_absolute().then(|| Silent { path: path.to_path_buf(), file: identity_of(path) })
}

impl TmuxServer {
    /// One `list-panes`, as tmux answers it, or nothing at all when no server
    /// is running and the last read said so.
    pub(crate) async fn read_panes_once(&self) -> Result<PaneRead> {
        if self.server_is_known_absent() {
            return Ok(PaneRead::default());
        }
        let out = self.run(&["list-panes", "-a", "-F", &list_format()]).await?;
        if !out.ok() {
            // No server or no session is not an error: it means nothing is alive.
            if out.stderr.contains("no server running")
                || out.stderr.contains("no current session")
                || out.stderr.contains("error connecting")
            {
                *self.absent_socket.lock().expect("absent socket lock") = silent_socket(&out.stderr);
                return Ok(PaneRead::default());
            }
            tracing::warn!(stderr = %out.stderr, "list-panes failed");
            return Err(DomainError::TmuxUnavailable);
        }
        *self.absent_socket.lock().expect("absent socket lock") = None;

        let panes: Vec<TaggedPane> = out.stdout.lines().filter_map(parse_pane_line).collect();

        // Lines arrived and none of them parsed.
        //
        // Worth saying out loud because it is indistinguishable from "nothing is
        // running" everywhere downstream: the snapshot is empty either way,
        // `derive_terminal` reports every terminal `Lost`, and the app looks
        // broken with nothing anywhere saying why. That is exactly how the
        // missing-locale bug hid — tmux sanitized the tab delimiter to `_`, every
        // line was dropped here in silence, and the symptom surfaced three layers
        // away as panes that never leave `starting`.
        if panes.is_empty() && out.stdout.lines().any(|l| !l.trim().is_empty()) {
            tracing::warn!(
                lines = out.stdout.lines().count(),
                "tmux listed panes but none could be parsed; the delimiter or the tags have changed"
            );
        }
        Ok(PaneRead { panes, unfinished: out.stdout.lines().filter_map(parse_unfinished).collect() })
    }

    /// Whether the socket the last read found silent is still the same silent
    /// socket, which is "no server" without asking one.
    ///
    /// A server that starts binds its socket before it answers, so a socket
    /// that has appeared, or been replaced, is never skipped. Not used with a
    /// test's named program, whose "tmux" is not a server to be absent.
    fn server_is_known_absent(&self) -> bool {
        if self.has_program() {
            return false;
        }
        let known = self.absent_socket.lock().expect("absent socket lock").clone();
        known.is_some_and(|silent| identity_of(&silent.path) == silent.file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_silent_socket_is_the_one_tmux_names() {
        let gone = silent_socket("error connecting to /private/tmp/tmux-502/farcooler-01a1 (No such file or directory)\n");
        assert_eq!(
            gone,
            Some(Silent { path: PathBuf::from("/private/tmp/tmux-502/farcooler-01a1"), file: None }),
            "a path nothing is at"
        );
        let left = silent_socket("no server running on /private/tmp/tmux-502/farcooler-01a1\n").expect("left behind");
        assert_eq!(left.path, PathBuf::from("/private/tmp/tmux-502/farcooler-01a1"));
        let refused = silent_socket("error connecting to /tmp/tmux-1/x (Connection refused)\n").expect("refused");
        assert_eq!(refused.path, PathBuf::from("/tmp/tmux-1/x"));
    }

    #[test]
    fn any_other_failure_is_not_a_silent_socket() {
        assert_eq!(silent_socket("error connecting to /tmp/tmux-1/x (Permission denied)\n"), None);
        assert_eq!(silent_socket("error connecting to relative (No such file or directory)"), None);
        assert_eq!(silent_socket("unknown command: list-pane\n"), None);
        assert_eq!(silent_socket(""), None);
    }

    #[test]
    fn a_replaced_file_is_not_the_one_that_was_silent() {
        let dir = std::env::temp_dir().join(format!("fc-silent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sock");
        std::fs::write(&path, "x").unwrap();
        let then = identity_of(&path);
        assert!(then.is_some());
        assert_eq!(identity_of(&path), then, "the same file is the same");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(identity_of(&path), None, "a file that went away is not it");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "x").unwrap();
        assert_ne!(identity_of(&path), then, "a file bound in its place is not it");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unfinished_open_needs_the_session_the_mark_and_no_terminal_id() {
        let line = |session: &str, tag: &str, marked: &str| {
            format!("%4\t@1\t80\t24\t\t\t{tag}\t\t\t\tsh\t0\t0\t1\t1\t0\t/dev/ttys001\t\t1790000000\t0\t0\t0\t4242\t{session}\t{marked}\t")
        };
        let mine = parse_unfinished(&line("farcooler", "", "1")).expect("an unfinished open");
        assert_eq!((mine.pane_id.as_str(), mine.pid), ("%4", 4242));
        assert_eq!(parse_unfinished(&line("farcooler", "", "0")), None, "no opening mark");
        assert_eq!(parse_unfinished(&line("other", "", "1")), None, "someone else's session");
        assert_eq!(parse_unfinished(&line("farcooler", "abc", "1")), None, "it was tagged");
    }
}
