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

/// A position in one session log file, advanced only past complete lines.
///
/// Deliberately holds nothing but a path and a byte offset. Every hazard in
/// `docs/agent-session-logs.md` — the half-written tail, the oversized line,
/// the file that shrinks — is handled by re-deriving state from the file on
/// each call rather than remembering "mid-skip" or "mid-line" between calls,
/// so a crash or restart loses nothing worse than re-scanning from the last
/// complete line.
pub struct Tail {
    path: PathBuf,
    offset: u64,
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
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let offset = if len <= READ_FROM_START_BYTES { 0 } else { len };
        Tail { path, offset }
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
    pub fn read_new_lines(&mut self) -> Vec<String> {
        // A missing file is the normal case for an agent that has not started
        // writing yet, not an error: return nothing and leave the offset
        // alone, so the very next call notices the file once it appears.
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let len = match file.metadata() {
            Ok(m) => m.len(),
            Err(_) => return Vec::new(),
        };

        // Smaller than what was already read means the file was truncated or
        // replaced underneath us — the old offset now points into the middle
        // of a record that no longer exists. Reset rather than seek there.
        if len < self.offset {
            self.offset = 0;
        }
        if len == self.offset {
            return Vec::new();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }

        let mut lines = Vec::new();
        // Bytes of the line currently being accumulated. Cleared (not just
        // truncated) once it crosses the cap, so an oversized or
        // still-growing line never holds more than MAX_LINE_BYTES in memory
        // regardless of how large it eventually turns out to be.
        let mut current = Vec::new();
        let mut current_over_cap = false;
        // Bytes belonging to lines already terminated by `\n` — safe to add
        // to the stored offset. Kept separate from bytes of the in-progress
        // line, which must NOT advance the offset until its own newline
        // arrives, or a half-written record would be skipped over rather
        // than re-read whole on the next call.
        let mut consumed: u64 = 0;

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
            for &byte in &chunk[..n] {
                if byte == b'\n' {
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
                    consumed += current.len() as u64 + 1;
                    current.clear();
                    current_over_cap = false;
                } else if !current_over_cap {
                    current.push(byte);
                    if current.len() > MAX_LINE_BYTES {
                        current_over_cap = true;
                        current.clear();
                    }
                }
                // While over cap, bytes are neither stored nor counted here —
                // `consumed` only grows when the terminating `\n` is found
                // above, which is what keeps a skipped line's byte count
                // correct without holding the line itself.
            }
        }

        self.offset += consumed;
        lines
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

    #[test]
    fn a_missing_file_yields_nothing_and_does_not_error() {
        let path = scratch("missing-file");
        // Deliberately never created.
        let mut tail = Tail::new(path);
        assert_eq!(tail.read_new_lines(), Vec::<String>::new());
    }
}
