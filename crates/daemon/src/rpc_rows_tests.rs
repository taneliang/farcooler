//! `agent.rows`' own pieces: where a terminal's transcript is, and a follow
//! put on the wire. The routes themselves are tested over every transport
//! (`tests/agent_rows_over_every_transport.rs`).

use farcooler_core::session_log::projector::{Gap, GapReason, Row, RowKind};
use farcooler_protocol::v1::AgentRowChangeKind;

use super::*;

fn a_row(id: &str, ord: u64, rev: u64) -> Row {
    Row {
        ord,
        rev,
        id: id.into(),
        turn: None,
        provisional: false,
        retracted: false,
        born: 0,
        kind: RowKind::Gap(Gap { reason: GapReason::Unparsed, count: 1 }),
    }
}

#[test]
fn a_transcript_is_found_under_the_worktree_then_anywhere_then_expected_there() {
    let config = tempfile::tempdir().unwrap();
    let worktree = tempfile::tempdir().unwrap();
    let path = worktree.path().to_string_lossy().into_owned();
    let resolved = std::fs::canonicalize(worktree.path()).unwrap().to_string_lossy().into_owned();
    let own = config.path().join("projects").join(farcooler_core::session_log::claude_slug(&resolved)).join("s1.jsonl");
    assert_eq!(transcript_of(config.path(), &path, "s1"), own, "not written yet: where it will be");

    let elsewhere = config.path().join("projects/-orchestrator-home/s1.jsonl");
    std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    std::fs::write(&elsewhere, "").unwrap();
    assert_eq!(transcript_of(config.path(), &path, "s1"), elsewhere, "an orchestrator's, from its own directory");

    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, "").unwrap();
    assert_eq!(transcript_of(config.path(), &path, "s1"), own, "the worktree's own comes first");
}

#[test]
fn a_follow_goes_on_the_wire_as_kinds_by_id() {
    let terminal = Uuid::now_v7();
    let follow = Follow::Changes {
        epoch: 7,
        rev: 9,
        changes: vec![
            RowChange::Update(a_row("turn:p1", 0, 8)),
            RowChange::Remove { id: "hprose:m1".into(), rev: 7 },
            RowChange::Insert(a_row("prose:a1:0", 2, 9)),
        ],
    };
    let wire = pb_changes(terminal, follow);
    assert_eq!((wire.epoch, wire.rev, wire.reset), (7, 9, false));
    let kinds: Vec<(i32, &str, bool)> = wire.changes.iter().map(|c| (c.kind, c.id.as_str(), c.row.is_some())).collect();
    assert_eq!(
        kinds,
        [
            (AgentRowChangeKind::Update as i32, "turn:p1", true),
            (AgentRowChangeKind::Remove as i32, "hprose:m1", false),
            (AgentRowChangeKind::Insert as i32, "prose:a1:0", true),
        ]
    );
    let row: serde_json::Value = serde_json::from_str(&wire.changes[2].row.as_ref().unwrap().row_json).unwrap();
    assert_eq!(row["id"], "prose:a1:0");
    assert!(row.get("born").is_none() && row.get("retracted").is_none(), "bookkeeping stays off the wire: {row}");
    let reset = pb_changes(terminal, Follow::Reset { epoch: 8, rev: 3 });
    assert!(reset.reset && reset.changes.is_empty());
}

/// A follow's wait counts from the call's arrival: an open that took 300 ms
/// (a rebuild) leaves 200 ms of a 500 ms wait, not another 500.
#[tokio::test]
async fn a_follows_wait_counts_from_arrival_not_from_the_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    std::fs::write(&path, "{\"type\":\"user\",\"promptId\":\"p1\",\"promptSource\":\"typed\",\"uuid\":\"u1\",\"message\":{\"content\":\"hi\"}}\n").unwrap();
    let projectors = session_projectors::SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, path);
    let page = projectors.read_page(terminal, None, 10).unwrap();
    let p = pb::AgentRowsFollow { terminal_id: wire::id_bytes(terminal), epoch: page.epoch, after_rev: page.rev, wait_ms: 500 };
    let arrived = tokio::time::Instant::now();
    let slow_open = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ok(true)
    };
    let follow = follow_answer(&projectors, terminal, &p, arrived, slow_open).await.unwrap();
    let took = arrived.elapsed();
    assert!(matches!(follow, Follow::Changes { ref changes, .. } if changes.is_empty()), "{follow:?}");
    assert!(took >= Duration::from_millis(480) && took < Duration::from_millis(700), "took {took:?}");
}
