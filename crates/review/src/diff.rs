//! Unified diff in, hunks out — or an honest refusal.
//!
//! The parser is deliberately narrow. It accepts the unified diff git produces
//! for an ordinary two-sided comparison and rejects everything else by name.
//! The alternative, a parser that tries its best on anything, fails silently on
//! the one input it is most likely to meet.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::limits;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

impl LineKind {
    pub fn marker(self) -> char {
        match self {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub kind: LineKind,
    /// Absent on an added line, which has no position on the old side.
    pub old_no: Option<u32>,
    /// Absent on a removed line.
    pub new_no: Option<u32>,
    pub text: String,
    /// The file had no trailing newline here.
    ///
    /// Carried rather than dropped because it is a real difference: a diff that
    /// silently normalizes it will show a one-line change as no change, and a
    /// reviewer will believe the file is untouched.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_newline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    /// Position in this response. The selection key, and only meaningful
    /// alongside the response it came from.
    pub index: u32,
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<Line>,
    /// Hash of the path and the CHANGED lines only.
    ///
    /// Not an identity — the same changed lines can legitimately appear twice in
    /// one file, and anything resolving an anchor has to treat a second match as
    /// ambiguity rather than picking one. It excludes surrounding context on
    /// purpose, so that editing the code above a hunk does not renumber every
    /// comment attached below it.
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Truncation {
    LineCap,
    HunkCap,
    ByteCap,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub hunks: Vec<Hunk>,
    /// Present when the response is not the whole diff. A client showing a
    /// truncated diff must say so; one that does not is claiming the rest of the
    /// file is unchanged.
    pub truncated: Option<Truncation>,
    /// Cursor for the next page, when `truncated` is a hunk or line cap.
    pub next_hunk: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffError {
    /// A merge commit's combined diff (`git show` on a merge, `--cc`/`--combined`).
    ///
    /// Its lines carry one column per parent, so `++foo` and ` -bar` mean things
    /// an ordinary unified parser reads as literal text and a leading space. It
    /// is refused rather than parsed, and the caller shows the merge against its
    /// first parent instead.
    CombinedDiff,
    /// A hunk header that is not `@@ -a,b +c,d @@`.
    MalformedHeader(String),
    /// A line inside a hunk whose first byte is not one of ` `, `+`, `-`, `\`.
    MalformedLine(String),
    /// The hunk header promised a line count the body did not deliver.
    HunkLengthMismatch { header: String, expected: u32, found: u32 },
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiffError::CombinedDiff => write!(f, "combined diff (merge commit)"),
            DiffError::MalformedHeader(h) => write!(f, "malformed hunk header: {h}"),
            DiffError::MalformedLine(l) => write!(f, "malformed diff line: {l}"),
            DiffError::HunkLengthMismatch { header, expected, found } => {
                write!(f, "hunk {header} promised {expected} lines, body had {found}")
            }
        }
    }
}

impl std::error::Error for DiffError {}

/// Parse the body of one file's unified diff.
///
/// Input is everything after the `diff --git` / `---` / `+++` preamble: a
/// sequence of `@@` hunks. The preamble is the caller's to strip, because only
/// the caller knows whether it asked for one file or many.
pub fn parse_unified(path: &str, patch: &str) -> Result<FileDiff, DiffError> {
    parse_unified_from(path, patch, 0)
}

/// Parse from `from_hunk`, for paging a file whose diff exceeded a cap.
pub fn parse_unified_from(path: &str, patch: &str, from_hunk: u32) -> Result<FileDiff, DiffError> {
    if patch.len() > limits::MAX_BYTES {
        return Ok(FileDiff {
            path: path.to_string(),
            hunks: Vec::new(),
            truncated: Some(Truncation::ByteCap),
            next_hunk: None,
        });
    }

    let mut hunks: Vec<Hunk> = Vec::new();
    let mut truncated = None;
    let mut emitted_lines = 0usize;
    let mut hunk_index = 0u32;

    let mut lines = patch.lines().peekable();

    while let Some(raw) = lines.next() {
        // A combined diff's hunk header has one `@` per parent plus one, so a
        // merge of two parents opens with `@@@`. Checked before the ordinary
        // header, because `@@@ -1,2 -1,2 +1,3 @@@` starts with `@@` and would
        // otherwise be half-parsed into nonsense.
        if raw.starts_with("@@@") {
            return Err(DiffError::CombinedDiff);
        }
        if !raw.starts_with("@@") {
            // Preamble or trailing junk between hunks. `diff --git`, `index`,
            // `---`, `+++`, `new file mode`, `Binary files ... differ`. None of
            // it is ours to interpret.
            continue;
        }

        let header = raw.to_string();
        let (old_start, old_lines, new_start, new_lines) = parse_header(&header)?;

        // Skipping a page's worth of hunks still has to walk their bodies, so
        // that the next header is found at the right place.
        let keep = hunk_index >= from_hunk;

        let mut body: Vec<Line> = Vec::new();
        let mut old_no = old_start;
        let mut new_no = new_start;
        let mut seen_old = 0u32;
        let mut seen_new = 0u32;

        while seen_old < old_lines || seen_new < new_lines {
            let Some(l) = lines.peek() else { break };
            // The next hunk starts here: the body was short. Reported rather
            // than accepted, because a hunk whose body does not match its header
            // means the input is not what this parser thinks it is.
            if l.starts_with("@@") {
                break;
            }
            let l = lines.next().expect("peeked");

            // `\ No newline at end of file` annotates the line BEFORE it, and is
            // not itself a line of either side.
            if let Some(stripped) = l.strip_prefix('\\') {
                let _ = stripped;
                if let Some(last) = body.last_mut() {
                    last.no_newline = true;
                }
                continue;
            }

            let (kind, text) = match l.as_bytes().first() {
                Some(b' ') => (LineKind::Context, &l[1..]),
                Some(b'+') => (LineKind::Added, &l[1..]),
                Some(b'-') => (LineKind::Removed, &l[1..]),
                // An empty line in a patch is a context line whose content is
                // empty and whose leading space git chose not to emit. Common
                // enough in the wild that refusing it would refuse real diffs.
                None => (LineKind::Context, ""),
                Some(_) => return Err(DiffError::MalformedLine(l.to_string())),
            };

            let line = match kind {
                LineKind::Context => {
                    let v = Line {
                        kind,
                        old_no: Some(old_no),
                        new_no: Some(new_no),
                        text: text.to_string(),
                        no_newline: false,
                    };
                    old_no += 1;
                    new_no += 1;
                    seen_old += 1;
                    seen_new += 1;
                    v
                }
                LineKind::Added => {
                    let v = Line {
                        kind,
                        old_no: None,
                        new_no: Some(new_no),
                        text: text.to_string(),
                        no_newline: false,
                    };
                    new_no += 1;
                    seen_new += 1;
                    v
                }
                LineKind::Removed => {
                    let v = Line {
                        kind,
                        old_no: Some(old_no),
                        new_no: None,
                        text: text.to_string(),
                        no_newline: false,
                    };
                    old_no += 1;
                    seen_old += 1;
                    v
                }
            };
            body.push(line);
        }

        if !keep {
            hunk_index += 1;
            continue;
        }

        if hunks.len() >= limits::MAX_HUNKS {
            truncated = Some(Truncation::HunkCap);
            break;
        }
        if emitted_lines + body.len() > limits::MAX_LINES {
            truncated = Some(Truncation::LineCap);
            break;
        }
        emitted_lines += body.len();

        hunks.push(Hunk {
            index: hunk_index,
            fingerprint: fingerprint(path, &body),
            header,
            old_start,
            old_lines,
            new_start,
            new_lines,
            lines: body,
            });
        hunk_index += 1;
    }

    let next_hunk = truncated.and_then(|t| match t {
        Truncation::HunkCap | Truncation::LineCap => Some(hunk_index),
        _ => None,
    });

    Ok(FileDiff { path: path.to_string(), hunks, truncated, next_hunk })
}

fn parse_header(header: &str) -> Result<(u32, u32, u32, u32), DiffError> {
    // `@@ -12,7 +12,9 @@ optional trailing context`
    let bad = || DiffError::MalformedHeader(header.to_string());
    let rest = header.strip_prefix("@@ ").ok_or_else(bad)?;
    let end = rest.find(" @@").ok_or_else(bad)?;
    let ranges = &rest[..end];
    let parts: Vec<&str> = ranges.split(' ').collect();
    // Counted BEFORE parsing. A combined diff has one range per parent plus one,
    // and its second range starts with `-` where an ordinary header has `+`, so
    // parsing first reports a malformed header for what is really a merge — the
    // wrong error, and the caller would show a parse failure instead of falling
    // back to the first parent.
    if parts.len() > 2 {
        return Err(DiffError::CombinedDiff);
    }
    let old = parts.first().ok_or_else(bad)?.strip_prefix('-').ok_or_else(bad)?;
    let new = parts.get(1).ok_or_else(bad)?.strip_prefix('+').ok_or_else(bad)?;
    let (os, ol) = split_range(old).ok_or_else(bad)?;
    let (ns, nl) = split_range(new).ok_or_else(bad)?;
    Ok((os, ol, ns, nl))
}

/// `12,7` or bare `12`, which means a count of one.
fn split_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

/// Hash of the path and the changed lines. See [`Hunk::fingerprint`].
pub fn fingerprint(path: &str, lines: &[Line]) -> String {
    let mut h = Sha256::new();
    h.update(path.as_bytes());
    h.update([0]);
    for l in lines {
        if l.kind == LineKind::Context {
            continue;
        }
        h.update([l.kind.marker() as u8]);
        h.update(l.text.as_bytes());
        h.update([0]);
    }
    format!("{:x}", h.finalize())
}

/// Diff two whole texts.
///
/// The other way a diff arrives: an agent's tool call carries the file before
/// and after, with no patch and no git involved. This is the same computation
/// the Mac app used to do in Swift, moved here so the transcript and the review
/// surface cannot disagree about what changed — and so iOS and Android get it
/// without writing it twice more.
pub fn diff_texts(old: &str, new: &str) -> Vec<Line> {
    let a: Vec<&str> = split_lines(old);
    let b: Vec<&str> = split_lines(new);

    let script = myers(&a, &b);

    let mut out = Vec::with_capacity(script.len());
    let mut old_no = 1u32;
    let mut new_no = 1u32;
    for op in script {
        match op {
            Op::Equal(i) => {
                out.push(Line {
                    kind: LineKind::Context,
                    old_no: Some(old_no),
                    new_no: Some(new_no),
                    text: a[i].to_string(),
                    no_newline: false,
                });
                old_no += 1;
                new_no += 1;
            }
            Op::Remove(i) => {
                out.push(Line {
                    kind: LineKind::Removed,
                    old_no: Some(old_no),
                    new_no: None,
                    text: a[i].to_string(),
                    no_newline: false,
                });
                old_no += 1;
            }
            Op::Insert(j) => {
                out.push(Line {
                    kind: LineKind::Added,
                    old_no: None,
                    new_no: Some(new_no),
                    text: b[j].to_string(),
                    no_newline: false,
                });
                new_no += 1;
            }
        }
    }
    out
}

/// Split into lines WITHOUT swallowing a trailing empty line.
///
/// `str::lines` treats "a\n" and "a" as the same one-line input, which is
/// exactly the distinction a diff exists to show.
fn split_lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut v: Vec<&str> = s.split('\n').collect();
    // A trailing newline produces a final empty element that is not a line.
    if s.ends_with('\n') {
        v.pop();
    }
    v
}

enum Op {
    Equal(usize),
    Remove(usize),
    Insert(usize),
}

/// Beyond this many edits, stop looking for a minimal script.
///
/// Myers is O((N+M)·D): cheap when the two texts are similar, which is the case
/// this exists for, and quadratic when they are not. A generated file that was
/// entirely rewritten has D on the order of its length, and computing the
/// prettiest possible diff of two unrelated 50 000-line files is work nobody
/// asked for. Past the bound the answer is "all of it changed", which is both
/// true and what a reader would have concluded anyway.
const MAX_EDIT_DISTANCE: usize = 4_000;

/// Myers' greedy algorithm, forward pass with a recorded trace.
fn myers(a: &[&str], b: &[&str]) -> Vec<Op> {
    let n = a.len();
    let m = b.len();

    if n == 0 {
        return (0..m).map(Op::Insert).collect();
    }
    if m == 0 {
        return (0..n).map(Op::Remove).collect();
    }

    let max = (n + m).min(MAX_EDIT_DISTANCE);
    let offset = max as isize;
    // v[k + offset] = furthest x reached on diagonal k.
    let mut v = vec![0usize; 2 * max + 1];
    let mut trace: Vec<Vec<usize>> = Vec::new();

    for d in 0..=max {
        trace.push(v.clone());
        let dd = d as isize;
        let mut k = -dd;
        while k <= dd {
            let idx = (k + offset) as usize;
            let mut x = if k == -dd || (k != dd && v[idx - 1] < v[idx + 1]) {
                v[idx + 1]
            } else {
                v[idx - 1] + 1
            };
            let mut y = (x as isize - k) as usize;
            while x < n && y < m && a[x] == b[y] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                return backtrack(a, b, &trace, offset);
            }
            k += 2;
        }
    }

    // Bound exceeded: say the whole thing changed rather than half-solving it.
    let mut out: Vec<Op> = (0..n).map(Op::Remove).collect();
    out.extend((0..m).map(Op::Insert));
    out
}

fn backtrack(a: &[&str], b: &[&str], trace: &[Vec<usize>], offset: isize) -> Vec<Op> {
    let mut ops: Vec<Op> = Vec::new();
    let mut x = a.len();
    let mut y = b.len();

    for (d, v) in trace.iter().enumerate().rev() {
        // At d == 0 there is no previous diagonal to step back to: everything
        // remaining is the opening snake, straight back to the origin. Computing
        // a `prev_k` here reads `v` off the end of the reachable band and yields
        // a `prev_y` of -1, which underflows into a huge usize and silently
        // produces an empty edit script — two identical files diffing to nothing
        // at all rather than to all-context.
        if d == 0 {
            while x > 0 && y > 0 {
                x -= 1;
                y -= 1;
                ops.push(Op::Equal(x));
            }
            break;
        }

        let dd = d as isize;
        let k = x as isize - y as isize;
        let idx = (k + offset) as usize;

        let prev_k =
            if k == -dd || (k != dd && v[idx - 1] < v[idx + 1]) { k + 1 } else { k - 1 };
        let prev_idx = (prev_k + offset) as usize;
        let prev_x = v[prev_idx];
        let prev_y = (prev_x as isize - prev_k) as usize;

        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            ops.push(Op::Equal(x));
        }
        if x > prev_x {
            x -= 1;
            ops.push(Op::Remove(x));
        } else {
            y -= 1;
            ops.push(Op::Insert(y));
        }
    }

    ops.reverse();
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[Line]) -> Vec<char> {
        lines.iter().map(|l| l.kind.marker()).collect()
    }

    #[test]
    fn an_ordinary_hunk_numbers_both_sides() {
        let patch = "@@ -1,3 +1,4 @@\n one\n-two\n+TWO\n+two and a half\n three\n";
        let d = parse_unified("a.txt", patch).expect("parses");
        assert_eq!(d.hunks.len(), 1);
        let h = &d.hunks[0];
        assert_eq!(kinds(&h.lines), vec![' ', '-', '+', '+', ' ']);
        assert_eq!(h.lines[0].old_no, Some(1));
        assert_eq!(h.lines[0].new_no, Some(1));
        // The removed line has no position on the new side, and vice versa.
        assert_eq!(h.lines[1].new_no, None);
        assert_eq!(h.lines[2].old_no, None);
        // Context after the change is numbered past both insertions.
        assert_eq!(h.lines[4].old_no, Some(3));
        assert_eq!(h.lines[4].new_no, Some(4));
    }

    #[test]
    fn a_merge_commit_is_refused_rather_than_misparsed() {
        // What `git show` prints for a merge: three `@`, two columns.
        let patch = "@@@ -1,2 -1,2 +1,3 @@@\n  context\n++both added\n - only in one\n";
        assert_eq!(parse_unified("a.txt", patch), Err(DiffError::CombinedDiff));
    }

    #[test]
    fn a_three_range_header_is_also_a_combined_diff() {
        let patch = "@@ -1,2 -1,2 +1,3 @@\n context\n";
        assert_eq!(parse_unified("a.txt", patch), Err(DiffError::CombinedDiff));
    }

    #[test]
    fn a_missing_trailing_newline_is_carried_not_dropped() {
        let patch = "@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n";
        let d = parse_unified("a.txt", patch).expect("parses");
        let h = &d.hunks[0];
        assert_eq!(kinds(&h.lines), vec!['-', '+']);
        assert!(h.lines[0].no_newline, "the marker annotates the line before it");
        assert!(!h.lines[1].no_newline);
    }

    #[test]
    fn a_bare_range_means_one_line() {
        let patch = "@@ -7 +7 @@\n-a\n+b\n";
        let d = parse_unified("a.txt", patch).expect("parses");
        let h = &d.hunks[0];
        assert_eq!((h.old_start, h.old_lines), (7, 1));
        assert_eq!((h.new_start, h.new_lines), (7, 1));
    }

    #[test]
    fn an_empty_context_line_is_context_not_a_malformed_line() {
        // git omits the leading space on an empty context line.
        let patch = "@@ -1,3 +1,3 @@\n a\n\n-b\n+B\n";
        let d = parse_unified("a.txt", patch).expect("parses");
        assert_eq!(kinds(&d.hunks[0].lines), vec![' ', ' ', '-', '+']);
    }

    #[test]
    fn a_malformed_header_is_named_not_skipped() {
        let patch = "@@ nonsense @@\n a\n";
        assert!(matches!(
            parse_unified("a.txt", patch),
            Err(DiffError::MalformedHeader(_))
        ));
    }

    #[test]
    fn preamble_between_hunks_is_ignored() {
        let patch = "diff --git a/x b/x\nindex 111..222 100644\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        let d = parse_unified("x", patch).expect("parses");
        assert_eq!(d.hunks.len(), 1);
    }

    #[test]
    fn several_hunks_keep_their_own_numbering() {
        let patch = "@@ -1,2 +1,2 @@\n a\n-b\n+B\n@@ -10,2 +10,2 @@\n j\n-k\n+K\n";
        let d = parse_unified("x", patch).expect("parses");
        assert_eq!(d.hunks.len(), 2);
        assert_eq!(d.hunks[0].index, 0);
        assert_eq!(d.hunks[1].index, 1);
        assert_eq!(d.hunks[1].old_start, 10);
    }

    #[test]
    fn two_hunks_with_identical_changed_lines_do_not_collide_into_one() {
        // The same edit made twice in one file. Fingerprints MATCH — that is the
        // point: identity would be a lie, so a resolver has to see two matches
        // and call it ambiguous rather than pick the first.
        let patch = "@@ -1,2 +1,2 @@\n a\n-x\n+y\n@@ -50,2 +50,2 @@\n a\n-x\n+y\n";
        let d = parse_unified("x", patch).expect("parses");
        assert_eq!(d.hunks.len(), 2);
        assert_eq!(
            d.hunks[0].fingerprint, d.hunks[1].fingerprint,
            "a fingerprint is a content hash, not an identity"
        );
        assert_ne!(d.hunks[0].index, d.hunks[1].index, "index is what distinguishes them");
    }

    #[test]
    fn a_fingerprint_is_stable_when_the_code_around_it_moves() {
        let a = "@@ -1,3 +1,3 @@\n before\n-x\n+y\n";
        let b = "@@ -900,3 +900,3 @@\n something else entirely\n-x\n+y\n";
        let da = parse_unified("x", a).expect("parses");
        let db = parse_unified("x", b).expect("parses");
        assert_eq!(da.hunks[0].fingerprint, db.hunks[0].fingerprint);
    }

    #[test]
    fn a_fingerprint_is_scoped_to_its_path() {
        let patch = "@@ -1,2 +1,2 @@\n a\n-x\n+y\n";
        let a = parse_unified("one.rs", patch).expect("parses");
        let b = parse_unified("two.rs", patch).expect("parses");
        assert_ne!(a.hunks[0].fingerprint, b.hunks[0].fingerprint);
    }

    #[test]
    fn a_patch_over_the_byte_cap_truncates_rather_than_parsing() {
        let big = format!("@@ -1,1 +1,1 @@\n-{}\n+b\n", "x".repeat(limits::MAX_BYTES));
        let d = parse_unified("x", &big).expect("does not error");
        assert_eq!(d.truncated, Some(Truncation::ByteCap));
        assert!(d.hunks.is_empty());
    }

    #[test]
    fn too_many_hunks_truncates_and_hands_back_a_cursor() {
        let mut patch = String::new();
        for i in 0..(limits::MAX_HUNKS + 10) {
            patch.push_str(&format!("@@ -{},1 +{},1 @@\n-a\n+b\n", i * 10 + 1, i * 10 + 1));
        }
        let d = parse_unified("x", &patch).expect("parses");
        assert_eq!(d.truncated, Some(Truncation::HunkCap));
        assert_eq!(d.hunks.len(), limits::MAX_HUNKS);
        assert_eq!(d.next_hunk, Some(limits::MAX_HUNKS as u32));
    }

    #[test]
    fn paging_resumes_at_the_requested_hunk() {
        let patch = "@@ -1,2 +1,2 @@\n a\n-b\n+B\n@@ -10,2 +10,2 @@\n j\n-k\n+K\n";
        let d = parse_unified_from("x", patch, 1).expect("parses");
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].index, 1, "index stays absolute across pages");
        assert_eq!(d.hunks[0].old_start, 10);
    }

    // --- diff_texts, the DiffComputation replacement -----------------------

    #[test]
    fn two_identical_texts_are_all_context() {
        let d = diff_texts("a\nb\nc\n", "a\nb\nc\n");
        assert_eq!(kinds(&d), vec![' ', ' ', ' ']);
    }

    #[test]
    fn a_one_line_change_is_one_removal_and_one_insertion() {
        let d = diff_texts("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(kinds(&d), vec![' ', '-', '+', ' ']);
        assert_eq!(d[1].text, "b");
        assert_eq!(d[2].text, "B");
    }

    #[test]
    fn an_empty_old_text_is_all_insertion() {
        let d = diff_texts("", "a\nb\n");
        assert_eq!(kinds(&d), vec!['+', '+']);
        assert_eq!(d[0].new_no, Some(1));
        assert_eq!(d[1].new_no, Some(2));
    }

    #[test]
    fn an_empty_new_text_is_all_removal() {
        let d = diff_texts("a\nb\n", "");
        assert_eq!(kinds(&d), vec!['-', '-']);
    }

    #[test]
    fn a_trailing_newline_is_not_a_line_of_its_own() {
        // "a\n" and "a" are the same one line, but "a\n\n" has two.
        assert_eq!(diff_texts("a\n", "a\n").len(), 1);
        assert_eq!(diff_texts("a\n\n", "a\n\n").len(), 2);
        // Adding a trailing blank line shows up as one insertion.
        assert_eq!(kinds(&diff_texts("a\n", "a\n\n")), vec![' ', '+']);
    }

    #[test]
    fn line_numbers_advance_independently_on_each_side() {
        let d = diff_texts("a\nb\nc\nd\n", "a\nX\nY\nd\n");
        let olds: Vec<Option<u32>> = d.iter().map(|l| l.old_no).collect();
        let news: Vec<Option<u32>> = d.iter().map(|l| l.new_no).collect();
        // Every old line is numbered 1..4 across context+removals, and every
        // new line 1..4 across context+insertions, with no gaps in either.
        let old_seq: Vec<u32> = olds.into_iter().flatten().collect();
        let new_seq: Vec<u32> = news.into_iter().flatten().collect();
        assert_eq!(old_seq, vec![1, 2, 3, 4]);
        assert_eq!(new_seq, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_move_is_a_removal_and_an_insertion_not_a_rename() {
        // Myers has no concept of a moved line, and pretending otherwise would
        // be a different algorithm. Locked down so nobody "fixes" it by accident.
        let d = diff_texts("a\nb\n", "b\na\n");
        assert!(d.iter().any(|l| l.kind == LineKind::Removed));
        assert!(d.iter().any(|l| l.kind == LineKind::Added));
    }

    #[test]
    fn two_unrelated_texts_past_the_bound_are_reported_as_wholly_changed() {
        let a: String = (0..MAX_EDIT_DISTANCE + 100).map(|i| format!("a{i}\n")).collect();
        let b: String = (0..MAX_EDIT_DISTANCE + 100).map(|i| format!("b{i}\n")).collect();
        let d = diff_texts(&a, &b);
        assert!(d.iter().all(|l| l.kind != LineKind::Context));
        assert!(!d.is_empty());
    }

    #[test]
    fn a_realistic_edit_stays_minimal() {
        let old = "fn main() {\n    let x = 1;\n    println!(\"{}\", x);\n}\n";
        let new = "fn main() {\n    let x = 2;\n    println!(\"{}\", x);\n}\n";
        let d = diff_texts(old, new);
        let changed = d.iter().filter(|l| l.kind != LineKind::Context).count();
        assert_eq!(changed, 2, "one line changed means exactly one - and one +");
    }
}
