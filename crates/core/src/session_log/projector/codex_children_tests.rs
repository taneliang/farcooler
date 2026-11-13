//! Producer 0.167.2 bytes: a fork's inherited history is deliberately in the
//! fixture. A child must show only its own turn, with its native thread id.
use super::{SessionProjector, subagent_transcript};
use super::rows::*;

const CHILD: &str = include_str!("../../../fixtures/session-logs/codex-child-thread.jsonl");
const PARENT: &str = "01a1d140-cd9e-76c1-bab6-f67b239f418f";
const ID: &str = "01a1d142-5ac6-76b3-a8a0-fc7491c5720c";

fn setup(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let root = crate::scratch::dir(name);
    let dir = root.join("sessions/2026/11/13");
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join(format!("rollout-parent-{PARENT}.jsonl"));
    std::fs::write(&main, format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{PARENT}\",\"source\":\"cli\"}}}}\n")).unwrap();
    let child = dir.join(format!("rollout-child-{ID}.jsonl"));
    std::fs::write(&child, CHILD).unwrap();
    (root, main, child)
}

#[test]
fn native_id_and_registered_path_resolve_only_a_verified_direct_child() {
    let (root, main, child) = setup("codex-child-join");
    assert_eq!(subagent_transcript(&main, ID), Some(child.clone()));
    assert_eq!(subagent_transcript(&main, "/root/independent_review"), Some(child));
    assert_eq!(subagent_transcript(&main, PARENT), None, "inherited session_id is not the child id");
    assert_eq!(subagent_transcript(&main, "../../escape"), None);
    let unrelated = main.with_file_name("rollout-other.jsonl");
    std::fs::write(&unrelated, "{\"type\":\"session_meta\",\"payload\":{\"id\":\"another-session\"}}\n").unwrap();
    assert_eq!(subagent_transcript(&unrelated, ID), None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_fork_opens_only_its_own_history_and_parent_gets_only_a_summary() {
    let (root, main, child) = setup("codex-child-isolation");
    let mut own = SessionProjector::open(child);
    own.poll();
    let turns: Vec<_> = own.projection().rows().iter().filter(|r| matches!(r.kind, RowKind::Turn(_))).collect();
    assert_eq!(turns.len(), 1, "copied parent task_started must not open a turn");
    assert!(turns[0].id.contains("01a1d142-5b6e-7401-ac2f-648eb4a39287"));
    let wire = serde_json::to_string(own.projection().rows()).unwrap();
    assert!(!wire.contains("Inherited parent prompt"));
    assert!(!wire.contains("parent-model-must-not-leak"));
    assert!(wire.contains("Child review complete"), "the child's own prose opens");
    assert!(own.projection().rows().iter().any(|r| matches!(r.kind, RowKind::Tool(_))), "the child's own calls open");
    let mut parent = SessionProjector::open(main);
    parent.poll();
    let rows = parent.projection().rows();
    assert_eq!(rows.len(), 1, "no child prose or child turns in the parent");
    let RowKind::Subagent(sub) = &rows[0].kind else { panic!("{rows:?}") };
    assert_eq!(sub.agent_id.as_deref(), Some(ID));
    assert_eq!(sub.status, SubagentState::Completed);
    assert!(sub.ended_ms.is_some());
    assert_eq!(sub.model.as_deref(), Some("gpt-6.1-sol"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn guardian_wrong_parent_ambiguous_path_and_unsafe_files_are_refused() {
    let (root, main, child) = setup("codex-child-refuse");
    let duplicate = child.with_file_name("rollout-duplicate.jsonl");
    std::fs::write(&duplicate, CHILD.replace(ID, "01a1d142-5ac6-76b3-a8a0-fc7491c5720d")).unwrap();
    assert_eq!(subagent_transcript(&main, "/root/independent_review"), None);
    std::fs::remove_file(&duplicate).unwrap();
    let header: serde_json::Value = serde_json::from_str(CHILD.lines().next().unwrap()).unwrap();
    let mut guardian = header.clone();
    guardian["payload"]["source"] = serde_json::json!({"subagent":{"other":"guardian"}});
    guardian["payload"]["thread_source"] = "guardian_review".into();
    std::fs::write(&child, format!("{guardian}\n")).unwrap();
    assert_eq!(subagent_transcript(&main, ID), None);
    std::fs::remove_file(&child).unwrap();
    std::fs::write(&duplicate, CHILD).unwrap();
    std::os::unix::fs::symlink(&duplicate, &child).unwrap();
    // The regular file still resolves; the symlink cannot create ambiguity.
    assert_eq!(subagent_transcript(&main, ID), Some(duplicate.clone()));
    std::fs::remove_file(&duplicate).unwrap();
    assert_eq!(subagent_transcript(&main, ID), None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_child_grows_live_then_ends_and_resumes_without_copying_parent_rows() {
    let (root, main, child) = setup("codex-child-live");
    let initial: String = CHILD.lines().filter(|l| !l.contains("task_complete")).map(|l| format!("{l}\n")).collect();
    std::fs::write(&child, initial).unwrap();
    let mut parent = SessionProjector::open(main);
    parent.poll();
    let state = |p: &SessionProjector| match &p.projection().rows()[0].kind { RowKind::Subagent(s) => s.status, _ => panic!() };
    assert_eq!(state(&parent), SubagentState::Running);
    let mut file = std::fs::OpenOptions::new().append(true).open(&child).unwrap();
    use std::io::Write;
    writeln!(file, "{}", CHILD.lines().last().unwrap()).unwrap();
    parent.poll_paths(std::slice::from_ref(&child));
    assert_eq!(state(&parent), SubagentState::Completed);
    writeln!(file, "{{\"ordinal\":232,\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"resumed\",\"started_at\":1794512400}}}}").unwrap();
    parent.poll_paths(std::slice::from_ref(&child));
    assert_eq!(state(&parent), SubagentState::Running);
    assert_eq!(parent.projection().rows().len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_fork_without_a_native_history_boundary_is_unavailable() {
    let (root, main, child) = setup("codex-child-no-boundary");
    std::fs::write(&child, CHILD.replace("\"subagent_history_start_ordinal\":47", "\"unknown\":47")).unwrap();
    assert_eq!(subagent_transcript(&main, ID), None);
    let mut own = SessionProjector::open(child);
    own.poll();
    assert!(own.projection().rows().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_open_drill_refuses_regular_replacement_with_a_guardian_or_other_child() {
    let (root, main, child) = setup("codex-child-replaced");
    let mut drill = SessionProjector::open_subagent(child.clone(), main);
    drill.poll();
    assert!(!drill.projection().rows().is_empty());
    let other = CHILD.replace(ID, "01a1d142-5ac6-76b3-a8a0-fc7491c5720d");
    std::fs::write(&child, other).unwrap();
    drill.poll();
    assert!(drill.binding_refused());
    assert!(drill.projection().rows().is_empty());
    let guardian = CHILD.replace("\"thread_source\":\"subagent\"", "\"thread_source\":\"guardian_review\"");
    std::fs::write(&child, guardian).unwrap();
    drill.poll();
    assert!(drill.binding_refused());
    assert!(drill.projection().rows().is_empty());
    std::fs::write(&child, CHILD).unwrap();
    drill.poll();
    assert!(!drill.binding_refused());
    assert!(!drill.projection().rows().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

/// Explicit read-only producer check. Paths come from the caller so this
/// never searches another person's conversations or prints their content.
#[test]
#[ignore]
fn real_codex_child_opens_under_its_parent_with_its_own_rows() {
    let main = std::path::PathBuf::from(std::env::var_os("FARCOOLER_CODEX_PARENT_ROLLOUT").expect("parent rollout"));
    let id = std::env::var("FARCOOLER_CODEX_CHILD_THREAD").expect("native child id");
    let child = subagent_transcript(&main, &id).expect("verified direct child");
    let mut parent = SessionProjector::open(main.clone());
    parent.poll();
    assert!(parent.projection().rows().iter().any(|r| matches!(&r.kind, RowKind::Subagent(s) if s.agent_id.as_deref() == Some(&id) && s.model.is_some())));
    let mut drill = SessionProjector::open_subagent(child, main);
    drill.poll();
    assert!(!drill.binding_refused());
    assert!(drill.projection().rows().iter().any(|r| matches!(r.kind, RowKind::Prose(_) | RowKind::Tool(_))));
}

#[test]
fn a_rebuild_keeps_the_child_under_the_parent_turn_that_launched_it() {
    let (root, main, _) = setup("codex-child-parent-turn");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(&main).unwrap();
    for id in ["01a1d140-ce2b-7e33-9337-544964ec7288", "later-ticket-turn"] {
        writeln!(file, "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"{id}\",\"started_at\":1794512047}}}}").unwrap();
    }
    let mut parent = SessionProjector::open(main);
    parent.poll();
    let child = parent.projection().rows().iter().find(|r| matches!(r.kind, RowKind::Subagent(_))).unwrap();
    assert_eq!(child.turn.as_deref(), Some("turn:01a1d140-ce2b-7e33-9337-544964ec7288"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_parent_counts_only_running_children_in_their_launch_turn() {
    let (root, main, child) = setup("codex-child-background-count");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(&main).unwrap();
    writeln!(file, "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"01a1d140-ce2b-7e33-9337-544964ec7288\",\"started_at\":1794512047}}}}").unwrap();
    std::fs::write(&child, CHILD.lines().filter(|l| !l.contains("task_complete")).map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    let mut parent = SessionProjector::open(main);
    parent.poll();
    let count = |p: &SessionProjector| match &p.projection().rows()[0].kind { RowKind::Turn(t) => t.background_running, _ => panic!() };
    assert_eq!(count(&parent), 1);
    let rev = parent.projection().revision();
    parent.poll();
    assert_eq!(rev, parent.projection().revision(), "an unchanged poll must not wake followers");
    let mut file = std::fs::OpenOptions::new().append(true).open(&child).unwrap();
    writeln!(file, "{}", CHILD.lines().last().unwrap()).unwrap();
    parent.poll_paths(&[child]);
    assert_eq!(count(&parent), 0);
    std::fs::remove_dir_all(root).unwrap();
}
