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
    let wire = pb_changes(terminal, follow, true);
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
    let reset = pb_changes(terminal, Follow::Reset { epoch: 8, rev: 3 }, true);
    assert!(reset.reset && reset.changes.is_empty());
}

/// An older client counts the `Hint` row as a row it cannot draw, and a fresh
/// session would lose its empty state, so only a client that said it draws
/// one is sent it, in a page and in a follow.
#[test]
fn the_hint_row_goes_only_to_a_client_that_draws_it() {
    let hint = || a_row(HINT_ID, 1, 5);
    let mut page = vec![a_row("turn:p1", 0, 4), hint()];
    keep_hint_for(&mut page, false);
    assert_eq!(page.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["turn:p1"]);
    let mut page = vec![a_row("turn:p1", 0, 4), hint()];
    keep_hint_for(&mut page, true);
    assert_eq!(page.len(), 2);

    let follow = || Follow::Changes {
        epoch: 1,
        rev: 6,
        changes: vec![RowChange::Insert(hint()), RowChange::Update(a_row("turn:p1", 0, 6)), RowChange::Remove { id: HINT_ID.into(), rev: 6 }],
    };
    let ids = |wire: pb::AgentRowChanges| wire.changes.into_iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(pb_changes(Uuid::now_v7(), follow(), false)), ["turn:p1"]);
    assert_eq!(ids(pb_changes(Uuid::now_v7(), follow(), true)), [HINT_ID, "turn:p1", HINT_ID]);
    // The revision still advances for a client that is not sent the row.
    assert_eq!(pb_changes(Uuid::now_v7(), follow(), false).rev, 6);
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
    let p = pb::AgentRowsFollow { terminal_id: wire::id_bytes(terminal), epoch: page.epoch, after_rev: page.rev, wait_ms: 500, hint_rows: true, agent_id: String::new() };
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

/// B2 (ov-830 review): a pane whose projector the tick opens after the
/// first page, a claude typed into a shell, answers its waiting follow with
/// Reset once it opens, not at the end of a 20 s wait; and so does a return
/// after the linger let the projector go. On tokio's paused clock: the
/// bound is the follow's own, never the machine's speed. Goes red with the
/// wait's look for an opened projector gone (`OPENED_EVERY`).
#[tokio::test(start_paused = true)]
async fn a_follow_answers_reset_as_soon_as_the_tick_opens_the_pane() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    std::fs::write(&path, "{\"type\":\"user\",\"promptId\":\"p1\",\"promptSource\":\"typed\",\"uuid\":\"u1\",\"message\":{\"content\":\"hi\"}}\n").unwrap();
    let projectors = session_projectors::SessionProjectors::default();
    let terminal = Uuid::now_v7();
    let ask = |epoch: u64, rev: u64| pb::AgentRowsFollow { terminal_id: wire::id_bytes(terminal), epoch, after_rev: rev, wait_ms: 20_000, hint_rows: true, agent_id: String::new() };
    let tick = Duration::from_secs(1);

    for (epoch, rev, when) in [(0, 0, "the first open"), (7, 3, "a return after the linger")] {
        let arrived = tokio::time::Instant::now();
        let p = ask(epoch, rev);
        let opened_by_the_tick = async {
            tokio::time::sleep(tick).await;
            projectors.open(terminal, path.clone());
        };
        let (follow, ()) = tokio::join!(follow_answer(&projectors, terminal, &p, arrived, async { Ok(false) }), opened_by_the_tick);
        let took = arrived.elapsed();
        assert!(matches!(follow, Ok(Follow::Reset { epoch, .. }) if epoch != 0), "{when}: {follow:?}");
        assert!(took <= tick + OPENED_EVERY, "{when}: answered after {took:?}, not once it opened");
        // The linger runs out: the tick lets it go.
        projectors.forget(terminal);
    }
}

/// A follow marks its pane viewed, as a page does, so a pane somebody is
/// following keeps its projector past `LINGER` (ov-830 review). Goes red
/// with `viewed` taken off `agent.rows_follow`'s path.
#[tokio::test]
async fn a_follow_keeps_its_pane_viewed() {
    use farcooler_protocol::v1::TerminalIntent;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let worktree = svc.store.list_worktrees_in_order().unwrap().into_iter().find(|w| w.repository_id == repo).expect("the checkout");
    let terminal = svc.store.create_terminal(worktree.id, "pane", "shell", TerminalIntent::Running, 80, 24).unwrap().id;
    let follow = || Request {
        method: "agent.rows_follow".into(),
        payload: Some(request::Payload::AgentRowsFollow(pb::AgentRowsFollow { terminal_id: wire::id_bytes(terminal), epoch: 0, after_rev: 0, wait_ms: 0, hint_rows: true, agent_id: String::new() })),
        ..Default::default()
    };
    dispatch(&svc, follow()).await.expect("follow");
    assert!(session_projectors::wanted(terminal), "a follow didn't mark its pane viewed");
    session_projectors::age_view(terminal, session_projectors::LINGER + Duration::from_secs(1));
    assert!(!session_projectors::wanted(terminal), "the linger ran out");
    dispatch(&svc, follow()).await.expect("follow");
    assert!(session_projectors::wanted(terminal), "a follow after the linger didn't mark it again");

    // An id the runner has no terminal for marks nothing.
    let unknown = Uuid::now_v7();
    let mut req = follow();
    if let Some(request::Payload::AgentRowsFollow(p)) = req.payload.as_mut() {
        p.terminal_id = wire::id_bytes(unknown);
    }
    assert!(dispatch(&svc, req).await.is_err());
    assert!(!session_projectors::wanted(unknown));
}
