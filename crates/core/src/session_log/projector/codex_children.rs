//! Codex child threads (ov-879). Task paths identify collaboration roles;
//! only a native header's `id` identifies the rollout to open. Never join by
//! filename alone, inherited `session_id`, cwd, nickname, or a board record.

use std::collections::BTreeMap;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;
use super::{LineReader, Projection};
use super::files::Line;
use super::rows::*;

#[derive(Clone, Debug)]
struct Child {
    id: String,
    parent: String,
    task: String,
    path: PathBuf,
}

/// Header reads cannot follow a final symlink or block on a FIFO. Both the
/// byte bound and descriptor check matter when discovering private logs.
fn header(path: &Path) -> Option<Value> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    header_file(&file)
}

fn header_file(file: &std::fs::File) -> Option<Value> {
    let meta = file.metadata().ok()?;
    if !meta.is_file() { return None; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 { return None; }
    }
    let mut line = String::new();
    std::io::BufReader::new(file.try_clone().ok()?.take(1 << 20)).read_line(&mut line).ok()?;
    let record: Value = serde_json::from_str(&line).ok()?;
    (record.get("type")?.as_str()? == "session_meta").then(|| record.get("payload").cloned()).flatten()
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> { v.get(key)?.as_str() }

fn child(path: PathBuf) -> Option<Child> {
    let h = header(&path)?;
    child_header(path, &h)
}

fn child_header(path: PathBuf, h: &Value) -> Option<Child> {
    let spawn = h.pointer("/source/subagent/thread_spawn")?;
    let id = text(h, "id")?;
    uuid::Uuid::parse_str(id).ok()?;
    let parent = text(spawn, "parent_thread_id")?;
    uuid::Uuid::parse_str(parent).ok()?;
    if text(h, "thread_source") != Some("subagent")
        || text(h, "parent_thread_id").is_some_and(|p| p != parent)
        || text(h, "forked_from_id").is_some_and(|p| p != parent)
        || text(h, "forked_from_id").is_some() && h.get("subagent_history_start_ordinal").and_then(Value::as_u64).is_none()
    { return None; }
    let task = text(spawn, "agent_path")?;
    if !task.starts_with("/root/") || task.len() > 512 { return None; }
    Some(Child { id: id.into(), parent: parent.into(), task: task.into(),
        path })
}

/// A drill-in reader validates the descriptor it will actually consume,
/// including after a rewrite. Pathname checks alone leave a replacement race.
#[derive(Debug)]
pub(super) struct Binding {
    parent: PathBuf,
    parent_id: Option<String>,
    id: Option<String>,
    history_start: Option<u64>,
}

impl Binding {
    pub(super) fn new(path: &Path, parent: PathBuf) -> Self {
        let parent_id = header(&parent).and_then(|h| text(&h, "id").map(str::to_string));
        let id = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.get(s.len().checked_sub(36)?..))
            .filter(|s| uuid::Uuid::parse_str(s).is_ok()).map(str::to_string);
        let history_start = header(path).and_then(|h| h.get("subagent_history_start_ordinal").and_then(Value::as_u64));
        Self { parent, parent_id, id, history_start }
    }

    pub(super) fn permits(&self, file: &std::fs::File, path: &Path) -> bool {
        let Some(h) = header_file(file) else { return false };
        let Some(c) = child_header(path.to_path_buf(), &h) else { return false };
        self.id.as_deref() == Some(c.id.as_str()) && self.parent_id.as_deref() == Some(c.parent.as_str())
            && h.get("subagent_history_start_ordinal").and_then(Value::as_u64) == self.history_start
            && header(&self.parent).and_then(|h| text(&h, "id").map(str::to_string)) == self.parent_id
    }
}

/// Completed headers are shared across panes and drill-in RPCs. Appends
/// invalidate only that file; the finite cache cannot grow with an unbounded
/// archive. Binding validation deliberately bypasses this discovery cache.
fn cached_child(path: PathBuf) -> Option<Child> {
    use std::sync::{Mutex, OnceLock};
    type Version = (u64, Option<std::time::SystemTime>);
    type Cache = BTreeMap<PathBuf, (Version, Option<Child>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() { return None; }
    let version = (meta.len(), meta.modified().ok());
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, value)) = cache.get(&path).filter(|(held, _)| *held == version) {
        return value.clone();
    }
    let value = child(path.clone());
    if cache.len() >= 16384 { cache.clear(); }
    cache.insert(path, (version, value.clone()));
    value
}

/// Walk only the producer's dated sessions tree, with finite work and no
/// directory symlinks. This also works for a pane with a custom CODEX_HOME.
fn children(main: &Path) -> Vec<Child> {
    let Some(h) = header(main) else { return Vec::new() };
    let Some(parent) = text(&h, "id") else { return Vec::new() };
    let Some(root) = main.ancestors().nth(4).filter(|p| p.file_name().is_some_and(|n| n == "sessions")) else { return Vec::new() };
    let since = main.parent().and_then(|p| p.strip_prefix(root).ok()).map(Path::to_path_buf);
    let mut dirs = vec![(root.to_path_buf(), 0)];
    let mut found = Vec::new();
    let mut examined = 0;
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            examined += 1;
            if examined > 16384 { return Vec::new(); }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() && depth < 3 && entry.file_name().to_str().is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit())) {
                let prefix: PathBuf = since.as_ref().map(|p| p.components().take(depth + 1).collect()).unwrap_or_default();
                if entry.path().strip_prefix(root).is_ok_and(|p| p >= prefix.as_path()) {
                    dirs.push((entry.path(), depth + 1));
                }
            } else if kind.is_file() && depth == 3 && entry.file_name().to_str().is_some_and(|s| s.starts_with("rollout-") && s.ends_with(".jsonl")) {
                if let Some(c) = cached_child(entry.path()).filter(|c| c.parent == parent && c.id != parent) { found.push(c); }
            }
        }
    }
    found
}

/// Resolve a native id or a legacy registered task path only when exactly
/// one direct child proves it. Reused task paths are ambiguous, not aliases
/// for whichever file happens to be newest.
pub(super) fn transcript(main: &Path, agent: &str) -> Option<PathBuf> {
    let mut matches = children(main).into_iter().filter(|c| c.id == agent || c.task == agent);
    let first = matches.next()?;
    matches.next().is_none().then_some(first.path)
}

#[derive(Default, Debug)]
pub(super) struct Children {
    scanned: Option<Instant>,
    files: BTreeMap<String, (Child, LineReader, Projection)>,
}

impl Children {
    pub(super) fn poll(&mut self, main: &Path, parent: &mut Projection) -> u64 {
        if self.scanned.is_none_or(|at| at.elapsed() >= Duration::from_secs(2)) {
            for c in children(main) {
                self.files.entry(c.id.clone()).or_insert_with(|| {
                    let mut reader = LineReader::new(c.path.clone());
                    reader.require_binding(Binding::new(&c.path, main.to_path_buf()));
                    let projection = Projection::for_session(&c.id);
                    (c, reader, projection)
                });
            }
            self.scanned = Some(Instant::now());
        }
        let mut folded = 0;
        for (c, reader, projection) in self.files.values_mut() {
            // Recheck the binding on every poll, including after file replacement.
            if !child(c.path.clone()).is_some_and(|now| now.id == c.id && now.parent == c.parent) { continue; }
            loop {
                let report = reader.read(|line| match line {
                    Line::Complete(bytes) => projection.fold_codex_line(bytes),
                    Line::Restart => *projection = Projection::for_session(&c.id),
                    Line::TooLarge(_) => {},
                });
                folded += report.lines;
                if !report.more || report.bytes == 0 { break; }
            }
            let turn = projection.rows().iter().rev().find_map(|r| match &r.kind { RowKind::Turn(t) => Some(t), _ => None });
            let Some(turn) = turn else { continue };
            let status = match turn.outcome {
                None => SubagentState::Running,
                Some(TurnOutcome::Finished) => SubagentState::Completed,
                Some(_) => SubagentState::Stopped,
            };
            let sub = Subagent {
                tool_use_id: c.task.clone(), agent_id: Some(c.id.clone()), agent_type: "codex".into(),
                type_words: "Codex".into(), description: c.task.clone(), background: true, status,
                model: projection.codex.model.clone(), started_ms: turn.started_ms, ended_ms: turn.ended_ms,
                last_ms: projection.codex.last_at, ..Subagent::default()
            };
            let id = format!("sub:codex:{}", c.id);
            if let Some(&i) = parent.index.get(&id) {
                if parent.rows[i].kind != RowKind::Subagent(sub.clone()) {
                    parent.rows[i].kind = RowKind::Subagent(sub);
                    parent.touch(i);
                    parent.count_background(i);
                }
            } else {
                // A long-lived orchestrator can work many tickets. The native
                // launch attribution, not whichever parent turn is newest at
                // rebuild time, places the child under its actual work.
                let turn = projection.codex.parent_turn.as_ref()
                    .and_then(|turn| parent.index.get(&format!("turn:{turn}"))).copied();
                let i = parent.push(id, turn, false, RowKind::Subagent(sub));
                parent.count_background(i);
            }
        }
        folded
    }
}
