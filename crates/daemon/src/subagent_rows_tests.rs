//! A subagent's own rows (ov-453): paged from its transcript, followed as it
//! grows, and let go.

use std::io::Write;

use super::*;
use crate::session_projectors::RowChange;

const SIDECHAIN: &str = include_str!("../../core/fixtures/session-logs/claude-subagent-transcript.jsonl");

fn written(dir: &std::path::Path, lines: &[&str]) -> PathBuf {
    let path = dir.join("agent-a1.jsonl");
    std::fs::write(&path, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    path
}

fn soon(ms: u64) -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_millis(ms)
}

#[tokio::test]
async fn a_subagents_rows_are_paged_then_followed_as_its_file_grows() {
    let dir = tempfile::tempdir().unwrap();
    let lines: Vec<&str> = SIDECHAIN.lines().collect();
    let path = written(dir.path(), &lines[..3]);
    let projectors = SubagentProjectors::default();
    let terminal = Uuid::now_v7();

    let page = projectors.page(terminal, "a1", path.clone(), path.clone(), None, 100);
    assert!(page.rows.iter().any(|r| r.id.starts_with("turn:")), "the task it was given opens it");
    let quiet = projectors.follow(terminal, "a1", (path.clone(), path.clone()), (page.epoch, page.rev), soon(300), 500).await;
    assert!(matches!(&quiet, Follow::Changes { changes, rev, .. } if changes.is_empty() && *rev == page.rev), "nothing new: {quiet:?}");

    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    for line in &lines[3..] {
        writeln!(file, "{line}").unwrap();
    }
    let Follow::Changes { changes, .. } = projectors.follow(terminal, "a1", (path.clone(), path.clone()), (page.epoch, page.rev), soon(5_000), 500).await else {
        panic!("a reset for a follower that is current")
    };
    assert!(changes.iter().any(|c| matches!(c, RowChange::Insert(row) if row.id.starts_with("prose:"))), "its words arrive: {changes:?}");

    let stale = projectors.follow(terminal, "a1", (path.clone(), path), (page.epoch + 1, page.rev), soon(0), 500).await;
    assert!(matches!(stale, Follow::Reset { .. }), "another projection's revisions page again");
}

#[test]
fn at_most_kept_are_held_and_a_terminals_go_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path(), &[]);
    let projectors = SubagentProjectors::default();
    let terminal = Uuid::now_v7();
    for n in 0..KEPT + 3 {
        projectors.page(terminal, &format!("a{n}"), path.clone(), path.clone(), None, 10);
    }
    assert_eq!(projectors.held(), KEPT, "the least recently read let go");
    projectors.forget(terminal);
    assert_eq!(projectors.held(), 0);
}

#[tokio::test]
async fn codex_child_rows_stream_and_a_replaced_rollout_resets_without_leaking() {
    const CHILD: &str = include_str!("../../core/fixtures/session-logs/codex-child-thread.jsonl");
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions/2026/11/13");
    std::fs::create_dir_all(&sessions).unwrap();
    let parent = sessions.join("rollout-parent-01a1d140-cd9e-76c1-bab6-f67b239f418f.jsonl");
    std::fs::write(&parent, "{\"type\":\"session_meta\",\"payload\":{\"id\":\"01a1d140-cd9e-76c1-bab6-f67b239f418f\",\"source\":\"cli\"}}\n").unwrap();
    let id = "01a1d142-5ac6-76b3-a8a0-fc7491c5720c";
    let path = sessions.join(format!("rollout-child-{id}.jsonl"));
    let lines: Vec<_> = CHILD.lines().collect();
    std::fs::write(&path, lines[..lines.len() - 1].iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    let projectors = SubagentProjectors::default();
    let terminal = Uuid::now_v7();
    let page = projectors.page(terminal, id, path.clone(), parent.clone(), None, 100);
    let wire = serde_json::to_string(&page.rows).unwrap();
    assert!(wire.contains("Child review complete"));
    assert!(!wire.contains("Inherited parent prompt"));
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", lines.last().unwrap()).unwrap();
    let follow = projectors.follow(terminal, id, (path.clone(), parent.clone()), (page.epoch, page.rev), soon(1_000), 500).await;
    assert!(matches!(follow, Follow::Changes { changes, .. } if !changes.is_empty()));
    std::fs::write(&path, CHILD.replace("\"thread_source\":\"subagent\"", "\"thread_source\":\"guardian_review\"")).unwrap();
    let follow = projectors.follow(terminal, id, (path.clone(), parent.clone()), (page.epoch, page.rev), soon(1_000), 500).await;
    assert!(matches!(follow, Follow::Reset { rev: 0, .. }));
    assert!(projectors.page(terminal, id, path, parent, None, 100).rows.is_empty());
}
