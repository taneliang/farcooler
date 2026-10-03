//! Read a session log as it grows, without re-reading what has already been
//! seen.
//!
//! The three formats in `docs/agent-session-logs.md` are append-only files
//! that a live agent keeps writing to while Far Cooler is watching. That
//! creates three problems no ordinary line reader has to solve:
//!
//! - a 100 MB claude log must be attached to at its END, not replayed from the
//!   start, or the first read costs minutes and megabytes to learn what the
//!   agent is doing right now — while a SMALL one must be read whole, or the
//!   turn the pane is in the middle of is never seen at all (see
//!   [`READ_FROM_START_BYTES`]);
//! - the last line is frequently half-written, because the writer is still
//!   mid-`write()` when this reads it. A half-written JSON record parsed as a
//!   whole one is a wrong answer, not a parse error — so it must be held until
//!   its newline arrives, not returned early;
//! - claude has been observed writing single lines over 1 MB (a tool result
//!   carrying a file dump). Buffering one of those per pane per tick is a
//!   memory problem nobody asked for, so an oversized line is skipped rather
//!   than held, and skipping must still leave the offset correct for every
//!   line after it.
//!
//! And a fourth, which breaks the premise: cursor's transcript is not
//! append-only. It rewrites its last line when a turn starts -- see
//! [`Tail::read_new_lines`].

use std::fs::File;
use std::hash::{DefaultHasher, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The largest line this stage will hand back whole. The biggest line ever
/// observed in a real log was 1.35 MB; nothing this stage reads is anywhere
/// near 64 KiB, so skipping a line over the cap is lossless in practice and
/// bounded in the worst case.
const MAX_LINE_BYTES: usize = 64 * 1024;

/// How much is pulled from disk per `read` syscall while scanning for
/// newlines. Kept well below `MAX_LINE_BYTES` so a single oversized (or
/// still-growing) line is scanned in bounded steps instead of being pulled
/// into memory in one shot before its size is even known.
const CHUNK_BYTES: usize = 8 * 1024;

/// The largest file this will attach to at its START rather than its end.
///
/// Attaching at the end is what keeps a 108 MB claude log from costing minutes
/// on the first read, and it is also what made a pane's FIRST TURN invisible.
/// Codex is the worst case: it opens its rollout only when the first turn is
/// submitted, so the `task_started` line is already behind the end of the file
/// by the time the join can possibly succeed — and a `Step` arriving with no
/// turn believed is dropped, so the row stayed on stage 1 for the whole of
/// that turn and only recovered from the second one.
///
/// One mebibyte, chosen against the files on a real machine rather than by
/// taste. Of 781 claude session files here, 709 are under it; of 279 codex
/// rollouts, 260 are — so nine in ten sessions are read whole, for a read
/// bounded at a megabyte, once per pane per join, on a blocking thread. The
/// files this excludes miss by three orders of magnitude (108 MB and 60 MB are
/// the largest of each), so the bound is nowhere near the ones it decides
/// about, and no startup can stall on it: 12 panes joining at once is 12 MB of
/// sequential reads in the worst case anyone has.
///
/// What is given up above the bound is exactly what was given up everywhere
/// before this existed: a log already that large has run many turns, so the
/// first one is long gone and there is nothing to be early for. Such a pane
/// attaches at the end and behaves as it always did — it sees the next
/// boundary, not this one.
const READ_FROM_START_BYTES: u64 = 1024 * 1024;

/// How far back from the last line's start the bytes before it are checked,
/// to tell that line rewritten from the whole file replaced.
///
/// Four KiB is a few records in any of the three formats, read in the same
/// `pread` as the last line itself. A replacement that keeps the last 4 KiB
/// before that line byte for byte, and changes something earlier, is not
/// caught; nothing that writes these logs does that.
const BEFORE_BYTES: u64 = 4 * 1024;

/// A position in one session log file, advanced only past complete lines.
///
/// Holds a path, a byte offset, which file the path named when it was last
/// read, where the last line read began, and how far into an oversized line
/// the reader has already skipped. Every hazard in
/// `docs/agent-session-logs.md` — the half-written tail, the oversized line,
/// the file that shrinks, the file that is replaced — is checked against the
/// file on each call, so what is remembered is only ever a shortcut: when the
/// file no longer agrees with it, it is dropped and the state re-derived. A
/// crash or restart loses nothing worse than re-scanning from the last
/// complete line.
pub struct Tail {
    path: PathBuf,
    offset: u64,
    /// The device and inode `path` named on the last read. `None` before the
    /// file exists, and always on a platform without inodes.
    identity: Option<(u64, u64)>,
    last_line: Option<LastLine>,
    /// Where scanning stopped inside an oversized line that has no newline
    /// yet. Every byte from `offset` to here belongs to that line, so the
    /// next call resumes here instead of scanning the whole line again.
    skipped_to: Option<u64>,
    /// Bytes pulled from disk by the scanning loop, over this `Tail`'s life.
    #[cfg(test)]
    scanned: u64,
}

/// The last complete line handed back, by position and content.
///
/// Hashes rather than bytes, so a follower holds a few words per pane
/// instead of up to `MAX_LINE_BYTES`.
#[derive(Clone, Copy)]
struct LastLine {
    /// Where the line starts. The end of the line is `offset`.
    start: u64,
    /// Of the line's content, without its `\n`.
    hash: u64,
    /// Of the up to `BEFORE_BYTES` bytes that end where the line starts.
    before: u64,
}

/// What a file whose length moved did to the last line read.
enum Change {
    /// Still there, unchanged, and everything after it is new.
    None,
    /// The line was rewritten, cut short, or removed; everything before it
    /// is as it was.
    LastLine,
    /// The bytes before the line changed too: this is a different file.
    Replaced,
}

fn hash_of(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

#[cfg(unix)]
fn identity_of(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn identity_of(_: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// Reads `[from, to)`, or `None` if any of it cannot be read.
fn read_range(file: &mut File, from: u64, to: u64) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; (to - from) as usize];
    file.seek(SeekFrom::Start(from)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

impl Tail {
    /// Starts at the START of a small file and at the END of a large one — see
    /// [`READ_FROM_START_BYTES`] for where the line is drawn and why.
    ///
    /// A file that does not exist yet starts at 0, which is both answers at
    /// once: there is nothing to replay and nothing to skip.
    ///
    /// Reading a small file whole is not merely cheap, it is the fix for a
    /// pane's first turn being invisible: the turn boundary is written before
    /// anything can find the file, so an attachment that begins at the end
    /// begins after the only line that says a turn is open.
    pub fn new(path: PathBuf) -> Tail {
        let meta = std::fs::metadata(&path).ok();
        let len = meta.as_ref().map_or(0, |m| m.len());
        let offset = if len <= READ_FROM_START_BYTES { 0 } else { len };
        Tail {
            identity: meta.as_ref().and_then(identity_of),
            path,
            offset,
            last_line: None,
            skipped_to: None,
            #[cfg(test)]
            scanned: 0,
        }
    }

    /// Which file this is following.
    ///
    /// Exposed so a caller that re-derives which file a pane should be reading
    /// can compare the answer against the one already open, and keep this
    /// `Tail` when they are the same. Replacing an identical one is not a
    /// no-op, whichever end a fresh `Tail` would start at: on a large file it
    /// starts at the end and swallows everything written between the two, and
    /// on a small one it starts at the beginning and hands back every line
    /// already seen a second time.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns every complete line appended since the last call, in file
    /// order — the only order these logs guarantee, since `timestamp` fields
    /// are not monotonic (see `docs/agent-session-logs.md`).
    ///
    /// Never returns a line that does not yet end in `\n`, and never returns
    /// a line whose content exceeds `MAX_LINE_BYTES`. Both kinds still
    /// advance (or correctly fail to advance) the stored offset.
    ///
    /// A last line that has CHANGED since it was read is read again, from its
    /// start. Cursor-agent (2026.09.02, seen live) starts a turn by removing
    /// the previous turn's `{"type":"turn_ended"}` line and writing the new
    /// prompt where it was, so the file grows -- 3,619 bytes to 3,911 -- and
    /// the old offset lands 41 bytes into the new record. Read from there,
    /// the prompt is a fragment that is not JSON, the turn start is never
    /// seen, and every turn after the first read Idle. Only the last line is
    /// compared whole: that is the one cursor rewrites, and a change further
    /// back is history the parsers have already folded.
    ///
    /// The same holds when the file now ends INSIDE that line, shorter than
    /// it was: removed and not yet rewritten, or rewritten shorter. Reading
    /// resumes at the line's start, so a removed line yields nothing and the
    /// line written in its place is read once. That is right only if the
    /// line is all that changed, so the `BEFORE_BYTES` before it are checked
    /// too; if they differ, the file was truncated and refilled with
    /// something else, and is read from its start.
    ///
    /// A different file at the same path (a new inode, as a rename over it
    /// leaves) is read from its start, whatever its length. Callers fold
    /// lines into last-boundary-wins state, so a replacement that repeats
    /// lines already read leaves them where they would have been.
    ///
    /// Checked only when the length has moved past what was scanned. A
    /// rewrite to exactly the same length is missed until the file next
    /// changes, and is then caught, because the line at the old position
    /// still differs.
    pub fn read_new_lines(&mut self) -> Vec<String> {
        // A missing file is the normal case for an agent that has not started
        // writing yet, not an error: return nothing and leave the offset
        // alone, so the very next call notices the file once it appears.
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let (len, identity) = match file.metadata() {
            Ok(m) => (m.len(), identity_of(&m)),
            Err(_) => return Vec::new(),
        };

        // A different file now has this name. Nothing remembered about the
        // old one says anything about it, however their lengths compare.
        if self.identity.is_some() && identity != self.identity {
            self.offset = 0;
            self.last_line = None;
            self.skipped_to = None;
        }
        self.identity = identity;

        // Before the shrink rule, so a rewrite that leaves the file shorter
        // than it was re-reads one line rather than the whole file.
        if len != self.skipped_to.unwrap_or(self.offset) {
            if let Some(last) = self.last_line {
                match self.change(&mut file, last, len) {
                    Change::None => {}
                    Change::LastLine => {
                        self.offset = last.start;
                        self.last_line = None;
                        self.skipped_to = None;
                    }
                    Change::Replaced => {
                        self.offset = 0;
                        self.last_line = None;
                        self.skipped_to = None;
                    }
                }
            }
        }

        // Smaller than what was already read means the file was truncated or
        // replaced underneath us — the old offset now points into the middle
        // of a record that no longer exists. Reset rather than seek there.
        if len < self.offset {
            self.offset = 0;
            self.last_line = None;
            self.skipped_to = None;
        }
        // Every rule above that moves the offset drops the skip with it. One
        // left standing still starts at `offset`; if the file no longer
        // reaches where it stopped, the line was cut short or rewritten, and
        // is scanned again from its start.
        if self.skipped_to.is_some_and(|to| len < to) {
            self.skipped_to = None;
        }
        let resume = self.skipped_to.unwrap_or(self.offset);
        if len == resume {
            return Vec::new();
        }
        if file.seek(SeekFrom::Start(resume)).is_err() {
            return Vec::new();
        }

        let mut lines = Vec::new();
        // Bytes of the line currently being accumulated. Cleared (not just
        // truncated) once it crosses the cap, so an oversized or
        // still-growing line never holds more than MAX_LINE_BYTES in memory
        // regardless of how large it eventually turns out to be.
        let mut current = Vec::new();
        let mut current_over_cap = self.skipped_to.is_some();
        // Every byte of the line being accumulated, including the ones an
        // over-cap line has stopped storing. `current.len()` cannot stand in
        // for it: once the line crosses the cap, `current` is emptied and
        // stays empty, so it says nothing about how long the line was. A
        // resumed skip starts with every byte an earlier call counted.
        let mut current_len: u64 = resume - self.offset;
        // Bytes belonging to lines already terminated by `\n` — safe to add
        // to the stored offset. Kept separate from bytes of the in-progress
        // line, which must NOT advance the offset until its own newline
        // arrives, or a half-written record would be skipped over rather
        // than re-read whole on the next call.
        let mut consumed: u64 = 0;

        let mut last = self.last_line;
        let mut last_is_new = false;

        let mut chunk = [0u8; CHUNK_BYTES];
        loop {
            let n = match file.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => n,
                // Stop on a read error; whatever complete lines were already
                // found are still returned, and the offset only advances past
                // them, so the unread remainder is picked up next call.
                Err(_) => break,
            };
            #[cfg(test)]
            {
                self.scanned += n as u64;
            }
            for &byte in &chunk[..n] {
                if byte == b'\n' {
                    // Where this line started, before `consumed` moves past
                    // it. An oversized line cannot be checked later, since
                    // none of it was kept, so it leaves nothing to check.
                    // `before` is filled in once the scan is done.
                    last = (!current_over_cap).then(|| LastLine {
                        start: self.offset + consumed,
                        hash: hash_of(&current),
                        before: 0,
                    });
                    last_is_new = true;
                    if !current_over_cap {
                        // Session logs are UTF-8 JSONL; a line that is not
                        // valid UTF-8 cannot become a `String` and is dropped
                        // the same way an oversized line is — its bytes still
                        // count toward `consumed` so the next line is not
                        // misread.
                        if let Ok(text) = std::str::from_utf8(&current) {
                            lines.push(text.to_string());
                        }
                    }
                    consumed += current_len + 1;
                    current.clear();
                    current_len = 0;
                    current_over_cap = false;
                } else {
                    current_len += 1;
                    if !current_over_cap {
                        current.push(byte);
                        if current.len() > MAX_LINE_BYTES {
                            current_over_cap = true;
                            current.clear();
                        }
                    }
                }
                // While over cap, bytes are counted in `current_len` but not
                // stored. `consumed` only grows when the terminating `\n` is
                // found above, by the whole line's length, so the offset
                // lands exactly after a skipped line without holding it.
            }
        }

        self.offset += consumed;
        self.skipped_to = current_over_cap.then_some(self.offset + current_len);
        if last_is_new {
            if let Some(line) = last.as_mut() {
                let from = line.start.saturating_sub(BEFORE_BYTES);
                // A line whose surroundings cannot be read cannot be checked
                // later, the same as an oversized one.
                match read_range(&mut file, from, line.start) {
                    Some(bytes) => line.before = hash_of(&bytes),
                    None => last = None,
                }
            }
        }
        self.last_line = last;
        lines
    }

    /// What became of the last line read, now that the file is `len` bytes.
    ///
    /// A file too short to reach the line's start has plainly been replaced.
    /// A read that fails says nothing either way and is not a change: the
    /// offset is left alone, as every other failure here leaves it.
    fn change(&self, file: &mut File, last: LastLine, len: u64) -> Change {
        if len < last.start {
            return Change::Replaced;
        }
        let from = last.start.saturating_sub(BEFORE_BYTES);
        let to = if len < self.offset { last.start } else { self.offset };
        let Some(mut bytes) = read_range(file, from, to) else { return Change::None };
        let mut line = bytes.split_off((last.start - from) as usize);
        if hash_of(&bytes) != last.before {
            return Change::Replaced;
        }
        if len < self.offset || line.pop() != Some(b'\n') || hash_of(&line) != last.hash {
            return Change::LastLine;
        }
        Change::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "farcooler-tail-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p.join("session.jsonl")
    }

    fn append(path: &PathBuf, bytes: &[u8]) {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
    }

    #[test]
    fn two_appended_lines_are_read_once_each() {
        let path = scratch("two-lines");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        append(&path, b"{\"a\":1}\n{\"a\":2}\n");
        assert_eq!(tail.read_new_lines(), vec!["{\"a\":1}", "{\"a\":2}"]);
        // Nothing new since the last read.
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    #[test]
    fn a_partial_final_line_is_held_until_its_newline_arrives() {
        let path = scratch("partial-line");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        append(&path, b"{\"whole\":true}\n{\"half");
        // The half-written record must not be handed back as if it were whole.
        assert_eq!(tail.read_new_lines(), vec!["{\"whole\":true}"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, b"-written\":true}\n");
        assert_eq!(tail.read_new_lines(), vec!["{\"half-written\":true}"]);
    }

    #[test]
    fn a_line_over_the_cap_is_skipped_and_the_offset_stays_correct() {
        let path = scratch("oversized-line");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        let huge = "x".repeat(MAX_LINE_BYTES + 1);
        append(&path, format!("before\n{huge}\nafter\n").as_bytes());

        // The oversized line is skipped, not held, and does not corrupt the
        // lines around it.
        assert_eq!(tail.read_new_lines(), vec!["before", "after"]);
        // Read again: the offset must sit after "after", not short of it by
        // the skipped line's length, or this hands back a fragment of the
        // long line and "after" a second time.
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, b"later\n");
        assert_eq!(tail.read_new_lines(), vec!["later"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// A line of exactly `MAX_LINE_BYTES` is the largest one kept, and one
    /// byte more is the smallest one skipped. Each is followed by repeated
    /// reads, since a wrong offset only shows on the read after.
    #[test]
    fn lines_either_side_of_the_cap_are_each_read_once() {
        let path = scratch("cap-boundary");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        let at_cap = "a".repeat(MAX_LINE_BYTES);
        append(&path, format!("{at_cap}\none\n").as_bytes());
        assert_eq!(tail.read_new_lines(), vec![at_cap.as_str(), "one"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        let over_cap = "b".repeat(MAX_LINE_BYTES + 1);
        append(&path, format!("{over_cap}\ntwo\n").as_bytes());
        assert_eq!(tail.read_new_lines(), vec!["two"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, b"three\n");
        assert_eq!(tail.read_new_lines(), vec!["three"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// An oversized line still being written: it is past the cap before its
    /// newline arrives, so the read that finds the newline has to count
    /// bytes it never stored, some of them seen by an earlier call.
    #[test]
    fn an_oversized_line_written_in_pieces_is_skipped_once() {
        let path = scratch("oversized-partial");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        let first_half = "x".repeat(MAX_LINE_BYTES + 10);
        append(&path, format!("before\n{first_half}").as_bytes());
        assert_eq!(tail.read_new_lines(), vec!["before"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, format!("{}\nafter\n", "y".repeat(CHUNK_BYTES * 3)).as_bytes());
        assert_eq!(tail.read_new_lines(), vec!["after"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// Truncated after an oversized line was skipped. The reset has to start
    /// from the replacement's beginning and then hold its place there.
    #[test]
    fn a_file_truncated_after_an_oversized_line_is_read_once() {
        let path = scratch("oversized-then-truncated");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        let huge = "x".repeat(MAX_LINE_BYTES * 2);
        append(&path, format!("{huge}\nold\n").as_bytes());
        assert_eq!(tail.read_new_lines(), vec!["old"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        // Replaced by something longer than the bytes a short offset would
        // have counted, but shorter than what was really read.
        let replacement = format!("{}\n", "n".repeat(200));
        std::fs::write(&path, &replacement).unwrap();
        assert_eq!(tail.read_new_lines(), vec![&replacement[..200]]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, b"next\n");
        assert_eq!(tail.read_new_lines(), vec!["next"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// CRLF line endings: the `\r` is part of the line's bytes, which
    /// serde_json reads as trailing whitespace, and it is counted toward the
    /// offset like any other byte, oversized line included.
    #[test]
    fn crlf_lines_are_read_once_including_around_an_oversized_one() {
        let path = scratch("crlf");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        let huge = "x".repeat(MAX_LINE_BYTES + 1);
        append(&path, format!("{{\"a\":1}}\r\n{huge}\r\n{{\"a\":2}}\r\n").as_bytes());
        assert_eq!(tail.read_new_lines(), vec!["{\"a\":1}\r", "{\"a\":2}\r"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, b"{\"a\":3}\r\n");
        assert_eq!(tail.read_new_lines(), vec!["{\"a\":3}\r"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// A line of `width` bytes, repeated until the file is over `bytes`.
    ///
    /// Real enough for the size rule: what is under test is the byte count, and
    /// every line has to be complete or the reader would legitimately hold the
    /// last one back.
    fn write_bigger_than(path: &PathBuf, bytes: u64) {
        let line = format!("{}\n", "x".repeat(1023));
        let mut file = std::fs::File::create(path).unwrap();
        let mut written = 0u64;
        while written <= bytes {
            file.write_all(line.as_bytes()).unwrap();
            written += line.len() as u64;
        }
        file.flush().unwrap();
    }

    #[test]
    fn a_shrunk_file_resets_the_offset_to_zero() {
        let path = scratch("shrunk-file");
        // Over the bound, so this attaches at the end — which is what gives
        // the offset something to be reset FROM.
        write_bigger_than(&path, READ_FROM_START_BYTES);
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        // Truncate and replace with something shorter than the old offset —
        // this is what a rotated or truncated log looks like on disk.
        std::fs::write(&path, "new\n").unwrap();
        assert_eq!(tail.read_new_lines(), vec!["new"]);
    }

    /// The pane's FIRST TURN, which attaching at the end always missed.
    ///
    /// Codex is the case that cannot be worked around: it opens its rollout
    /// when the first turn is submitted, so by the time anything can find the
    /// file the `task_started` line is already behind it. A reader that starts
    /// at the end therefore never sees a turn begin, drops the steps that
    /// follow — a step with no turn believed is not evidence of one — and only
    /// recovers when a SECOND turn starts.
    #[test]
    fn a_small_file_is_read_from_the_start_so_the_first_turn_is_seen() {
        let path = scratch("small-file");
        std::fs::write(&path, "{\"turn\":\"started\"}\n{\"step\":1}\n").unwrap();

        let mut tail = Tail::new(path.clone());
        assert_eq!(
            tail.read_new_lines(),
            vec!["{\"turn\":\"started\"}", "{\"step\":1}"],
            "the turn that was already underway when we attached"
        );
        // And still exactly once: reading from the start is not re-reading.
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// The other half of the rule, and the reason it is a rule and not simply
    /// "read from the start": one real claude directory here holds 632 MB
    /// across 49 files, with single files at 108 MB. Replaying one of those to
    /// find out what an agent is doing right now is minutes and megabytes
    /// spent on history nobody asked for.
    #[test]
    fn a_large_file_is_still_attached_to_at_its_end() {
        let path = scratch("large-file");
        write_bigger_than(&path, READ_FROM_START_BYTES);

        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), Vec::<String>::new(), "no history is replayed");

        // What matters is what happens NEXT: the pane still follows its log.
        append(&path, b"{\"appended\":true}\n");
        assert_eq!(tail.read_new_lines(), vec!["{\"appended\":true}"]);
    }

    /// The last record of turn 1 in a real cursor-agent 2026.09.02 transcript,
    /// the `turn_ended` after it, and the user record of turn 2 -- verbatim.
    const CURSOR_LAST_STEP: &str = r#"{"role":"assistant","message":{"content":[{"type":"text","text":"Finished both sleeps and wrote `note.md`."}]}}"#;
    const CURSOR_TURN_ENDED: &str = r#"{"type":"turn_ended","status":"success"}"#;
    const CURSOR_NEXT_PROMPT: &str = r#"{"role":"user","message":{"content":[{"type":"text","text":"<timestamp>Thursday, Sep 24, 2026, 11:08 AM (UTC+8)</timestamp>\n<user_query>\nWithout using any tools, write a careful 400-word explanation of how Raft leader election handles split votes. Then run `sleep 40 && echo three` in the shell, then say done.\n</user_query>"}]}}"#;

    /// Cursor does not only append. When a new turn starts it REWRITES its
    /// transcript: the previous turn's `turn_ended` line is removed and the
    /// new user record written where it was. Seen live: 3,619 bytes ending
    /// in `turn_ended`, then 3,911 bytes ending in the user record, with
    /// `turn_ended` in the file once, at the very end of the session.
    ///
    /// The file grew, so the shrink rule never fired, and the read resumed
    /// at byte 3,619 -- 41 bytes into the new user record. What came back
    /// was a fragment that is not JSON, so the turn start was never seen and
    /// the row read Idle for every turn after the first.
    #[test]
    fn a_last_line_rewritten_in_place_is_read_again_whole() {
        let path = scratch("rewritten-last-line");
        std::fs::write(&path, format!("{CURSOR_LAST_STEP}\n{CURSOR_TURN_ENDED}\n")).unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), vec![CURSOR_LAST_STEP, CURSOR_TURN_ENDED]);

        // Turn 2: the same bytes up to the end of the last step, then the
        // prompt where `turn_ended` was. Longer than what it replaced, as it
        // was live, so nothing about the length says anything changed.
        std::fs::write(&path, format!("{CURSOR_LAST_STEP}\n{CURSOR_NEXT_PROMPT}\n")).unwrap();
        assert!(CURSOR_NEXT_PROMPT.len() > CURSOR_TURN_ENDED.len());
        assert_eq!(tail.read_new_lines(), vec![CURSOR_NEXT_PROMPT]);
        // Read once, and the lines before it were never handed back again.
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// The same rewrite with a replacement SHORTER than `turn_ended`, which
    /// shrinks the file. The shrink rule alone would start over from zero and
    /// hand the previous turn back a second time -- its start, its steps and
    /// its end, all over again. Only the rewritten line is read.
    #[test]
    fn a_last_line_rewritten_shorter_is_read_again_without_the_rest() {
        let path = scratch("rewritten-shorter");
        std::fs::write(&path, format!("{CURSOR_LAST_STEP}\n{CURSOR_TURN_ENDED}\n")).unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), vec![CURSOR_LAST_STEP, CURSOR_TURN_ENDED]);

        let short = r#"{"role":"user"}"#;
        assert!(short.len() < CURSOR_TURN_ENDED.len());
        std::fs::write(&path, format!("{CURSOR_LAST_STEP}\n{short}\n")).unwrap();
        assert_eq!(tail.read_new_lines(), vec![short]);
    }

    /// A log replaced by a different file at the same path, at least as long
    /// as what was read, so no length rule can see it. The last line read was
    /// oversized, so there is no last line to check either: only the file's
    /// identity says this is not the file that was being read.
    #[test]
    fn a_log_replaced_by_a_new_file_at_least_as_long_is_read_from_its_start() {
        let path = scratch("replaced-by-new-file");
        let huge = "x".repeat(MAX_LINE_BYTES + 1);
        std::fs::write(&path, format!("one\n{huge}\n")).unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), vec!["one"]);

        // Written beside it and renamed over it, as a rotation would be. Old
        // and new agree on every length, so the old offset lands exactly at
        // "three" and would hand back nothing before it.
        let next = path.with_extension("next");
        std::fs::write(&next, format!("uno\n{huge}\nthree\n")).unwrap();
        std::fs::rename(&next, &path).unwrap();
        assert_eq!(tail.read_new_lines(), vec!["uno", "three"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// An over-cap line still being written is skipped from where the last
    /// call left off, not scanned again from its start on every poll. The
    /// second read here scans only the bytes appended since the first.
    #[test]
    fn a_growing_oversized_line_is_not_scanned_again_from_its_start() {
        let path = scratch("growing-oversized");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        let first = format!("before\n{}", "x".repeat(MAX_LINE_BYTES + 10));
        append(&path, first.as_bytes());
        assert_eq!(tail.read_new_lines(), vec!["before"]);
        assert_eq!(tail.scanned, first.len() as u64);

        // Nothing appended: nothing scanned.
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
        assert_eq!(tail.scanned, first.len() as u64);

        let more = format!("{}\nafter\n", "y".repeat(CHUNK_BYTES * 3));
        append(&path, more.as_bytes());
        assert_eq!(tail.read_new_lines(), vec!["after"]);
        assert_eq!(tail.scanned, (first.len() + more.len()) as u64);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, b"later\n");
        assert_eq!(tail.read_new_lines(), vec!["later"]);
    }

    /// The over-cap line being skipped is cut short before its newline. What
    /// was remembered about it no longer holds, so it is scanned again from
    /// its start, and the line that replaced it is read.
    #[test]
    fn a_growing_oversized_line_cut_short_is_scanned_again() {
        let path = scratch("growing-oversized-cut");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        append(&path, format!("before\n{}", "x".repeat(MAX_LINE_BYTES * 2)).as_bytes());
        assert_eq!(tail.read_new_lines(), vec!["before"]);

        std::fs::write(&path, "before\nshort\n").unwrap();
        assert_eq!(tail.read_new_lines(), vec!["short"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// The file is cut back to somewhere inside the last line read, and what
    /// is there now is not that line rewritten: the bytes before it changed
    /// too, so the whole file is different and is read from its start. The
    /// same file, not a new one, so identity says nothing here.
    #[test]
    fn a_shrink_inside_the_last_line_with_a_new_head_is_read_from_the_start() {
        let path = scratch("shrink-new-head");
        std::fs::write(&path, "first line here\nsecond-last-line-long-content\n").unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), vec!["first line here", "second-last-line-long-content"]);

        // 26 bytes: past where the last line started (16), short of where it
        // ended (45).
        std::fs::write(&path, "replaced-file\nnew-content\n").unwrap();
        assert_eq!(tail.read_new_lines(), vec!["replaced-file", "new-content"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// Cursor caught between removing its last line and writing the new one:
    /// the file ends where that line began. Nothing is read, and the line
    /// written there next is read once, without the lines before it.
    #[test]
    fn a_last_line_removed_then_written_is_read_once() {
        let path = scratch("removed-then-written");
        std::fs::write(&path, format!("{CURSOR_LAST_STEP}\n{CURSOR_TURN_ENDED}\n")).unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), vec![CURSOR_LAST_STEP, CURSOR_TURN_ENDED]);

        std::fs::write(&path, format!("{CURSOR_LAST_STEP}\n")).unwrap();
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, format!("{CURSOR_NEXT_PROMPT}\n").as_bytes());
        assert_eq!(tail.read_new_lines(), vec![CURSOR_NEXT_PROMPT]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// Multi-byte UTF-8 at the cap. A line of exactly `MAX_LINE_BYTES` that
    /// ends in a three-byte character is kept whole; one whose last
    /// character straddles the cap is skipped whole. Neither is ever cut
    /// inside a character, and the lines after each are read once.
    #[test]
    fn a_multi_byte_character_at_the_cap_is_never_split() {
        let path = scratch("utf8-at-cap");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        let euro = "\u{20ac}";
        assert_eq!(euro.len(), 3);
        let at_cap = format!("{}{euro}", "a".repeat(MAX_LINE_BYTES - 3));
        assert_eq!(at_cap.len(), MAX_LINE_BYTES);
        // The euro's first byte is the cap's last, so its other two are over.
        let straddling = format!("{}{euro}", "b".repeat(MAX_LINE_BYTES - 1));
        assert_eq!(straddling.len(), MAX_LINE_BYTES + 2);

        append(&path, format!("{at_cap}\none\n{straddling}\ntwo \u{e9}\n").as_bytes());
        assert_eq!(tail.read_new_lines(), vec![at_cap.as_str(), "one", "two \u{e9}"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, b"three\n");
        assert_eq!(tail.read_new_lines(), vec!["three"]);
    }

    #[test]
    fn a_missing_file_yields_nothing_and_does_not_error() {
        let path = scratch("missing-file");
        // Deliberately never created.
        let mut tail = Tail::new(path);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }
}
