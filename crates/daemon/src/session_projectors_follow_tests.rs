//! Following a projector (ov-366): its watch, its held hooks, its epoch.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_core::session_log::projector::RowKind;
use serde_json::json;
use uuid::Uuid;

use super::*;

const EDITS: &str = include_str!("../../core/src/session_log/projector/fixtures/edits.jsonl");

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("fc-follow-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn append(path: &Path, line: serde_json::Value) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(f, "{line}").unwrap();
}

fn prompt(id: &str) -> serde_json::Value {
    json!({"type":"user","promptId":id,"promptSource":"typed","uuid":format!("u-{id}"),"timestamp":"2026-10-06T11:02:00.000Z","message":{"content":"next"}})
}

/// A record claude appends is read by the projector's watch and handed to a
/// waiting follower, with no tick: the 300 ms budget the tick's 1 s could
/// never meet.
#[tokio::test]
async fn a_record_appended_reaches_a_waiting_follower_without_a_tick() {
    let dir = Dir::new("watch");
    let path = dir.file("s.jsonl", EDITS);
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, path.clone());
    let page = projectors.read_page(terminal, None, 100).unwrap();
    let writer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        append(&path, prompt("p9"));
        Instant::now()
    });
    let follow = projectors.follow(terminal, page.epoch, page.rev, Duration::from_secs(5)).await.unwrap();
    let got = Instant::now();
    let written = writer.await.unwrap();
    let Follow::Changes { changes, .. } = follow else { panic!("{follow:?}") };
    assert!(changes.iter().any(|c| matches!(c, RowChange::Insert(r) if r.id == "turn:p9")), "{changes:?}");
    assert!(got.duration_since(written) < Duration::from_millis(1000), "took {:?}", got.duration_since(written));
}

/// Hooks that arrive while a terminal's first projector is being read wait
/// for the read, and then take their places after every row it read.
#[test]
fn a_hook_during_a_rebuild_is_applied_after_it_never_ahead_of_older_turns() {
    let dir = Dir::new("held");
    let path = dir.file("s.jsonl", EDITS);
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    // `open`'s two halves, with a hook between them.
    projectors.building.lock().unwrap().insert(terminal, Vec::new());
    projectors.hook(terminal, "UserPromptSubmit", &json!({"prompt_id":"p9","prompt":"While you were reading"}));
    let mut session = farcooler_core::session_log::projector::SessionProjector::open(canonical(&path));
    session.poll();
    let read = session.projection().rows().len() as u64;
    projectors.finish(terminal, session);
    let rows = projectors.page(terminal, None, 1000).unwrap();
    let held = rows.iter().find(|r| r.id == "turn:p9").expect("the held hook is applied, not dropped");
    assert!(held.provisional);
    assert_eq!(held.ord, read, "after every row the rebuild read");
    let older_turns = rows.iter().filter(|r| matches!(r.kind, RowKind::Turn(_)) && r.id != "turn:p9").count();
    assert!(older_turns >= 2 && rows.iter().all(|r| r.id == "turn:p9" || r.ord < held.ord));
}

#[tokio::test]
async fn a_follower_of_another_projection_is_told_to_page_again() {
    let dir = Dir::new("epoch");
    let path = dir.file("s.jsonl", EDITS);
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, path.clone());
    let page = projectors.read_page(terminal, None, 3).unwrap();
    assert_eq!(page.rows.len(), 3);
    assert!(page.more_before);
    let stale = projectors.follow(terminal, page.epoch + 1, page.rev, Duration::ZERO).await.unwrap();
    assert!(matches!(stale, Follow::Reset { .. }), "{stale:?}");
    let behind = projectors.follow(terminal, page.epoch, 0, Duration::ZERO).await.unwrap();
    assert!(matches!(behind, Follow::Changes { ref changes, .. } if changes.len() > 3), "a whole small session is still a follow: {behind:?}");

    // Reopened (a daemon restart is a new process, so a new projector): the
    // same rows by id, in another epoch.
    projectors.forget(terminal);
    projectors.open(terminal, path);
    let again = projectors.read_page(terminal, None, 3).unwrap();
    assert_ne!(again.epoch, page.epoch);
    let ids = |p: &Page| p.rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&again), ids(&page));
}

#[tokio::test]
async fn a_quiet_follow_waits_then_answers_nothing() {
    let dir = Dir::new("quiet");
    let path = dir.file("s.jsonl", EDITS);
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, path);
    let page = projectors.read_page(terminal, None, 100).unwrap();
    let started = Instant::now();
    let quiet = projectors.follow(terminal, page.epoch, page.rev, Duration::from_millis(200)).await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(180));
    assert_eq!(quiet, Follow::Changes { epoch: page.epoch, rev: page.rev, changes: Vec::new() });
}
