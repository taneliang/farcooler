//! ov-951: the orchestrator moves a card by saying the column it wants; the move is the engine's.
//! `task set --status` from an agent on a tracker-only ticket becomes a `Move` command, so the card
//! changes only when the engine accepts it. A person's hand move is refused by the store.
use crate::service::Service;
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::models::TaskStatus;

/// The engine column a status names.
pub(crate) fn column(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Backlog => "backlog", TaskStatus::Todo => "todo", TaskStatus::InProgress => "doing",
        TaskStatus::InReview => "review", TaskStatus::NeedsDecision => "decision",
        TaskStatus::Done => "done", TaskStatus::Cancelled => "cancelled",
    }
}

/// Who may drive an engine move, decided before the engine is asked (ov-951). The orchestrator pane of the
/// card's workspace may move any tracked card to any column. A worker pane may move only the ticket it was
/// opened for, and only among the live columns: closing a ticket is permanent, so it stays the orchestrator's.
/// A person, a rep, an agent outside Far Cooler or an unreadable seat has no say here.
fn admit(svc: &Service, seat: crate::caller_seat::Seat, person: &crate::people::Person, card: &farcooler_store::models::Task, status: TaskStatus) -> Result<()> {
    use crate::caller_seat::Seat;
    use farcooler_store::models::TerminalRole;
    let Seat::Agent(Some(pane)) = seat else { return Err(DomainError::ScopeDenied { needed: "workspace orchestrator" }) };
    let term = svc.store.get_terminal(pane)?;
    if term.workspace_id == Some(card.workspace_id) && term.role == TerminalRole::Orchestrator {
        // `workflow.move` is the owner's; a member's pane is turned away here too.
        if *person != crate::people::Person::Owner { return Err(DomainError::ScopeDenied { needed: "owner" }); }
        // The same lead-login check `workflow.move` makes (`Admission::capture`), not a copy of it.
        return svc.refuse_another_leads_login(&term, card.workspace_id);
    }
    if term.task_id == Some(card.id) && !matches!(status, TaskStatus::Done | TaskStatus::Cancelled) { return Ok(()); }
    Err(DomainError::ScopeDenied { needed: "workspace orchestrator" })
}

/// Moves `task` through the engine. `None` when the engine does not hold it as a ticket at rest (a run owns
/// it, or the engine is off), and the caller carries on as before. A tracked card is refused to a caller
/// `admit` turns away, whatever the engine would say.
pub(crate) async fn move_through_engine(svc: &Service, seat: crate::caller_seat::Seat, person: &crate::people::Person, task: uuid::Uuid, req: &pb::TaskSetStatus) -> Result<Option<pb::Task>> {
    let Some(status) = crate::task_ops::status_from_wire(req.status) else { return Ok(None) };
    let Some(handle) = svc.workflow_handle() else { return Ok(None) };
    // Only a ticket at rest is moved here. A card the engine does not hold works as it always did, and a card
    // with a run (or a closed one) falls to the store, whose plain sentence says the engine owns it, whoever asks.
    if !svc.store.task_is_tracked_ticket(task)? { return Ok(None); }
    let card = svc.store.get_task(task)?;
    admit(svc, seat, person, &card, status)?;
    let moved = handle.move_ticket(&card.workspace_id.to_string(), &card.key, column(status)).await
        .map_err(|_| DomainError::OperationFailed)?;
    if !moved { return Ok(None); }
    Ok(Some(crate::task_starts::pb_one(svc, &svc.store.get_task(task)?)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caller_seat::Seat;
    use crate::people::Person;
    use farcooler_protocol::v1::TerminalIntent;
    use farcooler_store::models::{Actor, TerminalRole};

    fn set(task: uuid::Uuid, status: TaskStatus) -> pb::TaskSetStatus {
        pb::TaskSetStatus { task_id: task.as_bytes().to_vec().into(), status: crate::task_ops::pb_status(status), actor: "manager".into() }
    }

    /// The workspace's orchestrator pane, and a worker pane opened for `task`.
    fn panes(svc: &Service, repo: uuid::Uuid, workspace: uuid::Uuid, task: uuid::Uuid) -> (uuid::Uuid, uuid::Uuid) {
        let worktree = svc.store.list_worktrees_for_repository(repo).unwrap()[0].id;
        let orchestrator = svc.store.create_terminal_for_test(worktree, workspace);
        svc.store.set_terminal_role(orchestrator, TerminalRole::Orchestrator).unwrap();
        let worker = svc.store.create_terminal_for_task(worktree, "worker", "claude", TerminalIntent::Running, 80, 24, Some(task)).unwrap().id;
        svc.store.set_terminal_workspace(worker, workspace).unwrap();
        (orchestrator, worker)
    }

    #[tokio::test]
    async fn the_orchestrator_moves_a_tracked_card_through_the_engine_and_a_hand_cannot() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let board = svc.store.ensure_main_workspace(repo).unwrap();
        let card = svc.store.create_task(board.id, "moved", Actor::User).unwrap();
        let elsewhere = svc.store.create_task(board.id, "elsewhere", Actor::User).unwrap();
        let (orchestrator, _) = panes(&svc, repo, board.id, elsewhere.id);
        let seat = Seat::Agent(Some(orchestrator));
        // Before the engine holds it, nothing is moved here.
        assert!(move_through_engine(&svc, seat, &Person::Owner, card.id, &set(card.id, TaskStatus::Todo)).await.unwrap().is_none());
        crate::test_support::publish_for_test(&svc, board.id);
        svc.start_workflows().unwrap();
        svc.track_tickets(None).await;
        for status in [TaskStatus::Todo, TaskStatus::InProgress, TaskStatus::InReview, TaskStatus::NeedsDecision, TaskStatus::Backlog] {
            let moved = move_through_engine(&svc, seat, &Person::Owner, card.id, &set(card.id, status)).await.unwrap().expect("the engine holds it");
            assert_eq!(moved.status, crate::task_ops::pb_status(status));
            assert_eq!(svc.store.get_task(card.id).unwrap().status, status);
        }
        // Where it already is changes nothing and is not a failure.
        assert!(move_through_engine(&svc, seat, &Person::Owner, card.id, &set(card.id, TaskStatus::Backlog)).await.unwrap().is_some());
        // A hand on the store is refused in words.
        assert!(svc.store.set_task_status(card.id, TaskStatus::Todo, Actor::User).is_err());
        let closed = move_through_engine(&svc, seat, &Person::Owner, card.id, &set(card.id, TaskStatus::Done)).await.unwrap().unwrap();
        assert_eq!(closed.status, crate::task_ops::pb_status(TaskStatus::Done));
        svc.stop_workflows().await.unwrap();
    }

    /// B2: a tracked card is moved by the orchestrator, or by the worker opened for it among the live
    /// columns. Nobody else, and nobody closes a card but the orchestrator.
    #[tokio::test]
    async fn only_the_orchestrator_or_the_cards_own_worker_moves_a_tracked_card() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let board = svc.store.ensure_main_workspace(repo).unwrap();
        let card = svc.store.create_task(board.id, "mine", Actor::User).unwrap();
        let other = svc.store.create_task(board.id, "theirs", Actor::User).unwrap();
        // The worker is linked before tracking: today's store refuses a hand-linked pane on a tracked card,
        // so `admit` is asked about it directly.
        let (orchestrator, worker) = panes(&svc, repo, board.id, card.id);
        let allowed = |seat, task: &farcooler_store::models::Task, status| admit(&svc, seat, &Person::Owner, task, status).is_ok();
        let own = Seat::Agent(Some(worker));
        assert!(allowed(own, &card, TaskStatus::InReview));
        assert!(allowed(own, &card, TaskStatus::NeedsDecision));
        assert!(!allowed(own, &card, TaskStatus::Done), "a worker does not close its ticket");
        assert!(!allowed(own, &card, TaskStatus::Cancelled));
        assert!(!allowed(own, &other, TaskStatus::InReview), "another ticket is not its to move");
        let lead = Seat::Agent(Some(orchestrator));
        assert!(allowed(lead, &card, TaskStatus::Done) && allowed(lead, &other, TaskStatus::Todo));
        // A person, an agent outside Far Cooler, an unreadable seat: none.
        for seat in [Seat::Person, Seat::Agent(None), Seat::Unreadable] {
            assert!(!allowed(seat, &card, TaskStatus::Todo), "{seat:?}");
        }
        // Through the engine, the same refusal comes before any move.
        crate::test_support::publish_for_test(&svc, board.id);
        svc.start_workflows().unwrap();
        svc.track_tickets(Some(other.id)).await;
        for seat in [Seat::Person, Seat::Agent(None), Seat::Unreadable, own] {
            let r = move_through_engine(&svc, seat, &Person::Owner, other.id, &set(other.id, TaskStatus::Done)).await;
            assert!(matches!(r, Err(DomainError::ScopeDenied { .. })), "{seat:?}: {r:?}");
        }
        assert_eq!(svc.store.get_task(other.id).unwrap().status, TaskStatus::Backlog);
        svc.stop_workflows().await.unwrap();
    }

    /// B2 remainder: the orchestrator is held to `workflow.move`'s lead-login check. One that runs on the
    /// runner's login while a member leads the workspace may not move or close a card.
    #[tokio::test]
    async fn an_orchestrator_on_another_leads_login_moves_nothing() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let board = svc.store.ensure_main_workspace(repo).unwrap();
        let card = svc.store.create_task(board.id, "mine", Actor::User).unwrap();
        let (orchestrator, _) = panes(&svc, repo, board.id, card.id);
        let lead = Seat::Agent(Some(orchestrator));
        assert!(admit(&svc, lead, &Person::Owner, &card, TaskStatus::Done).is_ok());
        svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
        svc.store.set_workspace_lead(board.id, Some("bob")).unwrap();
        for status in [TaskStatus::Todo, TaskStatus::Done] {
            let r = admit(&svc, lead, &Person::Owner, &card, status);
            assert!(matches!(r, Err(DomainError::Conflict { what: "lead_changed" })), "{status:?}: {r:?}");
        }
    }

    /// B2 remainder: `workflow.move` is the owner's, so an orchestrator pane that is a member's is too.
    #[tokio::test]
    async fn a_members_orchestrator_pane_moves_nothing() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let board = svc.store.ensure_main_workspace(repo).unwrap();
        let card = svc.store.create_task(board.id, "mine", Actor::User).unwrap();
        let (orchestrator, _) = panes(&svc, repo, board.id, card.id);
        let r = admit(&svc, Seat::Agent(Some(orchestrator)), &Person::Member("bob".into()), &card, TaskStatus::Done);
        assert!(matches!(r, Err(DomainError::ScopeDenied { .. })), "{r:?}");
    }

    /// N1: a card with a run of its own is not this module's. Whoever asks, it falls to the store, which says
    /// in plain words that the engine owns it, rather than turning a person away with a scope error.
    #[tokio::test]
    async fn a_card_with_a_run_is_left_to_the_stores_plain_refusal() {
        use farcooler_workflow::{Command, Definition, Grant, Input};
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let board = svc.store.ensure_main_workspace(repo).unwrap();
        let card = svc.store.create_task(board.id, "running", Actor::User).unwrap();
        let definition = Definition::tracker("owner");
        let input = Input { schema: 1, id: "e".into(), workspace: board.id.to_string(), expected_revision: 0, epoch: 1,
            source: "controller".into(), source_sequence: 1, now_ms: 0, grant: Grant::Controller, busy_resources: Default::default(),
            command: Command::Enqueue { item: card.key.clone(), policy_pin: definition.pin(), resource: "own".into(),
                dependencies: Default::default(), initial_candidate: Some(definition.pin()), definition } };
        assert!(svc.store.workflow_apply(&input).unwrap().result.is_ok());
        svc.start_workflows().unwrap();
        for seat in [Seat::Person, Seat::Agent(None), Seat::Unreadable] {
            let r = move_through_engine(&svc, seat, &Person::Owner, card.id, &set(card.id, TaskStatus::Todo)).await;
            assert!(matches!(r, Ok(None)), "{seat:?}: {r:?}");
        }
        let e = svc.store.set_task_status(card.id, TaskStatus::Todo, Actor::User).unwrap_err();
        assert!(matches!(e, DomainError::Conflict { what: "engine_owned" }), "{e:?}");
        svc.stop_workflows().await.unwrap();
    }

    /// N3: the lead of one board does not move another board's tickets.
    #[tokio::test]
    async fn another_boards_orchestrator_moves_nothing_here() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let board = svc.store.ensure_main_workspace(repo).unwrap();
        let card = svc.store.create_task(board.id, "mine", Actor::User).unwrap();
        let (orchestrator, _) = panes(&svc, repo, board.id, card.id);
        let elsewhere = svc.store.create_workspace(repo, "elsewhere", "el").unwrap();
        let foreign = svc.store.create_task(elsewhere.id, "theirs", Actor::User).unwrap();
        let lead = Seat::Agent(Some(orchestrator));
        assert!(admit(&svc, lead, &Person::Owner, &card, TaskStatus::Todo).is_ok());
        let r = admit(&svc, lead, &Person::Owner, &foreign, TaskStatus::Todo);
        assert!(matches!(r, Err(DomainError::ScopeDenied { .. })), "{r:?}");
    }
}
