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
//! a server that exited (tmux does not unlink it). Once reads have seen that,
//! later reads stat the socket instead of spawning, and the first one that
//! finds it missing, or a different file, spawns again: a new server binds a
//! new socket. The path is tmux's, read back from its message; this does not
//! guess where tmux keeps sockets.
//!
//! "Silent" is believed only of a socket that held still. A failure is stored
//! untrusted, and trusted only when the same path had the same identity before
//! the spawn as after the error: a server that binds between the client's
//! failure and the stat would otherwise be recorded as the silent one, and
//! skipped for as long as it lived (found in review, 7 of 800 races). Reads
//! that overlap are ordered by when they started, so a slow failure never
//! overwrites a newer answer, and a spawn is made anyway every `BACKSTOP`.

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
    /// Whether the same file was there before the spawn too. Only a trusted
    /// socket is skipped.
    trusted: bool,
}

/// How long a trusted silent socket is believed without a spawn.
const BACKSTOP: std::time::Duration = std::time::Duration::from_secs(20);

/// What the server handle has learned about its socket, shared by its clones.
#[derive(Debug, Default)]
pub(crate) struct Learned {
    /// Reads started so far.
    started: u64,
    /// The newest read whose answer has been kept.
    applied: u64,
    silent: Option<Silent>,
    spawned: Option<std::time::Instant>,
}

/// One read's place in line, and what its socket looked like before it spawned.
#[derive(Debug)]
pub(crate) struct Ticket {
    number: u64,
    before: Option<(PathBuf, Option<FileIdentity>)>,
}

impl Learned {
    /// Whether to skip the spawn: the socket is trusted silent, is still the
    /// very file that was, and a spawn was made within `BACKSTOP`.
    fn skip(&self, now: std::time::Instant, stat: impl Fn(&std::path::Path) -> Option<FileIdentity>) -> bool {
        let fresh = self.spawned.is_some_and(|at| now.duration_since(at) < BACKSTOP);
        fresh && self.silent.as_ref().is_some_and(|s| s.trusted && stat(&s.path) == s.file)
    }

    /// Take a place in line for a spawn made now.
    fn begin(&mut self, now: std::time::Instant, stat: impl Fn(&std::path::Path) -> Option<FileIdentity>) -> Ticket {
        self.started += 1;
        self.spawned = Some(now);
        let before = self.silent.as_ref().map(|s| (s.path.clone(), stat(&s.path)));
        Ticket { number: self.started, before }
    }

    /// The spawn found `path` silent, as `after` the error.
    fn failed(&mut self, ticket: Ticket, path: PathBuf, after: Option<FileIdentity>) {
        if ticket.number <= self.applied {
            return;
        }
        self.applied = ticket.number;
        let trusted = ticket.before == Some((path.clone(), after));
        self.silent = Some(Silent { path, file: after, trusted });
    }

    /// The spawn found a server.
    fn answered(&mut self, ticket: Ticket) {
        if ticket.number > self.applied {
            self.applied = ticket.number;
            self.silent = None;
        }
    }
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
fn silent_socket(stderr: &str) -> Option<PathBuf> {
    let line = stderr.lines().next()?.trim_end();
    let path = if let Some(rest) = line.strip_prefix("no server running on ") {
        rest
    } else {
        let rest = line.strip_prefix("error connecting to ")?;
        rest.strip_suffix(" (No such file or directory)").or_else(|| rest.strip_suffix(" (Connection refused)"))?
    };
    let path = std::path::Path::new(path.trim());
    path.is_absolute().then(|| path.to_path_buf())
}

impl TmuxServer {
    /// One `list-panes`, as tmux answers it, or nothing at all when no server
    /// is running and the last read said so.
    pub(crate) async fn read_panes_once(&self) -> Result<PaneRead> {
        let now = std::time::Instant::now();
        if !self.has_program() && self.absent_socket.lock().expect("absent socket lock").skip(now, identity_of) {
            // Still a place where other tasks run, as the spawn it replaces was.
            // The seat's read of tmux is ordered against a start that confirms
            // while it is out (`an_orchestrator_confirmed_during_the_seats_read_keeps_it`),
            // and a read that never awaits is a read nothing can interleave with.
            tokio::task::yield_now().await;
            return Ok(PaneRead::default());
        }
        let ticket = self.absent_socket.lock().expect("absent socket lock").begin(now, identity_of);
        let out = self.run(&["list-panes", "-a", "-F", &list_format()]).await?;
        if !out.ok() {
            // No server or no session is not an error: it means nothing is alive.
            if out.stderr.contains("no server running")
                || out.stderr.contains("no current session")
                || out.stderr.contains("error connecting")
            {
                // Judged after the error, which is the moment that matters: see
                // the module note on a socket that held still.
                let silent = silent_socket(&out.stderr);
                let mut learned = self.absent_socket.lock().expect("absent socket lock");
                match silent {
                    Some(path) => {
                        let after = identity_of(&path);
                        learned.failed(ticket, path, after)
                    }
                    None => learned.answered(ticket),
                }
                return Ok(PaneRead::default());
            }
            tracing::warn!(stderr = %out.stderr, "list-panes failed");
            return Err(DomainError::TmuxUnavailable);
        }
        self.absent_socket.lock().expect("absent socket lock").answered(ticket);

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_silent_socket_is_the_one_tmux_names() {
        let gone = silent_socket("error connecting to /private/tmp/tmux-502/farcooler-01a1 (No such file or directory)\n");
        assert_eq!(gone, Some(PathBuf::from("/private/tmp/tmux-502/farcooler-01a1")));
        let left = silent_socket("no server running on /private/tmp/tmux-502/farcooler-01a1\n").expect("left behind");
        assert_eq!(left, PathBuf::from("/private/tmp/tmux-502/farcooler-01a1"));
        let refused = silent_socket("error connecting to /tmp/tmux-1/x (Connection refused)\n").expect("refused");
        assert_eq!(refused, PathBuf::from("/tmp/tmux-1/x"));
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

    fn file(inode: u64) -> Option<FileIdentity> {
        Some(FileIdentity { inode, changed: (1, inode as i64), modified: (1, inode as i64) })
    }

    /// A socket's stat, as a table: what the file is at each path right now.
    fn stat_of(now: Option<FileIdentity>) -> impl Fn(&std::path::Path) -> Option<FileIdentity> {
        move |_| now
    }

    fn sock() -> PathBuf {
        PathBuf::from("/tmp/tmux-1/x")
    }

    /// A failure is believed only the second time, on a file that held still:
    /// the first failure's stat may be a server that bound a moment after the
    /// client gave up (7 of 800 races in review).
    #[test]
    fn a_first_failure_is_not_trusted_and_a_steady_one_is() {
        let t0 = std::time::Instant::now();
        let mut learned = Learned::default();
        let stat = stat_of(file(7));

        assert!(!learned.skip(t0, &stat), "nothing learned yet");
        let first = learned.begin(t0, &stat);
        learned.failed(first, sock(), file(7));
        assert!(!learned.skip(t0, &stat), "one failure on its own is not trusted");

        let second = learned.begin(t0, &stat);
        learned.failed(second, sock(), file(7));
        assert!(learned.skip(t0, &stat), "the same file before and after: believed");
    }

    /// The race: the socket is a different file after the error than before the
    /// spawn, because a server bound in between.
    #[test]
    fn a_socket_that_changed_during_the_spawn_is_not_trusted() {
        let t0 = std::time::Instant::now();
        let mut learned = Learned::default();
        let first = learned.begin(t0, stat_of(None));
        learned.failed(first, sock(), None);
        let second = learned.begin(t0, stat_of(None));
        // The server's own socket is what the stat after the error finds.
        learned.failed(second, sock(), file(9));
        assert!(!learned.skip(t0, stat_of(file(9))), "a live server's socket must not be recorded as silent");
    }

    /// Two reads in flight: the older one's failure arrives last.
    #[test]
    fn a_late_failure_does_not_overwrite_a_newer_answer() {
        let t0 = std::time::Instant::now();
        let stat = stat_of(file(7));
        let mut learned = Learned::default();
        let a = learned.begin(t0, &stat);
        learned.failed(a, sock(), file(7));
        let older = learned.begin(t0, &stat);
        let newer = learned.begin(t0, &stat);
        learned.answered(newer);
        learned.failed(older, sock(), file(7));
        assert!(learned.silent.is_none(), "the newer read found a server, and stands");
        assert!(!learned.skip(t0, &stat));
    }

    /// A trusted socket is still asked about now and then.
    #[test]
    fn a_trusted_socket_is_asked_again_after_the_backstop() {
        let t0 = std::time::Instant::now();
        let stat = stat_of(file(7));
        let mut learned = Learned::default();
        for _ in 0..2 {
            let t = learned.begin(t0, &stat);
            learned.failed(t, sock(), file(7));
        }
        assert!(learned.skip(t0 + BACKSTOP / 2, &stat));
        assert!(!learned.skip(t0 + BACKSTOP + std::time::Duration::from_secs(1), &stat), "a spawn is due");
    }
}
