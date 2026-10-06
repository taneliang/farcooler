//! Following a projector (ov-366): its watch, its held hooks, its epoch.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_core::session_log::projector::RowKind;
use serde_json::json;
use uuid::Uuid;

use super::*;

const EDITS: &str = include_str!("../../core/src/session_log/projector/fixtures/edits.jsonl");
const BACKGROUND: &str = include_str!("../../core/src/session_log/projector/fixtures/background.jsonl");
const SESSION: &str = "b7a1c0de-0000-4000-8000-000000000001";
const AGENT_BG: &str = include_str!("../../core/src/session_log/projector/fixtures/b7a1c0de-0000-4000-8000-000000000001/subagents/agent-abg1.jsonl");
const AGENT_BG_META: &str =
    include_str!("../../core/src/session_log/projector/fixtures/b7a1c0de-0000-4000-8000-000000000001/subagents/agent-abg1.meta.json");

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
    let follow = projectors.follow_for(terminal, page.epoch, page.rev, Duration::from_secs(5)).await.unwrap();
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
    // `open`'s two halves, with two hooks between them.
    let guard = projectors.begin(terminal);
    projectors.hook(terminal, "UserPromptSubmit", &json!({"prompt_id":"p8","prompt":"While you were reading"}));
    projectors.hook(terminal, "UserPromptSubmit", &json!({"prompt_id":"p9","prompt":"And then this"}));
    let mut session = farcooler_core::session_log::projector::SessionProjector::open(canonical(&path));
    session.poll();
    let read = session.projection().rows().len() as u64;
    projectors.finish(guard, session);
    let rows = projectors.page(terminal, None, 1000).unwrap();
    let first = rows.iter().find(|r| r.id == "turn:p8").expect("the held hooks are applied, not dropped");
    let held = rows.iter().find(|r| r.id == "turn:p9").unwrap();
    assert!(held.provisional);
    assert_eq!((first.ord, held.ord), (read, read + 1), "after every row the rebuild read, in the order they came");
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
    let stale = projectors.follow_for(terminal, page.epoch + 1, page.rev, Duration::ZERO).await.unwrap();
    assert!(matches!(stale, Follow::Reset { .. }), "{stale:?}");
    let behind = projectors.follow_for(terminal, page.epoch, 0, Duration::ZERO).await.unwrap();
    assert!(matches!(behind, Follow::Changes { ref changes, .. } if changes.len() > 3), "a whole small session is still a follow: {behind:?}");
    // Further behind than a follow carries: told to page instead.
    let too_far = projectors.follow(terminal, page.epoch, 0, tokio::time::Instant::now(), 3).await.unwrap();
    assert!(matches!(too_far, Follow::Reset { .. }), "{too_far:?}");

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
    let quiet = projectors.follow_for(terminal, page.epoch, page.rev, Duration::from_millis(200)).await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(180));
    assert_eq!(quiet, Follow::Changes { epoch: page.epoch, rev: page.rev, changes: Vec::new() });
}

/// A subagent's own transcript is watched too: a record it appends reaches a
/// follower with no tick.
#[tokio::test]
async fn a_subagent_record_reaches_a_waiting_follower_without_a_tick() {
    let dir = Dir::new("watch-sub");
    let path = dir.file(&format!("{SESSION}.jsonl"), BACKGROUND);
    let subs = dir.0.join(SESSION).join("subagents");
    std::fs::create_dir_all(&subs).unwrap();
    let agent = subs.join("agent-abg1.jsonl");
    std::fs::write(&agent, AGENT_BG).unwrap();
    std::fs::write(subs.join("agent-abg1.meta.json"), AGENT_BG_META).unwrap();
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, path);
    let page = projectors.read_page(terminal, None, 100).unwrap();
    let writer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        append(&agent, json!({"type":"assistant","uuid":"sub-new","timestamp":"2026-10-06T10:00:30.000Z","message":{"content":[{"type":"tool_use","id":"toolu_s9","name":"Read","input":{"file_path":"NOTES.md"}}]}}));
        Instant::now()
    });
    let follow = projectors.follow_for(terminal, page.epoch, page.rev, Duration::from_secs(5)).await.unwrap();
    let got = Instant::now();
    let written = writer.await.unwrap();
    let Follow::Changes { changes, .. } = follow else { panic!("{follow:?}") };
    let sub = changes.iter().find_map(|c| match c {
        RowChange::Update(r) if r.id == "sub:toolu_bg" => Some(r),
        _ => None,
    });
    let sub = sub.unwrap_or_else(|| panic!("{changes:?}"));
    assert!(matches!(&sub.kind, RowKind::Subagent(s) if s.current_action == "Read NOTES.md"), "{sub:?}");
    assert!(got.duration_since(written) < Duration::from_millis(1000), "took {:?}", got.duration_since(written));
}

/// A second open while a build is under way waits for that build, and keeps
/// its projector, rather than reading the transcript again.
#[test]
fn a_second_open_waits_for_the_build_under_way() {
    let dir = Dir::new("second-open");
    let path = dir.file("s.jsonl", EDITS);
    let projectors = std::sync::Arc::new(SessionProjectors::default());
    let terminal = Uuid::now_v7();
    let guard = projectors.begin(terminal);
    let (done, opened) = std::sync::mpsc::channel();
    let second = {
        let (projectors, path) = (projectors.clone(), path.clone());
        std::thread::spawn(move || {
            projectors.open(terminal, path);
            let _ = done.send(());
        })
    };
    assert!(opened.recv_timeout(Duration::from_millis(300)).is_err(), "the second open built its own");
    let mut session = farcooler_core::session_log::projector::SessionProjector::open(canonical(&path));
    session.poll();
    projectors.finish(guard, session);
    let epoch = projectors.read_page(terminal, None, 1).unwrap().epoch;
    assert!(opened.recv_timeout(Duration::from_secs(5)).is_ok(), "the second open never woke");
    second.join().unwrap();
    assert_eq!(projectors.read_page(terminal, None, 1).unwrap().epoch, epoch, "the build under way is the one kept");
}

/// A build that dies (a panic in the read) stops holding hooks for its
/// terminal; and however long a build takes, it holds at most `MAX_HELD`.
#[test]
fn a_build_that_dies_holds_nothing_and_a_live_one_holds_a_bounded_number() {
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    let guard = projectors.begin(terminal);
    drop(guard);
    projectors.hook(terminal, "UserPromptSubmit", &json!({"prompt_id":"p1","prompt":"x"}));
    assert!(projectors.inner.building.lock().unwrap().is_empty(), "nothing held for a build that is gone");

    let dir = Dir::new("cap");
    let path = dir.file("s.jsonl", EDITS);
    let guard = projectors.begin(terminal);
    for n in 0..MAX_HELD + 10 {
        projectors.hook(terminal, "UserPromptSubmit", &json!({"prompt_id":format!("h{n}"),"prompt":"x"}));
    }
    assert_eq!(projectors.inner.building.lock().unwrap()[&terminal].len(), MAX_HELD);
    let mut session = farcooler_core::session_log::projector::SessionProjector::open(canonical(&path));
    session.poll();
    projectors.finish(guard, session);
    let held = projectors.page(terminal, None, 10_000).unwrap().iter().filter(|r| r.id.starts_with("turn:h")).count();
    assert_eq!(held, MAX_HELD);
}
