//! The daemon's projectors, fed the way the daemon feeds them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use farcooler_agent_hooks::Agent;
use farcooler_agent_hooks::assemble::MessageAssembler;
use farcooler_agent::event::{AgentEvent, Role};
use farcooler_core::session_log::projector::{Activity, RowKind, TurnOutcome};
use serde_json::json;
use uuid::Uuid;

use super::*;

const EDITS: &str = include_str!("../../core/src/session_log/projector/fixtures/edits.jsonl");
const CLEARED_AFTER: &str = include_str!("../../core/src/session_log/projector/fixtures/cleared-after.jsonl");

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("fc-projectors-{tag}-{}", std::process::id()));
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

fn kinds(projectors: &SessionProjectors, terminal: Uuid) -> Vec<(String, bool)> {
    projectors.page(terminal, None, 1000).unwrap().into_iter().map(|r| (r.id, r.provisional)).collect()
}

#[test]
fn an_opened_projector_is_rebuilt_from_disk_and_follows_hooks_then_the_file() {
    let dir = Dir::new("follow");
    let path = dir.file("s.jsonl", EDITS);
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, path.clone());
    assert_eq!(projectors.page(terminal, None, 1000).unwrap().len(), 8, "the whole file, from its start");

    projectors.hook(terminal, "UserPromptSubmit", &json!({"prompt_id":"p3","prompt":"And one more thing"}));
    assert!(kinds(&projectors, terminal).contains(&("turn:p3".into(), true)), "up at once, provisional");

    append(&path, json!({"type":"user","promptId":"p3","promptSource":"typed","timestamp":"2026-10-06T11:02:00.000Z","message":{"content":"And one more thing"}}));
    projectors.tick(terminal, Some(Activity::Busy));
    assert!(kinds(&projectors, terminal).contains(&("turn:p3".into(), false)), "confirmed by the tick's read");
    let newest = projectors.page(terminal, None, 1000).unwrap().into_iter().find(|r| r.id == "turn:p3").unwrap();
    let RowKind::Turn(turn) = newest.kind else { panic!() };
    assert_eq!(turn.activity, Some(Activity::Busy), "the registry's status, on the newest turn");
}

#[test]
fn a_follower_reads_only_what_changed_since_its_revision() {
    let dir = Dir::new("since");
    let path = dir.file("s.jsonl", EDITS);
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, path);
    let (all, rev) = projectors.changed_since(terminal, 0).unwrap();
    assert!(!all.is_empty());
    projectors.hook(terminal, "UserPromptSubmit", &json!({"prompt_id":"p3","prompt":"next"}));
    let (changed, later) = projectors.changed_since(terminal, rev).unwrap();
    assert!(later > rev);
    assert!(changed.iter().all(|r| r.rev > rev) && !changed.is_empty());
    assert!(changed.len() < all.len());
}

#[test]
fn session_start_for_a_new_session_moves_the_projector_to_its_file() {
    let dir = Dir::new("clear");
    let before = dir.file("b7a1c0de-0000-4000-8000-000000000001.jsonl", EDITS);
    let after = dir.file("b7a1c0de-0000-4000-8000-000000000002.jsonl", CLEARED_AFTER);
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, before);
    projectors.hook(
        terminal,
        "SessionStart",
        &json!({"session_id":"b7a1c0de-0000-4000-8000-000000000002","source":"clear","transcript_path":after}),
    );
    let ids: Vec<String> = kinds(&projectors, terminal).into_iter().map(|(id, _)| id).collect();
    assert!(ids.contains(&"turn:p1".to_string()), "the old session's rows stay");
    assert!(ids.contains(&"turn:p9".to_string()), "and the new session's are read");
}

#[test]
fn nothing_is_kept_for_a_terminal_with_no_projector_open() {
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.hook(terminal, "UserPromptSubmit", &json!({"prompt_id":"p","prompt":"x"}));
    projectors.tick(terminal, Some(Activity::Idle));
    assert!(!projectors.is_open(terminal));
    assert_eq!(projectors.page(terminal, None, 10), None);
}

/// The hook path's old reader, `MessageAssembler`, against the projector, over
/// the same claude hook payloads: the person's prompt, the agent's answer and
/// the turn's end agree.
#[test]
fn the_projector_agrees_with_the_hook_assembler() {
    let payloads = [
        ("UserPromptSubmit", json!({"prompt_id":"p1","prompt":"Write a haiku\nabout tmux"})),
        ("MessageDisplay", json!({"prompt_id":"p1","turn_id":"t1","message_id":"m1","index":0,"delta":"Panes split ","final":false})),
        ("MessageDisplay", json!({"prompt_id":"p1","turn_id":"t1","message_id":"m1","index":0,"delta":"Panes split ","final":false})),
        ("MessageDisplay", json!({"prompt_id":"p1","turn_id":"t1","message_id":"m1","index":1,"delta":"like thought.","final":true})),
        ("Stop", json!({"prompt_id":"p1"})),
    ];
    let mut assembler = MessageAssembler::new();
    let mut old = Vec::new();
    for (event, payload) in &payloads {
        old.extend(assembler.accept(Agent::Claude, event, payload));
    }
    let mut projection = farcooler_core::session_log::projector::Projection::new();
    for (n, (event, payload)) in payloads.iter().enumerate() {
        projection.hook(event, payload, n as i64);
    }
    let user: Vec<&str> = old.iter().filter_map(|e| match e { AgentEvent::Message { role: Role::User, text, .. } => Some(text.as_str()), _ => None }).collect();
    let agent: Vec<&str> = old.iter().filter_map(|e| match e { AgentEvent::Message { role: Role::Agent, text, .. } => Some(text.as_str()), _ => None }).collect();
    let ended = old.iter().filter(|e| matches!(e, AgentEvent::TurnEnded { .. })).count();
    let rows = projection.rows();
    let prompts: Vec<&str> = rows.iter().filter_map(|r| match &r.kind { RowKind::Turn(t) => Some(t.prompt.as_str()), _ => None }).collect();
    let prose: Vec<&str> = rows.iter().filter_map(|r| match &r.kind { RowKind::Prose(p) => Some(p.text.as_str()), _ => None }).collect();
    let finished = rows.iter().filter(|r| matches!(&r.kind, RowKind::Turn(t) if t.outcome == Some(TurnOutcome::Finished))).count();
    assert_eq!(prompts, user);
    assert_eq!(prose, agent);
    assert_eq!(finished, ended);
}

/// The call site: a claude hook accepted by the ingress reaches the terminal's
/// open projector; another agent's does not.
#[test]
fn the_ingress_feeds_a_claude_hook_to_the_open_projector() {
    let dir = Dir::new("ingress");
    let path = dir.file("s.jsonl", EDITS);
    let terminal = Uuid::now_v7();
    global().open(terminal, path);
    let store = Arc::new(farcooler_store::Store::open_in_memory().unwrap());
    let inventory: Arc<dyn farcooler_core::inventory::RuntimeInventory> = Arc::new(farcooler_core::inventory::FakeInventory::default());
    let ingress = crate::hook_ingress::HookIngress::new(store, inventory, Default::default());
    ingress.accept(terminal, Agent::Codex, "UserPromptSubmit", &json!({"prompt_id":"pc","prompt":"codex"}), None);
    ingress.accept(terminal, Agent::Claude, "UserPromptSubmit", &json!({"prompt_id":"pk","prompt":"claude"}), None);
    let ids: Vec<String> = global().page(terminal, None, 1000).unwrap().into_iter().map(|r| r.id).collect();
    assert!(ids.contains(&"turn:pk".to_string()));
    assert!(!ids.contains(&"turn:pc".to_string()));
    ingress.forget(terminal);
    assert!(!global().is_open(terminal), "forgetting the terminal closes its projector");
}

/// One terminal's projector busy (a rebuild, say) holds up no other
/// terminal's hooks.
#[test]
fn a_busy_projector_does_not_hold_up_another_terminals_hooks() {
    let dir = Dir::new("locks");
    let projectors = Arc::new(SessionProjectors::default());
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    projectors.open(a, dir.file("a.jsonl", EDITS));
    projectors.open(b, dir.file("b.jsonl", EDITS));
    let busy = projectors.get(a).unwrap();
    let _held = busy.lock();
    // A hook for `a` now waits on `a`'s projector; it must wait there and not
    // somewhere every terminal shares.
    let waiting = projectors.clone();
    std::thread::spawn(move || waiting.hook(a, "UserPromptSubmit", &json!({"prompt_id":"pa","prompt":"a"})));
    std::thread::sleep(std::time::Duration::from_millis(50));
    let (done, finished) = std::sync::mpsc::channel();
    let other = projectors.clone();
    std::thread::spawn(move || {
        other.hook(b, "UserPromptSubmit", &json!({"prompt_id":"pb","prompt":"b"}));
        let _ = done.send(());
    });
    assert!(finished.recv_timeout(std::time::Duration::from_secs(5)).is_ok(), "b's hook waited on a's projector");
}

/// A terminal forgotten while its first projector is being built does not
/// get that projector afterwards.
#[test]
fn a_projector_built_for_a_terminal_forgotten_meanwhile_is_dropped() {
    let dir = Dir::new("forgotten");
    let path = dir.file("s.jsonl", EDITS);
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    // `open`'s two halves, with `forget` between them.
    projectors.building.lock().unwrap().insert(terminal, Vec::new());
    let mut session = farcooler_core::session_log::projector::SessionProjector::open(path.clone());
    session.poll();
    projectors.forget(terminal);
    projectors.finish(terminal, session);
    assert!(!projectors.is_open(terminal), "leaked");
    projectors.open(terminal, path);
    assert!(projectors.is_open(terminal), "an ordinary open still keeps it");
}
