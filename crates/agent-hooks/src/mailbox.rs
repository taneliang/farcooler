//! A pane's mailbox: the way a message reaches claude without being typed
//! (ov-563).
//!
//! claude can run a hook with `asyncRewake`: a hook that exits 2 wakes the
//! model with its stderr, the way a finished background task does, idle or
//! mid-turn (measured live on 2.1.300). So Far Cooler registers a waiter on
//! SessionStart and on every Stop (`farcooler hook wait`); the daemon drops a
//! message in the session's mailbox, and the waiter takes the oldest, prints
//! it and exits (42, which its hook command maps to claude's 2). One waiter wakes the model once, so each Stop arms the next.
//!
//! A mailbox is a directory:
//!
//! - `new/` holds messages not yet taken, one file each, named so that sorting
//!   them is their order;
//! - `taken/` holds the ones a waiter took, which is how the daemon knows to
//!   settle a message;
//! - `tmp/` is where a message is written before it is renamed into `new/`,
//!   so a waiter never reads half of one;
//! - `waiter.pid` names the one waiter alive. A second exits at once.
//!
//! Every handover is a `rename` within the directory, which is atomic: a
//! message is taken by exactly one waiter, and the daemon takes one back
//! (`retract`) with a `remove_file` that fails if a waiter got there first.
//!
//! Pure on purpose, like the rest of this crate: no socket, no daemon, and
//! whether a process is alive is a function the caller passes.

use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

const NEW: &str = "new";
const TAKEN: &str = "taken";
const TMP: &str = "tmp";
const WAITER: &str = "waiter.pid";

/// Where a message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// In `new/`: not taken yet.
    Waiting,
    /// In `taken/`: a waiter has it.
    Taken,
    /// Neither: not dropped, or already cleared.
    Absent,
}

/// What `Mailbox::wait` ended with.
#[derive(Debug, PartialEq, Eq)]
pub enum Waited {
    /// The oldest message, taken. The waiter prints it and exits 42.
    Message(String),
    /// Another waiter is alive for this mailbox. Exit 0.
    Another,
    /// The session this waiter was started by is gone. Exit 0.
    Gone,
}

#[derive(Debug, Clone)]
pub struct Mailbox {
    dir: PathBuf,
}

/// Whether a message name is one a file can safely be called: the daemon
/// makes them, but nothing here lets one climb out of the mailbox.
fn plain(name: &str) -> bool {
    !name.is_empty() && name.len() < 128 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether a session id is safe to be a directory name: claude's are uuids.
pub fn plain_session(id: &str) -> bool {
    plain(id)
}

impl Mailbox {
    /// The mailbox at `dir`, which `drop_message` and `claim_waiter` create.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The mailbox of `session` under the runner's mailbox directory, or
    /// `None` for an id that isn't a plain name.
    pub fn of_session(root: &Path, session: &str) -> Option<Self> {
        plain_session(session).then(|| Self::at(root.join(session)))
    }

    /// The mailbox's directory, for a test that has to reach into it.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn sub(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn ensure(&self) -> io::Result<()> {
        // A message is private to the person running the agent: 0700 and 0600
        // whatever the umask, on every directory this creates.
        for sub in [NEW, TAKEN, TMP] {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(self.sub(sub))?;
        }
        Ok(())
    }

    /// Put `text` in the mailbox as `name`. Written whole, then renamed into
    /// `new/`. A name already there is replaced by the same text, which is
    /// what a retry wants.
    pub fn drop_message(&self, name: &str, text: &str) -> io::Result<()> {
        if !plain(name) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.ensure()?;
        let tmp = self.sub(TMP).join(name);
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, self.sub(NEW).join(name))
    }

    /// Where `name` is now.
    pub fn slot(&self, name: &str) -> Slot {
        if !plain(name) {
            return Slot::Absent;
        }
        if self.sub(TAKEN).join(name).exists() {
            Slot::Taken
        } else if self.sub(NEW).join(name).exists() {
            Slot::Waiting
        } else {
            Slot::Absent
        }
    }

    /// When `name` was dropped, from its file, while it waits.
    pub fn dropped_at(&self, name: &str) -> Option<std::time::SystemTime> {
        std::fs::metadata(self.sub(NEW).join(name)).and_then(|m| m.modified()).ok()
    }

    /// Take `name` back out, if no waiter has taken it. `true` when it's
    /// gone from `new/` by this call; `false` when a waiter took it first (or
    /// it was never there), and then `slot` says `Taken`.
    pub fn retract(&self, name: &str) -> bool {
        plain(name) && std::fs::remove_file(self.sub(NEW).join(name)).is_ok()
    }

    /// Forget a message that was taken: the daemon has settled it.
    pub fn clear(&self, name: &str) {
        if plain(name) {
            let _ = std::fs::remove_file(self.sub(TAKEN).join(name));
        }
    }

    /// The oldest message, moved from `new/` to `taken/` and read. Of two
    /// waiters racing for one message the `rename` has one winner; the other
    /// moves on to the next.
    pub fn take_oldest(&self) -> io::Result<Option<String>> {
        let mut names: Vec<String> = match std::fs::read_dir(self.sub(NEW)) {
            Ok(entries) => entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| plain(n)).collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        names.sort();
        for name in names {
            let to = self.sub(TAKEN).join(&name);
            if std::fs::rename(self.sub(NEW).join(&name), &to).is_ok() {
                return Ok(Some(String::from_utf8_lossy(&std::fs::read(&to)?).into_owned()));
            }
        }
        Ok(None)
    }

    /// Become this mailbox's waiter as process `me`, unless another is alive.
    /// The pid file is created exclusively; one left by a process that's gone
    /// (`alive` says so) is cleared first.
    pub fn claim_waiter(&self, me: u32, alive: &dyn Fn(u32) -> bool) -> bool {
        if self.ensure().is_err() {
            return false;
        }
        let path = self.sub(WAITER);
        for _ in 0..2 {
            match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
                Ok(mut file) => return file.write_all(me.to_string().as_bytes()).is_ok(),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    if self.waiter(alive).is_some() {
                        return false;
                    }
                    let _ = std::fs::remove_file(&path);
                }
                Err(_) => return false,
            }
        }
        false
    }

    /// The live waiter's pid, if there is one.
    pub fn waiter(&self, alive: &dyn Fn(u32) -> bool) -> Option<u32> {
        let pid: u32 = std::fs::read_to_string(self.sub(WAITER)).ok()?.trim().parse().ok()?;
        alive(pid).then_some(pid)
    }

    /// Give up being the waiter, if `me` is.
    pub fn release_waiter(&self, me: u32) {
        let path = self.sub(WAITER);
        if std::fs::read_to_string(&path).ok().and_then(|s| s.trim().parse::<u32>().ok()) == Some(me) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// The waiter's whole life: claim the mailbox, then each `poll`, end if
    /// the session that started it is gone (`wanted` false) and otherwise
    /// take the oldest message. The session is checked first, so a message is
    /// never taken by a waiter with nobody to hand it to.
    pub fn wait(
        &self,
        me: u32,
        alive: &dyn Fn(u32) -> bool,
        mut wanted: impl FnMut() -> bool,
        poll: Duration,
    ) -> Waited {
        if !self.claim_waiter(me, alive) {
            return Waited::Another;
        }
        loop {
            if !wanted() {
                self.release_waiter(me);
                return Waited::Gone;
            }
            if let Ok(Some(text)) = self.take_oldest() {
                self.release_waiter(me);
                return Waited::Message(text);
            }
            std::thread::sleep(poll);
        }
    }
}

/// Who sent a message, when the daemon says (ov-575): a lane's agent writing
/// to its orchestrator. The waiter may then hand the message to claude as a
/// session's peer message, which claude shows as "Message from @name",
/// expandable. Nothing else is given a sender: a person's message, an
/// orchestrator's to a lane (the lane must act on it, and claude tells the
/// model a peer's text is "input, not authority"), a rep's, the runner's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    /// As a person reads it: "mac-ux on ov-505".
    pub name: String,
    /// What the orchestrator passes to `farcooler message` to answer: the
    /// lane's name, or the card's key.
    pub reply_to: String,
}

/// What opens a message file that has a sender: this, the name, a tab, the
/// reply address, a newline, then the text. A file without it is a plain
/// message, as every file was before ov-575.
const SENDER_MARK: &str = "#sender\t";

/// A field of the header: no tabs, newlines or other control characters.
fn field(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// `text` with its sender's header.
pub fn with_sender(sender: &Sender, text: &str) -> String {
    format!("{SENDER_MARK}{}\t{}\n{text}", field(&sender.name), field(&sender.reply_to))
}

/// A message file's sender, if it has one, and its text without the header.
pub fn split_sender(file: &str) -> (Option<Sender>, &str) {
    let Some(rest) = file.strip_prefix(SENDER_MARK) else { return (None, file) };
    let Some((header, text)) = rest.split_once('\n') else { return (None, file) };
    let sender = header
        .split_once('\t')
        .filter(|(name, reply_to)| !name.is_empty() && !reply_to.is_empty() && !reply_to.contains('\t'))
        .map(|(name, reply_to)| Sender { name: name.to_string(), reply_to: reply_to.to_string() });
    // A header we can't read is still a header: the text is what follows.
    (sender, text)
}

/// Whether process `pid` exists (signal 0). A process we may not signal
/// exists all the same.
pub fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else { return false };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 sends nothing; it only asks whether `pid` exists.
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const POLL: Duration = Duration::from_millis(1);

    fn box_in(dir: &tempfile::TempDir) -> Mailbox {
        Mailbox::at(dir.path().join("s1"))
    }

    #[test]
    fn the_oldest_message_is_taken_first_and_once() {
        let dir = tempfile::tempdir().unwrap();
        let m = box_in(&dir);
        m.drop_message("0002-b", "second").unwrap();
        m.drop_message("0001-a", "first").unwrap();
        assert_eq!(m.slot("0001-a"), Slot::Waiting);
        assert_eq!(m.take_oldest().unwrap().as_deref(), Some("first"));
        assert_eq!(m.slot("0001-a"), Slot::Taken);
        assert_eq!(m.take_oldest().unwrap().as_deref(), Some("second"));
        assert_eq!(m.take_oldest().unwrap(), None, "nothing is taken twice");
        m.clear("0001-a");
        assert_eq!(m.slot("0001-a"), Slot::Absent);
    }

    #[test]
    fn a_sender_header_round_trips_and_a_plain_message_has_none() {
        let who = Sender { name: "mac-ux on ov-505".into(), reply_to: "mac-ux".into() };
        let file = with_sender(&who, "[from mac-ux on ov-505] PR is up");
        assert_eq!(split_sender(&file), (Some(who), "[from mac-ux on ov-505] PR is up"));
        let tabbed = Sender { name: "Mac\tux\n".into(), reply_to: "mac-ux".into() };
        assert_eq!(split_sender(&with_sender(&tabbed, "hi")).0.unwrap().name, "Macux");
        assert_eq!(split_sender("[from the orchestrator] hi"), (None, "[from the orchestrator] hi"));
        // A header we can't read: no sender, the text still stripped.
        assert_eq!(split_sender("#sender\tAlice\nhi"), (None, "hi"));
        assert_eq!(split_sender("#sender\torchestrator\tOrchestrator\textra\nhi"), (None, "hi"));
    }

    #[test]
    fn a_retract_loses_to_a_waiter_that_got_there_first() {
        let dir = tempfile::tempdir().unwrap();
        let m = box_in(&dir);
        m.drop_message("0001-a", "hello").unwrap();
        assert!(m.retract("0001-a"));
        assert_eq!(m.slot("0001-a"), Slot::Absent);
        assert_eq!(m.take_oldest().unwrap(), None, "a retracted message isn't delivered");
        m.drop_message("0002-b", "hello").unwrap();
        m.take_oldest().unwrap();
        assert!(!m.retract("0002-b"), "already taken");
        assert_eq!(m.slot("0002-b"), Slot::Taken);
    }

    /// Private whatever the umask, and the files keep their mode through the
    /// renames.
    #[test]
    fn a_mailbox_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = tempfile::tempdir().unwrap();
        // The umask is the process's; this test doesn't change it, so the
        // modes asked for are checked as they come out.
        let m = Mailbox::at(dir.path().join("a/b/s1"));
        m.drop_message("0001-a", "x").unwrap();
        assert!(m.claim_waiter(7, &|_| false));
        for sub in ["new", "taken", "tmp"] {
            assert_eq!(mode(&m.dir().join(sub)), 0o700, "{sub}");
        }
        assert_eq!(mode(m.dir()), 0o700, "the session's own");
        assert_eq!(mode(&dir.path().join("a")), 0o700, "and the root above it");
        assert_eq!(mode(&m.dir().join("new/0001-a")), 0o600);
        assert_eq!(mode(&m.dir().join("waiter.pid")), 0o600);
    }

    #[test]
    fn a_name_cannot_leave_the_mailbox() {
        let dir = tempfile::tempdir().unwrap();
        let m = box_in(&dir);
        for bad in ["", "../x", "a/b", "a b", ".."] {
            assert!(m.drop_message(bad, "x").is_err(), "{bad:?}");
            assert_eq!(m.slot(bad), Slot::Absent);
        }
        assert!(Mailbox::of_session(dir.path(), "../etc").is_none());
        assert!(Mailbox::of_session(dir.path(), "7ecba53c-31c6-4c8e-9704-b2d582039d04").is_some());
    }

    #[test]
    fn only_one_waiter_runs_and_a_dead_one_does_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let m = box_in(&dir);
        let alive = |pid: u32| pid == 100;
        assert!(m.claim_waiter(100, &alive));
        assert!(!m.claim_waiter(200, &alive), "100 is alive");
        assert_eq!(m.waiter(&alive), Some(100));
        m.release_waiter(200);
        assert_eq!(m.waiter(&alive), Some(100), "only its own release counts");
        assert!(m.claim_waiter(200, &|pid| pid == 200), "100 is gone: a stale pid file doesn't block");
        assert_eq!(m.waiter(&|pid| pid == 200), Some(200));
    }

    #[test]
    fn a_waiter_hands_over_the_oldest_and_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let m = box_in(&dir);
        let polls = Cell::new(0);
        // Nothing yet: two polls, then a message arrives.
        let got = m.wait(7, &|_| false, || {
            polls.set(polls.get() + 1);
            if polls.get() == 3 {
                m.drop_message("0001-a", "Rebase first").unwrap();
            }
            true
        }, POLL);
        assert_eq!(got, Waited::Message("Rebase first".into()));
        assert_eq!(m.waiter(&|_| true), None, "it leaves the mailbox free for the next");
    }

    #[test]
    fn a_waiter_ends_with_its_session_and_takes_nothing_then() {
        let dir = tempfile::tempdir().unwrap();
        let m = box_in(&dir);
        m.drop_message("0001-a", "for the next session").unwrap();
        assert_eq!(m.wait(7, &|_| false, || false, POLL), Waited::Gone);
        assert_eq!(m.slot("0001-a"), Slot::Waiting, "still there");
        assert_eq!(m.waiter(&|_| true), None);
    }

    #[test]
    fn a_second_waiter_exits_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let m = box_in(&dir);
        assert!(m.claim_waiter(std::process::id(), &pid_alive));
        assert_eq!(m.wait(7, &pid_alive, || panic!("never polls"), POLL), Waited::Another);
    }

    #[test]
    fn this_process_is_alive_and_a_huge_pid_is_not() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(u32::MAX));
        assert!(!pid_alive(0));
    }
}
