//! Messages between an orchestrator and its agents (ov-455), typed into
//! stand-ins on the real tmux server of `tests`, whose helpers these use.

use super::*;
use crate::watch::task_notice::AgentNews;

impl Board {
    /// Pump until `si` has logged `n` submissions, a pass at a time as the
    /// daemon's tick does. One pass and a sleep to cover the rest is a guess
    /// at how long the first send takes to settle (its confirmation, the
    /// spacing before the next), which a loaded runner beat (ov-640). The
    /// bound is a hang guard, not a wait.
    pub(super) async fn pump_until_submits(&self, si: &StandIn, n: usize) {
        let since = std::time::Instant::now();
        while si.submitted().len() < n {
            self.pump().await;
            // The stand-in reads the Enter a moment after a pass returns.
            for _ in 0..10 {
                if si.submitted().len() >= n {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(since.elapsed() < Duration::from_secs(60), "never submitted {n}: {}", si.log());
        }
    }

    /// Send `text` to `to` as `actor`, on the task's board, through
    /// `message.send` itself, from the owner's own connection.
    pub(super) async fn send(&self, actor: &str, to: &str, text: &str) -> Result<pb::MessageSent> {
        self.send_as(crate::people::Person::Owner, actor, to, text, "").await
    }

    /// `send`, from `person`'s connection (ov-494).
    pub(super) async fn send_as(
        &self,
        person: crate::people::Person,
        actor: &str,
        to: &str,
        text: &str,
        task: &str,
    ) -> Result<pb::MessageSent> {
        let req = pb::Request {
            method: "message.send".into(),
            payload: Some(pb::request::Payload::MessageSend(pb::MessageSend {
                to: to.into(),
                text: text.into(),
                actor: actor.into(),
                task: task.into(),
                workspace_id: self.task.workspace_id.as_bytes().to_vec().into(),
            })),
            ..Default::default()
        };
        // The local socket is a runner admin's; a device here holds Control.
        match self.watcher.message_send(req, person).await? {
            pb::result::Value::MessageSent(sent) => Ok(sent),
            other => panic!("not a sent message: {other:?}"),
        }
    }

    pub(super) fn messages(&self) -> Vec<PendingWake> {
        self.svc.store.pending_message_wakes().unwrap()
    }

    /// A claude pane opened as a builder on the task, as an agent dispatch
    /// opens one.
    pub(super) async fn builder_pane(&self) -> Terminal {
        let dispatch = pb::AgentDispatch { role: pb::AgentRole::Builder as i32, ..Default::default() };
        self.svc
            .create_terminal_as_agent(self.lane.id, "Builder", "claude", None, Some(self.task.id), Some(&dispatch))
            .await
            .unwrap()
    }
}

pub(super) fn agent_actor(t: &Terminal) -> String {
    format!("agent:{}", t.id)
}

pub(super) fn refused<T: std::fmt::Debug>(r: Result<T>) -> &'static str {
    match r {
        Err(DomainError::Conflict { what } | DomainError::InvalidArgument { what } | DomainError::NotYours { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// An agent's message reaches its orchestrator's box, tagged with the card
/// it works, and the card keeps it as a comment, with no extra note.
#[tokio::test]
async fn an_agents_message_reaches_its_orchestrator() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let agent = b.agent("Agent 2", "claude").await;
    let sent = b.send(&agent_actor(&agent), "orchestrator", "PR is up").await.unwrap();
    assert_eq!((sent.task_key.as_str(), sent.recipient.as_str()), (b.task.key.as_str(), "the orchestrator"));
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [format!("[from {}] PR is up", b.task.key)], "{}", si.log());
    let comments = b.svc.store.notes_for(b.task.id, Some(NoteKind::Comment)).unwrap();
    assert_eq!(comments.iter().map(|n| n.body.as_str()).collect::<Vec<_>>(), ["PR is up"]);
    assert!(b.progress().is_empty(), "delivered says nothing more: {:?}", b.progress());
    assert!(b.messages().is_empty());
}

/// The orchestrator messages a card, and the card's own agent gets it.
#[tokio::test]
async fn the_orchestrator_reaches_a_cards_agent() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let si = b.stand_in(&agent, "codex", "codex").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let sent = b.send("manager", &b.task.key, "Rebase on main first").await.unwrap();
    assert_eq!(sent.recipient, format!("{}'s agent", b.task.key));
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Rebase on main first"], "{}", si.log());
}

/// A ticket's builder is reached by the ticket's key, and what it sends is
/// tagged with its role and its ticket.
#[tokio::test]
async fn a_builder_is_addressed_by_its_ticket_and_tags_its_own() {
    let b = board().await;
    b.watcher.with_timing(|t| t.told_spacing = crate::watch::Timing::SHORT);
    let pane = b.builder_pane().await;
    let si = b.stand_in(&pane, "claude", "claude").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    let key = b.task.key.clone();
    let builder = format!("the builder on {key}");
    assert_eq!(b.send("manager", &key.to_uppercase(), "Ship it").await.unwrap().recipient, builder);
    assert_eq!(b.send("manager", &key, "Then stop").await.unwrap().recipient, builder);
    assert!(b.messages().iter().all(|w| w.to == Some(pane.id)));
    b.pump_until_submits(&si, 1).await;
    b.doing(pane.id, AgentActivity::Idle).await;
    b.pump_until_submits(&si, 2).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it", "[from the orchestrator] Then stop"], "in order: {}", si.log());

    let orchestrator = b.orchestrator().await;
    let osi = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.send(&agent_actor(&pane), "orchestrator", "Done").await.unwrap();
    b.pump().await;
    osi.submits(1).await;
    assert_eq!(osi.submitted(), [format!("[from the builder on {key}] Done")], "{}", osi.log());
}

/// Hub and spoke: an agent can't message another agent or card; nobody
/// messages themselves; a message is one bounded line; and a recipient can't
/// be flooded. Nothing is filed for a refusal.
#[tokio::test]
async fn refusals_file_nothing() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let other = b.svc.store.create_task(b.task.workspace_id, "Other", Actor::Manager).unwrap();
    assert_eq!(refused(b.send(&agent_actor(&agent), &other.key, "psst").await), "hub");
    assert_eq!(refused(b.send("manager", "orchestrator", "talking to myself").await), "self");
    assert_eq!(refused(b.send("manager", &other.key, "nobody's on it").await), "nobody");
    assert_eq!(refused(b.send("manager", "no-such-thing", "hello").await), "to");
    assert_eq!(refused(b.send(&agent_actor(&agent), "orchestrator", "  \n ").await), "text");
    assert_eq!(refused(b.send(&agent_actor(&agent), "orchestrator", &"x".repeat(LONGEST_TEXT_FOR_TESTS + 1)).await), "too_long");
    assert!(b.messages().is_empty());
    for n in 0..20 {
        b.send(&agent_actor(&agent), "orchestrator", &format!("ping {n}")).await.unwrap();
    }
    assert_eq!(refused(b.send(&agent_actor(&agent), "orchestrator", "one more").await), "flood");
    assert_eq!(b.messages().len(), 20);
}

const LONGEST_TEXT_FOR_TESTS: usize = messages::LONGEST_TEXT;

/// A builder that ends its turn without reporting has its orchestrator
/// told, once; one that reported isn't told about.
#[tokio::test]
async fn a_silent_stop_tells_the_orchestrator_once() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let pane = b.builder_pane().await;
    let finished = AgentNews::Finished { label: "Builder".into(), said: Some("All green.".into()) };
    let began = now_millis() - 1_000;
    b.watcher.agent_stopped(pane.id, &b.task, &finished, Some(began));
    b.watcher.agent_stopped(pane.id, &b.task, &finished, Some(began));
    assert_eq!(b.messages().len(), 1, "one notice waits at a time");
    b.pump().await;
    si.submits(1).await;
    let key = &b.task.key;
    let said = format!("[Far Cooler] The builder on {key} ended its turn without reporting. It last said: “All green.”");
    assert_eq!(si.submitted(), [said], "{}", si.log());

    b.svc.store.add_note(b.task.id, NoteKind::Decision, Actor::Agent { terminal: pane.id }, "Chose B", serde_json::json!({})).unwrap();
    b.watcher.agent_stopped(pane.id, &b.task, &finished, Some(began));
    assert!(b.messages().is_empty(), "it reported this turn");
    b.watcher.agent_stopped(pane.id, &b.task, &AgentNews::Failed { label: "Builder".into() }, Some(began));
    assert_eq!(b.messages().len(), 1, "a failed turn is told whatever it wrote");
}

/// The watcher's own transition is what tells: a builder read Done after
/// working, with an orchestrator running, queues the runner's notice.
#[tokio::test]
async fn a_done_transition_queues_the_notice() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let _si = b.stand_in(&orchestrator, "claude", "claude").await;
    let pane = b.builder_pane().await;
    let builder = b.stand_in(&pane, "claude", "claude").await;
    builder.show("working").await;
    b.doing(pane.id, AgentActivity::Working).await;
    builder.show("idle").await;
    for _ in 0..50 {
        b.watcher.sample().await;
        if !b.messages().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let waiting = b.messages();
    assert_eq!(waiting.len(), 1, "{waiting:?}");
    assert_eq!(waiting[0].actor, Actor::Runner);
    assert_eq!(waiting[0].to, None, "for the orchestrator");
}

/// No orchestrator running: the runner's notice isn't queued, since nobody
/// would read it, and the person's own notification already says it.
#[tokio::test]
async fn with_no_orchestrator_no_notice_is_queued() {
    let b = board().await;
    let pane = b.builder_pane().await;
    let finished = AgentNews::Finished { label: "Builder".into(), said: None };
    b.watcher.agent_stopped(pane.id, &b.task, &finished, None);
    assert!(b.messages().is_empty());
}

/// A message waits through a restart: a new watcher on the same store tells
/// it, once.
#[tokio::test]
async fn a_message_survives_a_restart() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    b.send(&agent_actor(&agent), "orchestrator", "Still here").await.unwrap();
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    let after = Watcher::new(b.svc.clone());
    observe(&after, orchestrator.id, AgentActivity::Idle).await;
    after.wakes_hint.store(true, Ordering::SeqCst);
    after.pump_wakes().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [format!("[from {}] Still here", b.task.key)], "{}", si.log());
    assert!(b.messages().is_empty());
}

/// A cursor pane (R-47) is told between turns: its box is read, the
/// message pasted, read back and sent. While it works, the message waits.
#[tokio::test]
async fn a_cursor_pane_is_told_between_turns_and_not_during_one() {
    let b = board().await;
    let agent = b.agent("Agent 2", "cursor").await;
    let si = b.stand_in(&agent, "cursor", "cursor-agent").await;
    si.show("working").await;
    b.doing(agent.id, AgentActivity::Working).await;
    b.send("manager", &b.task.key, "Run the gates").await.unwrap();
    b.pump().await;
    assert!(si.submitted().is_empty(), "typed into a cursor mid-turn: {}", si.log());
    assert_eq!(b.messages().len(), 1, "it waits");
    si.show("idle").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Run the gates"], "{}", si.log());
    assert!(b.messages().is_empty());
}

/// A message too long for one row of cursor's box wraps there, is read back
/// whole, and is sent (R-47): it isn't left pasted in the box.
#[tokio::test]
async fn a_long_message_wraps_in_cursors_box_and_is_sent() {
    let b = board().await;
    let agent = b.agent("Agent 2", "cursor").await;
    let si = b.stand_in(&agent, "cursor", "cursor-agent").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let long = "Run every CI gate in the foreground, fix whatever fails, commit as you go without pushing, and then report each gate's result back to me here.";
    b.send("manager", &b.task.key, long).await.unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [format!("[from the orchestrator] {long}")], "{}", si.log());
    assert!(b.progress().is_empty(), "nothing was left in the box: {:?}", b.progress());
}

/// A recipient's messages go in the order sent: while the first waits, a
/// later one for the same pane waits behind it, and then both are told in
/// order. Another pane's message isn't held up.
#[tokio::test]
async fn a_waiting_message_holds_back_later_ones_for_its_pane() {
    let b = board().await;
    b.watcher.with_timing(|t| t.told_spacing = crate::watch::Timing::SHORT);
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let agent = b.agent("Agent 2", "claude").await;
    let first = b.send(&agent_actor(&agent), "orchestrator", "First").await.unwrap();
    let first = Uuid::from_slice(&first.note_id).unwrap();
    b.send(&agent_actor(&agent), "orchestrator", "Second").await.unwrap();
    messages::hold_for_tests(first);
    b.pump().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(si.submitted().is_empty(), "the second passed the first: {}", si.log());
    assert_eq!(b.messages().len(), 2);
    messages::release_for_tests(first);
    b.pump_until_submits(&si, 1).await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.pump_until_submits(&si, 2).await;
    let tag = format!("[from {}]", b.task.key);
    assert_eq!(si.submitted(), [format!("{tag} First"), format!("{tag} Second")], "{}", si.log());
}

/// A builder's pane that has since been made the orchestrator is the sender
/// itself: the orchestrator messaging that ticket is refused as `self`.
#[tokio::test]
async fn the_orchestrator_cant_message_itself_through_a_ticket() {
    let b = board().await;
    let pane = b.builder_pane().await;
    b.svc.set_terminal_role(pane.id, farcooler_store::models::TerminalRole::Orchestrator).await.unwrap();
    assert_eq!(refused(b.send("manager", &b.task.key, "hello me").await), "self");
    assert!(b.messages().is_empty());
}

/// Nobody is typed as the owner: a message with no agent behind it is from
/// the terminal it was run in.
#[tokio::test]
async fn no_message_is_tagged_as_the_owners() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.send("user", &b.task.key, "Hello").await.unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the terminal] Hello"], "{}", si.log());
}

/// End to end across a cutover: a message queued before it stays inert history,
/// and a fresh manual message sent after restart is told exactly once.
///
/// The released phase is set by SQL on the frozen file, standing in for the
/// store's commit and release (covered by the store's own cutover tests); the
/// join, the atomic transfer, the gate and the delivery pump are the real ones.
#[tokio::test]
async fn a_manual_message_after_cutover_is_told_once_and_the_old_queue_stays_inert() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    let agent = b.agent("Agent 2", "claude").await;
    // The send's own pump finds the lock taken and gives up, so the message is
    // still queued at the cutover.
    let old_pump = b.watcher.wake_pump.try_lock().unwrap();
    b.send(&agent_actor(&agent), "orchestrator", "queued before cutover").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(old_pump);
    assert_eq!(b.messages().len(), 1);
    let Board { dir, svc, watcher, lane, task } = b;
    drop(watcher);
    let root = svc.root_dir().to_path_buf();
    let db = root.join("farcooler.db");
    svc.drain_writers().await.unwrap();
    drop(svc);
    farcooler_store::testing::released_transfer_for_tests(&db, "e2e");
    let svc = Arc::new(Service::open_in(root).await.expect("released engine is admitted by the gate"));
    let b = Board { dir, watcher: Watcher::new(svc.clone()), svc, lane, task };
    b.relearn_bracketing(&orchestrator, &si).await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    assert!(b.messages().is_empty(), "the pre-cutover message is not pending");
    b.pump().await;
    assert!(si.submitted().is_empty(), "and is never told: {}", si.log());
    b.send(&agent_actor(&agent), "orchestrator", "fresh manual").await.unwrap();
    b.pump_until_submits(&si, 1).await;
    b.pump().await;
    assert_eq!(si.submitted(), [format!("[from {}] fresh manual", b.task.key)], "{}", si.log());
    let rows: i64 = farcooler_store::testing::count_rows_for_tests(&b.svc.store, "message_wakes");
    assert_eq!(rows, 2, "the old row is retained beside the fresh one");
}
