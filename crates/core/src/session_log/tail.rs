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

/// How much of the file's start is kept as its signature: a cheap check, on
/// every read that finds the length moved, that the file is still the one
/// being read. One `pread` of 4 KiB covers the first record of any of the
/// three formats, which carries the session's id.
const HEAD_BYTES: u64 = 4 * 1024;

/// How much of an oversized line's start is kept, to check before resuming a
/// skip that what is at `offset` is still that line.
const SKIP_HEAD_BYTES: usize = 64;

/// FNV-1a, 64-bit. Not for anything adversarial: these hashes only notice
/// a log that changed under the reader. It is used because a hash of a long
/// range can be carried forward byte by byte as the range grows, which is
/// what lets the prefix hash below cost nothing extra while scanning.
const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv(mut state: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        state = (state ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
    }
    state
}

/// A position in one session log file, advanced only past complete lines.
///
/// Holds a path, a byte offset, and what is needed to tell whether the file
/// at that path is still the one the offset is in: its identity, its first
/// bytes, a hash of everything read so far, and the last line read. Every
/// hazard in `docs/agent-session-logs.md` — the half-written tail, the
/// oversized line, the file that shrinks, the file that is replaced — is
/// checked against the file on each call that finds its length moved, so
/// what is remembered is only ever a shortcut: when the file no longer agrees
/// with it, it is dropped and the state re-derived. A crash or restart loses
/// nothing worse than re-scanning from the last complete line.
pub struct Tail {
    path: PathBuf,
    offset: u64,
    /// Where this `Tail` began reading: 0, or the end of a large file it
    /// attached to. `prefix` covers the bytes from here.
    attach: u64,
    /// The FNV-1a hash of `[attach, offset)`, carried forward as lines are
    /// read. Checked against the file whenever the reader would otherwise go
    /// back, or the file looks replaced.
    prefix: u64,
    /// The device and inode `path` named on the last read. `None` before the
    /// file exists, and always on a platform without inodes.
    identity: Option<(u64, u64)>,
    head: Option<Head>,
    last_line: Option<LastLine>,
    skip: Option<Skip>,
    /// Bytes pulled from disk by the scanning loop, over this `Tail`'s life.
    #[cfg(test)]
    scanned: u64,
    /// Times `settle` fell through to hashing the file again from `attach`.
    #[cfg(test)]
    rehashed: u64,
}

/// The file's first bytes, as last seen.
#[derive(Clone, Copy)]
struct Head {
    /// How many: the file's length, up to `HEAD_BYTES`.
    len: u64,
    hash: u64,
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
    /// `prefix` as it was at `start`: the hash of `[attach, start)`.
    before: u64,
}

/// An oversized line, without its newline yet, that scanning stopped inside.
/// It starts at `offset`; every byte from there to `to` belongs to it, so the
/// next call resumes at `to` instead of scanning the whole line again.
#[derive(Clone, Copy)]
struct Skip {
    to: u64,
    /// Of the line's first `SKIP_HEAD_BYTES`.
    head: u64,
    /// The hash of `[attach, to)`, so the scan carries on from there.
    state: u64,
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

/// `state` carried forward over `[from, to)`, or `None` if any of it cannot
/// be read.
fn hash_range(file: &mut File, from: u64, to: u64, mut state: u64) -> Option<u64> {
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut left = to - from;
    let mut chunk = [0u8; CHUNK_BYTES];
    while left > 0 {
        let take = left.min(CHUNK_BYTES as u64) as usize;
        file.read_exact(&mut chunk[..take]).ok()?;
        state = fnv(state, &chunk[..take]);
        left -= take as u64;
    }
    Some(state)
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
            attach: offset,
            prefix: FNV_START,
            head: None,
            last_line: None,
            skip: None,
            #[cfg(test)]
            scanned: 0,
            #[cfg(test)]
            rehashed: 0,
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
    /// seen, and every turn after the first read Idle. The same holds when
    /// the file now ends inside that line: removed and not yet rewritten, or
    /// rewritten shorter.
    ///
    /// Going back to the line's start is right only if the line is all that
    /// changed, so every byte before it is hashed again and compared. That
    /// costs a read of everything this `Tail` has read, but only when the
    /// file shrank, the last line changed, or the file looks replaced; on an
    /// ordinary append it costs nothing. If the bytes differ, the file was
    /// replaced, and it is read from its start, whatever its size.
    ///
    /// "Looks replaced" is a new inode at the path (a rename over it), or
    /// first bytes that differ (which also catches a delete and re-create
    /// that reuses the inode, as ext4 often does). Even then, a file whose
    /// bytes still match what was read is followed from where it was, not
    /// replayed: a rename of a copy that only appended is just an append.
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
        let renamed = self.identity.is_some() && identity != self.identity;
        self.identity = identity;

        let end = self.skip.map_or(self.offset, |skip| skip.to);
        if !renamed && len == end {
            return Vec::new();
        }
        // A read that fails says nothing either way: leave everything as it
        // is, as every other failure here does, and look again next call.
        if self.settle(&mut file, len, renamed).is_none() {
            return Vec::new();
        }
        self.note_head(&mut file, len);

        let resume = self.skip.map_or(self.offset, |skip| skip.to);
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
        let mut current_over_cap = self.skip.is_some();
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
        // The prefix hash through `consumed`, and through every byte scanned.
        let mut committed = self.prefix;
        let mut running = self.skip.map_or(self.prefix, |skip| skip.state);
        let mut skip_head = self.skip.map(|skip| skip.head);

        let mut last = self.last_line;

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
                running = fnv(running, &[byte]);
                if byte == b'\n' {
                    // Where this line started, before `consumed` moves past
                    // it. An oversized line cannot be checked later, since
                    // none of it was kept, so it leaves nothing to check.
                    last = (!current_over_cap).then(|| LastLine {
                        start: self.offset + consumed,
                        hash: fnv(FNV_START, &current),
                        before: committed,
                    });
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
                    committed = running;
                    current.clear();
                    current_len = 0;
                    current_over_cap = false;
                    skip_head = None;
                } else {
                    current_len += 1;
                    if !current_over_cap {
                        current.push(byte);
                        if current.len() > MAX_LINE_BYTES {
                            current_over_cap = true;
                            skip_head = Some(fnv(FNV_START, &current[..SKIP_HEAD_BYTES]));
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
        self.prefix = committed;
        self.last_line = last;
        self.skip = match (current_over_cap, skip_head) {
            (true, Some(head)) => Some(Skip { to: self.offset + current_len, head, state: running }),
            _ => None,
        };
        lines
    }

    /// Decides where reading resumes, now that the file is `len` bytes and
    /// its length has moved (or its identity has). `None` if a read failed.
    ///
    /// The cheap checks come first: on an ordinary append to the same file,
    /// with its first bytes, its last line and any skipped line's start all
    /// as they were, nothing more is read. Anything else hashes the file
    /// again from `attach` and resumes at the furthest point the hash still
    /// agrees with: the offset, else the last line's start, else 0.
    fn settle(&mut self, file: &mut File, len: u64, renamed: bool) -> Option<()> {
        let end = self.skip.map_or(self.offset, |skip| skip.to);
        let head_moved = match self.head {
            Some(head) if len >= head.len => hash_range(file, 0, head.len, FNV_START)? != head.hash,
            _ => false,
        };
        // Spent either way: one that no longer matches, or no longer fits,
        // is taken again once this settles, so it is not tripped every call.
        if head_moved || self.head.is_some_and(|head| head.len > len) {
            self.head = None;
        }
        if !renamed && !head_moved && len >= end {
            let line_same = match self.last_line {
                Some(last) => {
                    let mut line = vec![0u8; (self.offset - last.start) as usize];
                    file.seek(SeekFrom::Start(last.start)).ok()?;
                    file.read_exact(&mut line).ok()?;
                    line.pop() == Some(b'\n') && fnv(FNV_START, &line) == last.hash
                }
                None => true,
            };
            if line_same && self.skip_same(file, len)? {
                return Some(());
            }
        }

        #[cfg(test)]
        {
            self.rehashed += 1;
        }
        if len < self.attach {
            self.replaced();
            return Some(());
        }
        let mut state = FNV_START;
        let mut at = self.attach;
        if let Some(last) = self.last_line {
            if len < last.start {
                self.replaced();
                return Some(());
            }
            state = hash_range(file, at, last.start, state)?;
            at = last.start;
            if state != last.before {
                self.replaced();
                return Some(());
            }
        }
        if len >= self.offset && hash_range(file, at, self.offset, state)? == self.prefix {
            if !self.skip_same(file, len)? {
                self.skip = None;
            }
            return Some(());
        }
        match self.last_line {
            Some(last) => {
                self.offset = last.start;
                self.prefix = last.before;
                self.last_line = None;
                self.skip = None;
            }
            None => self.replaced(),
        }
        Some(())
    }

    /// Whether a skip in progress, if any, can still be resumed: the file
    /// still reaches where it stopped, and still starts that line with the
    /// same bytes.
    fn skip_same(&self, file: &mut File, len: u64) -> Option<bool> {
        let Some(skip) = self.skip else { return Some(true) };
        if len < skip.to {
            return Some(false);
        }
        let head = hash_range(file, self.offset, self.offset + SKIP_HEAD_BYTES as u64, FNV_START)?;
        Some(head == skip.head)
    }

    /// The file is not the one that was being read: start it from 0.
    fn replaced(&mut self) {
        self.offset = 0;
        self.attach = 0;
        self.prefix = FNV_START;
        self.head = None;
        self.last_line = None;
        self.skip = None;
    }

    /// Takes the file's first bytes as its signature, until there are
    /// `HEAD_BYTES` of them. A failed read leaves the old one, if any.
    fn note_head(&mut self, file: &mut File, len: u64) {
        let want = len.min(HEAD_BYTES);
        if self.head.is_some_and(|head| head.len >= want) {
            return;
        }
        if let Some(hash) = hash_range(file, 0, want, FNV_START) {
            self.head = Some(Head { len: want, hash });
        }
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

    /// A skip in progress, then the same file truncated and refilled past
    /// where the skip stopped. What sits at the skipped line's start is not
    /// that line any more, so the skip is dropped and the refill read from
    /// its start, not resumed in the middle of bytes never seen.
    #[test]
    fn a_skip_is_not_resumed_into_a_file_refilled_in_place() {
        let path = scratch("skip-then-refilled");
        std::fs::write(&path, "").unwrap();
        let mut tail = Tail::new(path.clone());

        append(&path, "x".repeat(MAX_LINE_BYTES * 2).as_bytes());
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        let refill = format!("first\n{}\nkept\n", "y".repeat(MAX_LINE_BYTES * 2 + 100));
        std::fs::write(&path, refill).unwrap();
        assert_eq!(tail.read_new_lines(), vec!["first", "kept"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// Cut back inside the last line, with the 4 KiB before that line as
    /// they were but an earlier line changed. Every byte before the line is
    /// checked, not just the nearest, so this is a different file, read
    /// from its start.
    #[test]
    fn a_shrink_inside_the_last_line_checks_every_byte_before_it() {
        let path = scratch("shrink-old-change");
        let filler: String = (0..100).map(|i| format!("{{\"filler\":{i:040}}}\n")).collect();
        assert!(filler.len() > 4096);
        std::fs::write(&path, format!("early\n{filler}a-long-last-line\n")).unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines().len(), 102);

        std::fs::write(&path, format!("EARLY\n{filler}short\n")).unwrap();
        let again = tail.read_new_lines();
        assert_eq!(again.len(), 102);
        assert_eq!((again[0].as_str(), again[101].as_str()), ("EARLY", "short"));
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// A different file at the same path, on the same inode, at least as
    /// long, with no last line to check: what a delete and re-create looks
    /// like when the inode is reused, as ext4 often does. Its first bytes
    /// differ, so it is read from its start.
    #[test]
    fn a_log_refilled_on_the_same_inode_is_read_from_its_start() {
        let path = scratch("refilled-same-inode");
        let huge = "x".repeat(MAX_LINE_BYTES + 1);
        std::fs::write(&path, format!("one\n{huge}\n")).unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), vec!["one"]);

        std::fs::write(&path, format!("uno\n{huge}\nthree\n")).unwrap();
        assert_eq!(tail.read_new_lines(), vec!["uno", "three"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// A new inode whose bytes are the ones already read, plus more: an
    /// append delivered by a rename. Only what is new is read; nothing is
    /// handed back a second time.
    #[test]
    fn a_rename_of_the_same_bytes_plus_more_is_not_replayed() {
        let path = scratch("renamed-same-bytes");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), vec!["one", "two"]);

        let next = path.with_extension("next");
        std::fs::write(&next, "one\ntwo\nthree\n").unwrap();
        std::fs::rename(&next, &path).unwrap();
        assert_eq!(tail.read_new_lines(), vec!["three"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    /// Cursor's turn-start rewrite, if it were written to a new file and
    /// renamed over the old: the same as the rewrite in place, one line read
    /// again, nothing before it.
    #[test]
    fn a_last_line_rewritten_by_rename_is_read_again_without_the_rest() {
        let path = scratch("rewritten-by-rename");
        std::fs::write(&path, format!("{CURSOR_LAST_STEP}\n{CURSOR_TURN_ENDED}\n")).unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), vec![CURSOR_LAST_STEP, CURSOR_TURN_ENDED]);

        let next = path.with_extension("next");
        std::fs::write(&next, format!("{CURSOR_LAST_STEP}\n{CURSOR_NEXT_PROMPT}\n")).unwrap();
        std::fs::rename(&next, &path).unwrap();
        assert_eq!(tail.read_new_lines(), vec![CURSOR_NEXT_PROMPT]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());

        append(&path, b"{\"more\":1}\n");
        assert_eq!(tail.read_new_lines(), vec!["{\"more\":1}"]);
    }

    /// Hashing everything again is for a file that looks changed, once. A
    /// rewrite that reaches into the file's first `HEAD_BYTES` leaves the
    /// first bytes taken before it stale, so they are taken again, or every
    /// append after a cursor turn start would re-read the whole file.
    #[test]
    fn a_rewrite_is_checked_in_full_once_and_appends_after_it_are_not() {
        let path = scratch("rehash-once");
        // The last line starts before the 4 KiB mark and ends after it.
        let filler = format!("{}\n", "f".repeat(4070));
        std::fs::write(&path, format!("{filler}{CURSOR_TURN_ENDED}\n")).unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines().len(), 2);
        assert_eq!(tail.rehashed, 0);

        std::fs::write(&path, format!("{filler}{CURSOR_NEXT_PROMPT}\n")).unwrap();
        assert_eq!(tail.read_new_lines(), vec![CURSOR_NEXT_PROMPT]);
        assert_eq!(tail.rehashed, 1);

        for i in 0..3 {
            append(&path, format!("{{\"step\":{i}}}\n").as_bytes());
            assert_eq!(tail.read_new_lines().len(), 1);
        }
        assert_eq!(tail.rehashed, 1);
    }

    /// The refill keeps the file's first bytes and its last line read, and
    /// changes only what starts at the skipped line. The skipped line's own
    /// first bytes are what catch it.
    #[test]
    fn a_skip_is_not_resumed_when_only_the_skipped_line_changed() {
        let path = scratch("skip-line-changed");
        let filler = format!("{}\n", "f".repeat(5000));
        std::fs::write(&path, format!("{filler}{}", "x".repeat(MAX_LINE_BYTES * 2))).unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_new_lines(), vec![&filler[..5000]]);

        let refill = format!("{filler}first\n{}\nkept\n", "y".repeat(MAX_LINE_BYTES * 2 + 100));
        std::fs::write(&path, refill).unwrap();
        assert_eq!(tail.read_new_lines(), vec!["first", "kept"]);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }

    #[test]
    fn a_missing_file_yields_nothing_and_does_not_error() {
        let path = scratch("missing-file");
        // Deliberately never created.
        let mut tail = Tail::new(path);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }
}
