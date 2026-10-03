//! A chat pane's failed turn reaching its row (ov-140). Its own file to keep
//! `watch.rs` inside its size budget.

use super::*;

/// A chat pane's failed turn reaches the row through the supervisor,
/// because a chat pane has no session log for `turn_failed` to come from
/// (ov-140) — including when the row is already on the Done of a turn
/// that worked, where the activity does not move at all.
#[tokio::test]
async fn a_chat_panes_failed_turn_reaches_its_row() {
    use farcooler_agent::event::{AgentEvent, EndReason, FailureKind, Role};
    use farcooler_protocol::v1::event::Payload;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let main = rows.iter().find(|w| w.is_main_checkout).unwrap();
    let term = svc.create_terminal(main.id, "shell", "shell").await.expect("a shell pane");
    let row = svc.store.get_terminal(term.id).unwrap();
    svc.store
        .set_pane_mode(term.id, row.resource_version, farcooler_store::models::PaneMode::Agent, None, false)
        .unwrap();
    let watcher = Watcher::new(svc.clone());
    let mut rx = watcher.subscribe();
    let failed_on_the_wire = |rx: &mut broadcast::Receiver<Event>| {
        let mut failed = None;
        while let Ok(event) = rx.try_recv() {
            if let Some(Payload::TerminalChanged(t)) = event.payload
                && t.id.as_ref() == term.id.as_bytes()
            {
                failed = Some(t.turn_failed);
            }
        }
        failed
    };

    // A refused key on the first prompt: nothing before the end, so the
    // row never saw Working. Before, it stayed Idle and nobody was told.
    let refused = || AgentEvent::TurnEnded {
        reason: EndReason::Failed { kind: FailureKind::Auth, detail: String::new() },
    };
    svc.agents().record(term.id, vec![refused()], &|_, _| {});
    for _ in 0..CONFIRMATIONS {
        watcher.sample().await;
    }
    let seen = watcher.observed_snapshot().await;
    assert_eq!(seen[&term.id].activity, AgentActivity::Done, "a failure is news");
    assert!(seen[&term.id].turn_failed);
    assert_eq!(failed_on_the_wire(&mut rx), Some(true));
    // Looked at, it stays looked at: the same failure does not light the
    // row up again.
    watcher.mark_seen(term.id).await;
    for _ in 0..CONFIRMATIONS {
        watcher.sample().await;
    }
    assert_eq!(watcher.observed_snapshot().await[&term.id].activity, AgentActivity::Idle);

    // A turn that worked, left unseen.
    svc.agents().record(
        term.id,
        vec![AgentEvent::Message { role: Role::Agent, text: "hi".into(), parent: None }],
        &|_, _| {},
    );
    for _ in 0..CONFIRMATIONS {
        watcher.sample().await;
    }
    svc.agents().record(term.id, vec![AgentEvent::TurnEnded { reason: EndReason::EndTurn }], &|_, _| {});
    for _ in 0..CONFIRMATIONS {
        watcher.sample().await;
    }
    let seen = watcher.observed_snapshot().await;
    assert_eq!(seen[&term.id].activity, AgentActivity::Done);
    assert!(!seen[&term.id].turn_failed);
    assert_eq!(failed_on_the_wire(&mut rx), Some(false));

    // The next one is refused, with the row still on that Done.
    svc.agents().record(term.id, vec![refused()], &|_, _| {});
    watcher.sample().await;
    assert!(watcher.observed_snapshot().await[&term.id].turn_failed, "the row says it failed");
    assert_eq!(failed_on_the_wire(&mut rx), Some(true), "and a client is told");
}
