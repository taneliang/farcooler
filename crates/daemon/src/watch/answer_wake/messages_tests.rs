//! Messages between an orchestrator and its lanes (ov-455), typed into
//! stand-ins on the real tmux server of `tests`, whose helpers these use.

use super::*;
use crate::watch::task_notice::AgentNews;
use farcooler_store::plan::{LaneCard, NewLane};

impl Board {
    /// Send `text` to `to` as `actor`, on the task's board, through
    /// `message.send` itself.
    async fn send(&self, actor: &str, to: &str, text: &str) -> Result<pb::MessageSent> {
        let req = pb::Request {
            method: "message.send".into(),
            payload: Some(pb::request::Payload::MessageSend(pb::MessageSend {
                to: to.into(),
                text: text.into(),
                actor: actor.into(),
                task: String::new(),
                workspace_id: self.task.workspace_id.as_bytes().to_vec().into(),
            })),
            ..Default::default()
        };
        match self.watcher.message_send(req).await? {
            pb::result::Value::MessageSent(sent) => Ok(sent),
            other => panic!("not a sent message: {other:?}"),
        }
    }

    fn messages(&self) -> Vec<PendingWake> {
        self.svc.store.pending_message_wakes().unwrap()
    }

    /// A lane named `name` holding the task, worked by a claude pane opened
    /// for it as a lane dispatch opens one.
    async fn lane_pane(&self, name: &str) -> Terminal {
        let cards = [LaneCard { task_id: self.task.id, slice: String::new() }];
        let new = NewLane { name: name.into(), ..Default::default() };
        self.svc.store.create_lane(self.task.workspace_id, &new, &cards, None, Actor::Manager).unwrap();
        self.svc.create_terminal_in_lane(self.lane.id, "Lane", "claude", None, Some(self.task.id), Some(name)).await.unwrap()
    }
}

fn agent_actor(t: &Terminal) -> String {
    format!("agent:{}", t.id)
}

fn refused<T: std::fmt::Debug>(r: Result<T>) -> &'static str {
    match r {
        Err(DomainError::Conflict { what } | DomainError::InvalidArgument { what }) => what,
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

/// A lane is reached by its name or by any card on it, and what its agent
/// sends is tagged with the lane.
#[tokio::test]
async fn a_lane_is_addressed_by_name_or_card_and_tags_its_own() {
    let b = board().await;
    let pane = b.lane_pane("mac-ux").await;
    let si = b.stand_in(&pane, "claude", "claude").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    assert_eq!(b.send("manager", "MAC-UX", "Ship it").await.unwrap().recipient, "the lane mac-ux");
    assert_eq!(b.send("manager", &b.task.key, "Then stop").await.unwrap().recipient, "the lane mac-ux");
    assert!(b.messages().iter().all(|w| w.to == Some(pane.id)));
    b.pump().await;
    si.submits(1).await;
    b.doing(pane.id, AgentActivity::Idle).await;
    tokio::time::sleep(Duration::from_millis(TOLD_SPACING_MS as u64 + 100)).await;
    b.pump().await;
    si.submits(2).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it", "[from the orchestrator] Then stop"], "in order: {}", si.log());

    let orchestrator = b.orchestrator().await;
    let osi = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.send(&agent_actor(&pane), "orchestrator", "Done").await.unwrap();
    b.pump().await;
    osi.submits(1).await;
    assert_eq!(osi.submitted(), ["[from mac-ux] Done"], "{}", osi.log());
}

/// Hub and spoke: an agent can't message another lane or card; nobody
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

/// A lane's agent that ends its turn without reporting has its orchestrator
/// told, once; one that reported isn't told about.
#[tokio::test]
async fn a_silent_stop_tells_the_orchestrator_once() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let pane = b.lane_pane("phones").await;
    let finished = AgentNews::Finished { label: "Lane".into(), said: Some("All green.".into()) };
    let began = now_millis() - 1_000;
    b.watcher.lane_stopped(pane.id, &b.task, &finished, Some(began));
    b.watcher.lane_stopped(pane.id, &b.task, &finished, Some(began));
    assert_eq!(b.messages().len(), 1, "one notice waits at a time");
    b.pump().await;
    si.submits(1).await;
    let key = &b.task.key;
    let said = format!("[Far Cooler] The lane phones's agent, on {key}, ended its turn without reporting. It last said: “All green.”");
    assert_eq!(si.submitted(), [said], "{}", si.log());

    b.svc.store.add_note(b.task.id, NoteKind::Decision, Actor::Agent { terminal: pane.id }, "Chose B", serde_json::json!({})).unwrap();
    b.watcher.lane_stopped(pane.id, &b.task, &finished, Some(began));
    assert!(b.messages().is_empty(), "it reported this turn");
    b.watcher.lane_stopped(pane.id, &b.task, &AgentNews::Failed { label: "Lane".into() }, Some(began));
    assert_eq!(b.messages().len(), 1, "a failed turn is told whatever it wrote");
}

/// The watcher's own transition is what tells: a lane agent read Done after
/// working, with an orchestrator running, queues the runner's notice.
#[tokio::test]
async fn a_done_transition_queues_the_notice() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let _si = b.stand_in(&orchestrator, "claude", "claude").await;
    let pane = b.lane_pane("phones").await;
    let lane = b.stand_in(&pane, "claude", "claude").await;
    lane.show("working").await;
    b.doing(pane.id, AgentActivity::Working).await;
    lane.show("idle").await;
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
    let pane = b.lane_pane("phones").await;
    let finished = AgentNews::Finished { label: "Lane".into(), said: None };
    b.watcher.lane_stopped(pane.id, &b.task, &finished, None);
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
