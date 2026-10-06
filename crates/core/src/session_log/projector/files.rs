//! Reading a session's files as claude appends to them.
//!
//! One byte offset per file and complete lines only, as the spike did. What a
//! line reader has to get right here:
//!
//! - **The half-written last line.** Claude is often mid-`write()` when this
//!   reads, and a truncated JSON record can still parse. Bytes after the last
//!   newline are held, not folded, until their newline arrives.
//! - **The line cap.** The largest line seen in a real log is 1.35 MB (a base64
//!   image), and that line still carries a tool result the view needs, so the
//!   cap sits well above it. A line over the cap is skipped as it streams past,
//!   never held whole, and becomes one `Gap`.
//! - **A file that shrank or was replaced** is read again from its start, with
//!   a `Gap` saying so.
//!
//! The files are claude's: `<project>/<session>.jsonl`, and beside it
//! `<session>/subagents/agent-<id>.jsonl` with `agent-<id>.meta.json`. Main
//! file first, then each subagent file, each in its own line order.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::fold::Projection;
use super::record::SubagentMeta;

/// A line over this is skipped unread. Six times the largest line ever seen.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// The most one `read` call takes from a file, so a first read of a 100 MB
/// session is folded in slices rather than held whole.
const READ_SLICE: usize = 4 * 1024 * 1024;

/// What one call to `LineReader::read` found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReadReport {
    pub lines: u64,
    pub bytes: u64,
    /// The file had less in it than was already read, or was a different
    /// file, and reading began again at 0.
    pub rewritten: bool,
    /// Whether bytes after the last newline are being held for later.
    pub holding: bool,
    /// More is waiting than one slice took.
    pub more: bool,
}

/// One line, as the reader hands it over.
#[derive(Debug, PartialEq, Eq)]
pub enum Line<'a> {
    Complete(&'a [u8]),
    /// Skipped for its size, which is given.
    TooLarge(u64),
    /// The file shrank or was replaced, and what follows is read from its
    /// start. Handed over before the first line of the new read.
    Restart,
}

/// Follows one append-only file, a line at a time.
#[derive(Debug)]
pub struct LineReader {
    path: PathBuf,
    /// Bytes read so far, partial line included.
    offset: u64,
    /// The bytes after the last newline, held until it arrives.
    partial: Vec<u8>,
    /// Bytes of an over-cap line skipped so far, while one is streaming past.
    skipping: Option<u64>,
    identity: Option<(u64, u64)>,
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

impl LineReader {
    /// From the start of the file: the projection is rebuilt from disk, so the
    /// whole record is read once.
    pub fn new(path: PathBuf) -> LineReader {
        LineReader { path, offset: 0, partial: Vec::new(), skipping: None, identity: None }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Bytes held for a line whose newline has not arrived. Never more than
    /// `MAX_LINE_BYTES`: an over-cap line is counted past, not kept.
    pub fn held_bytes(&self) -> usize {
        self.partial.len()
    }

    /// Hand every complete line appended since the last call to `each`, in
    /// file order. A missing file is nothing yet, not an error.
    pub fn read(&mut self, mut each: impl FnMut(Line<'_>)) -> ReadReport {
        let mut report = ReadReport::default();
        let Ok(mut file) = File::open(&self.path) else { return report };
        let Ok(meta) = file.metadata() else { return report };
        let identity = identity_of(&meta);
        let replaced = self.identity.is_some() && identity != self.identity;
        self.identity = identity;
        if replaced || meta.len() < self.offset {
            self.offset = 0;
            self.partial.clear();
            self.skipping = None;
            report.rewritten = true;
            each(Line::Restart);
        }
        let want = meta.len().saturating_sub(self.offset);
        if want == 0 {
            report.holding = !self.partial.is_empty() || self.skipping.is_some();
            return report;
        }
        let take = want.min(READ_SLICE as u64) as usize;
        report.more = want > take as u64;
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return report;
        }
        let mut chunk = vec![0u8; take];
        let mut got = 0;
        while got < take {
            match file.read(&mut chunk[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        chunk.truncate(got);
        self.offset += got as u64;
        report.bytes = got as u64;
        // A short read (an error, or the file cut while reading) is not "more
        // waiting": the next poll looks again, rather than this one spinning.
        report.more = report.more && got == take;

        let mut rest = &chunk[..];
        while let Some(newline) = rest.iter().position(|&b| b == b'\n') {
            let (line, after) = rest.split_at(newline);
            rest = &after[1..];
            report.lines += 1;
            if let Some(skipped) = self.skipping.take() {
                each(Line::TooLarge(skipped + line.len() as u64));
                continue;
            }
            if self.partial.is_empty() {
                if line.len() > MAX_LINE_BYTES {
                    each(Line::TooLarge(line.len() as u64));
                } else if !line.iter().all(u8::is_ascii_whitespace) {
                    each(Line::Complete(line));
                }
            } else if self.partial.len() + line.len() > MAX_LINE_BYTES {
                each(Line::TooLarge((self.partial.len() + line.len()) as u64));
                self.partial.clear();
            } else {
                self.partial.extend_from_slice(line);
                let whole = std::mem::take(&mut self.partial);
                if !whole.iter().all(u8::is_ascii_whitespace) {
                    each(Line::Complete(&whole));
                }
            }
        }
        // What is left has no newline yet: hold it, or count it toward a skip.
        if let Some(skipped) = &mut self.skipping {
            *skipped += rest.len() as u64;
        } else if self.partial.len() + rest.len() > MAX_LINE_BYTES {
            self.skipping = Some((self.partial.len() + rest.len()) as u64);
            self.partial = Vec::new();
        } else {
            self.partial.extend_from_slice(rest);
        }
        report.holding = !self.partial.is_empty() || self.skipping.is_some();
        report
    }
}

/// One subagent's transcript and the meta file that joins it.
#[derive(Debug)]
struct SubagentFile {
    reader: LineReader,
    meta_path: PathBuf,
    meta: Option<SubagentMeta>,
}

/// A session's projection and the files it is folded from.
#[derive(Debug)]
pub struct SessionProjector {
    projection: Projection,
    main: LineReader,
    subagents_dir: PathBuf,
    /// By agent id, so the fold order is the same on every run.
    subagents: BTreeMap<String, SubagentFile>,
    /// Lines held back as half-written, summed over every read. For the
    /// benchmark and logs.
    pub held_back: u64,
}

impl SessionProjector {
    /// `transcript` is `<project>/<session>.jsonl`.
    pub fn open(transcript: PathBuf) -> SessionProjector {
        let session = transcript.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let subagents_dir = transcript.with_file_name(&session).join("subagents");
        SessionProjector {
            projection: Projection::for_session(&session),
            main: LineReader::new(transcript),
            subagents_dir,
            subagents: BTreeMap::new(),
            held_back: 0,
        }
    }

    pub fn projection(&self) -> &Projection {
        &self.projection
    }

    pub fn projection_mut(&mut self) -> &mut Projection {
        &mut self.projection
    }

    pub fn transcript(&self) -> &Path {
        self.main.path()
    }

    /// What the main transcript's reader is holding for an unfinished line.
    pub fn main_held_bytes(&self) -> usize {
        self.main.held_bytes()
    }

    /// Move to another session's transcript (`/clear`, or a `SessionStart`
    /// naming a session this was not reading), keeping the rows so far.
    pub fn rebind(&mut self, transcript: PathBuf) {
        if transcript == self.main.path() {
            return;
        }
        // Every file is read from its start again, the old one too if this
        // comes back to it (`/resume`), so lines are counted afresh.
        self.projection.reread(None);
        let session = transcript.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.subagents_dir = transcript.with_file_name(&session).join("subagents");
        self.subagents.clear();
        self.main = LineReader::new(transcript);
    }

    /// Read whatever every file gained and fold it. Returns the lines folded.
    pub fn poll(&mut self) -> u64 {
        let mut folded = self.poll_main();
        self.discover_subagents();
        let agents: Vec<String> = self.subagents.keys().cloned().collect();
        folded += self.poll_subagents(&agents);
        folded
    }

    /// Read only the files among `paths` (what a watch said changed), and any
    /// subagent file that is new. The tick's `poll` still reads every file,
    /// for a change a watch missed. A session with two thousand subagents
    /// then costs a watch event one read, not two thousand.
    pub fn poll_paths(&mut self, paths: &[PathBuf]) -> u64 {
        let mut folded = 0;
        if paths.iter().any(|p| p == self.main.path()) {
            folded += self.poll_main();
        }
        // Most events in a project directory are another session's.
        if !paths.iter().any(|p| p.starts_with(&self.subagents_dir)) {
            return folded;
        }
        let known: std::collections::HashSet<String> = self.subagents.keys().cloned().collect();
        self.discover_subagents();
        let agents: Vec<String> = self
            .subagents
            .iter()
            .filter(|(agent, file)| !known.contains(*agent) || paths.iter().any(|p| p == file.reader.path() || *p == file.meta_path))
            .map(|(agent, _)| agent.clone())
            .collect();
        folded += self.poll_subagents(&agents);
        folded
    }

    /// The directories a watch on this session covers: the main file's, and
    /// its subagents' (which may not exist yet).
    pub fn watched_dirs(&self) -> (PathBuf, PathBuf) {
        let main = self.main.path().parent().map(Path::to_path_buf).unwrap_or_default();
        (main, self.subagents_dir.clone())
    }

    fn poll_main(&mut self) -> u64 {
        let mut folded = 0;
        loop {
            let report = Self::drain(&mut self.main, &mut self.projection, None);
            folded += report.lines;
            self.held_back += u64::from(report.holding);
            if !report.more || report.bytes == 0 {
                break;
            }
        }
        folded
    }

    fn poll_subagents(&mut self, agents: &[String]) -> u64 {
        let mut folded = 0;
        for (agent, file) in self.subagents.iter_mut().filter(|(agent, _)| agents.contains(agent)) {
            if file.meta.is_none() {
                file.meta = std::fs::read(&file.meta_path).ok().and_then(|b| serde_json::from_slice(&b).ok());
            }
            // Every poll until it takes: the meta file can be written before
            // the parent's `Agent` call is, and then there is no row to join.
            if let Some(meta) = file.meta.as_ref().filter(|_| !self.projection.is_joined(agent)) {
                self.projection.join_by_meta(agent, meta);
            }
            loop {
                let report = Self::drain(&mut file.reader, &mut self.projection, Some((agent, file.meta.as_ref())));
                folded += report.lines;
                self.held_back += u64::from(report.holding);
                if !report.more || report.bytes == 0 {
                    break;
                }
            }
        }
        // A nested agent's meta names an `Agent` call in its parent's file,
        // which may sort after it and so be read after it: join it now,
        // rather than a tick later. Again while any joins, for a deeper one.
        loop {
            let unjoined: Vec<(String, SubagentMeta)> = self
                .subagents
                .iter()
                .filter(|(agent, _)| !self.projection.is_joined(agent))
                .filter_map(|(agent, file)| file.meta.clone().map(|m| (agent.clone(), m)))
                .collect();
            let before = unjoined.len();
            for (agent, meta) in &unjoined {
                self.projection.join_by_meta(agent, meta);
            }
            let after = self.subagents.keys().filter(|a| !self.projection.is_joined(a)).count();
            if after == 0 || after >= before {
                break;
            }
        }
        folded
    }

    fn drain(reader: &mut LineReader, projection: &mut Projection, agent: Option<(&str, Option<&SubagentMeta>)>) -> ReadReport {
        let report = reader.read(|line| match (line, agent) {
            (Line::Complete(bytes), None) => projection.fold_line(bytes),
            (Line::Complete(bytes), Some((agent, meta))) => projection.fold_subagent_line(agent, meta, bytes),
            (Line::TooLarge(n), None) => projection.fold_too_large(n),
            (Line::TooLarge(_), Some(_)) => {}
            (Line::Restart, agent) => projection.reread(Some(agent.map_or("", |(a, _)| a))),
        });
        if report.rewritten && agent.is_none() {
            projection.fold_rewritten();
        }
        report
    }

    /// Start following any `agent-<id>.jsonl` not yet followed.
    fn discover_subagents(&mut self) {
        let Ok(entries) = std::fs::read_dir(&self.subagents_dir) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(agent) = name.strip_prefix("agent-").and_then(|n| n.strip_suffix(".jsonl")) else { continue };
            if self.subagents.contains_key(agent) {
                continue;
            }
            let meta_path = self.subagents_dir.join(format!("agent-{agent}.meta.json"));
            self.subagents.insert(
                agent.to_string(),
                SubagentFile { reader: LineReader::new(entry.path()), meta_path, meta: None },
            );
        }
    }
}
