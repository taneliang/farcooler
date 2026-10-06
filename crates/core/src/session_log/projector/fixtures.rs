//! Fixtures and helpers for the projector's tests.
//!
//! `recorded-queued-image.jsonl` is a recorded haiku session (claude 2.1.290,
//! sandbox HOME, Oct 6) with its `attachment` and `cost-state` records cut: a
//! queued message, an image prompt and five synthetic replies (the sandbox's
//! login was revoked). The rest are hand-built from shapes recorded in the
//! same sandbox (`toolUseResult` for an async agent, a task notification, a
//! `structuredPatch`, `queue-operation`, `turn_duration`) and from claude's
//! documented compaction records, because that login could not run a tool,
//! subagent or compaction turn live.

use std::path::{Path, PathBuf};

use super::fold::Projection;
use super::rows::*;

pub const RECORDED: &str = include_str!("fixtures/recorded-queued-image.jsonl");
pub const BACKGROUND: &str = include_str!("fixtures/background.jsonl");
pub const BACKGROUND_SESSION: &str = "b7a1c0de-0000-4000-8000-000000000001";
pub const AGENT_BG: &str = include_str!("fixtures/b7a1c0de-0000-4000-8000-000000000001/subagents/agent-abg1.jsonl");
pub const AGENT_BG_META: &str = include_str!("fixtures/b7a1c0de-0000-4000-8000-000000000001/subagents/agent-abg1.meta.json");
pub const AGENT_FG: &str = include_str!("fixtures/b7a1c0de-0000-4000-8000-000000000001/subagents/agent-afg1.jsonl");
pub const AGENT_FG_META: &str = include_str!("fixtures/b7a1c0de-0000-4000-8000-000000000001/subagents/agent-afg1.meta.json");
pub const EDITS: &str = include_str!("fixtures/edits.jsonl");
pub const COMPACT: &str = include_str!("fixtures/compact.jsonl");
pub const NONMONOTONIC: &str = include_str!("fixtures/nonmonotonic.jsonl");
pub const UNKNOWN: &str = include_str!("fixtures/unknown.jsonl");
pub const CLEARED_BEFORE: &str = include_str!("fixtures/cleared-before.jsonl");
pub const CLEARED_AFTER: &str = include_str!("fixtures/cleared-after.jsonl");

// Synthetic, in the shapes real transcripts take (field names and record
// order read from ~/.claude/projects, every word invented). One per finding of
// the first review: real files are the owner's and are never copied here.
pub const META_PROMPT: &str = include_str!("fixtures/shapes/meta-prompt.jsonl");
pub const QUEUED_NOTIFICATION: &str = include_str!("fixtures/shapes/queued-notification.jsonl");
pub const QUEUE: &str = include_str!("fixtures/shapes/queue.jsonl");
pub const ERRORS_AND_GAPS: &str = include_str!("fixtures/shapes/errors-and-gaps.jsonl");

/// A directory under the system temp dir, removed when dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Scratch {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("fc-projector-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every line of `text` folded as a main transcript.
pub fn fold(text: &str) -> Projection {
    let mut p = Projection::new();
    for line in text.lines() {
        p.fold_line(line.as_bytes());
    }
    p
}

/// The background fixture laid out as claude writes it, in `dir`. Returns the
/// main transcript's path.
pub fn write_background(dir: &Path) -> PathBuf {
    let main = dir.join(format!("{BACKGROUND_SESSION}.jsonl"));
    std::fs::write(&main, BACKGROUND).unwrap();
    let subs = dir.join(BACKGROUND_SESSION).join("subagents");
    std::fs::create_dir_all(&subs).unwrap();
    std::fs::write(subs.join("agent-abg1.jsonl"), AGENT_BG).unwrap();
    std::fs::write(subs.join("agent-abg1.meta.json"), AGENT_BG_META).unwrap();
    std::fs::write(subs.join("agent-afg1.jsonl"), AGENT_FG).unwrap();
    std::fs::write(subs.join("agent-afg1.meta.json"), AGENT_FG_META).unwrap();
    main
}

pub fn turns(p: &Projection) -> Vec<(&Row, &Turn)> {
    p.rows()
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Turn(t) => Some((r, t)),
            _ => None,
        })
        .collect()
}

pub fn turn<'p>(p: &'p Projection, id: &str) -> &'p Turn {
    match &p.row(id).unwrap_or_else(|| panic!("no row {id}")).kind {
        RowKind::Turn(t) => t,
        other => panic!("{id} is {other:?}"),
    }
}

pub fn tool<'p>(p: &'p Projection, id: &str) -> &'p Tool {
    match &p.row(id).unwrap_or_else(|| panic!("no row {id}")).kind {
        RowKind::Tool(t) => t,
        other => panic!("{id} is {other:?}"),
    }
}

pub fn sub<'p>(p: &'p Projection, id: &str) -> &'p Subagent {
    match &p.row(id).unwrap_or_else(|| panic!("no row {id}")).kind {
        RowKind::Subagent(s) => s,
        other => panic!("{id} is {other:?}"),
    }
}

pub fn prose(p: &Projection) -> Vec<(&Row, &Prose)> {
    p.rows()
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Prose(t) => Some((r, t)),
            _ => None,
        })
        .collect()
}

/// Milliseconds since the epoch for `2026-10-06T<hms>Z`.
pub fn ms(hms: &str) -> i64 {
    crate::session_log::claude::parse_iso8601_millis(&format!("2026-10-06T{hms}Z")).unwrap()
}
