//! A landing that leaves for its checks by itself is told to its orchestrator
//! once (ov-520, ov-872), typed into a stand-in on the real tmux server of
//! `tests`.

use super::*;
use farcooler_store::landings::{LandingState, LandingUpdate, NewLanding, Route};
use farcooler_store::ticket_lines::{LineUpdate, NewTicketLine, Stage};

impl Board {
    /// A card on the task's board, on the line and taken to `last`.
    fn ticket_to(&self, title: &str, last: Stage) -> Task {
        let task = match title {
            "" => self.task.clone(),
            title => self.svc.store.create_task(self.task.workspace_id, title, Actor::Manager).unwrap(),
        };
        self.svc.store.put_on_line(task.id, &NewTicketLine::default(), Actor::Manager).unwrap();
        for s in [Stage::Building, Stage::Reviewing, Stage::Landing] {
            let up = LineUpdate { stage: Some(s), ..Default::default() };
            self.svc.store.update_line(task.id, &up, Actor::Manager).unwrap();
            if s == last {
                break;
            }
        }
        task
    }

    /// The runner's notices about landings leaving, filed on `tasks` for the
    /// orchestrator.
    fn departure_notices(&self, tasks: &[&Task]) -> Vec<String> {
        let mut said = Vec::new();
        for task in tasks {
            let notes = self.svc.store.notes_for(task.id, None).unwrap();
            said.extend(
                notes.into_iter().filter(|n| n.actor == Actor::Runner && n.body.contains(" is ready for its checks")).map(|n| n.body),
            );
        }
        said
    }
}

/// A ready ticket with nothing ahead sends the collecting landing off, and the
/// orchestrator is told once: not on the next tick, not by a restart's fresh
/// watcher. A landing it sends off later is a new notice.
#[tokio::test]
async fn a_landing_that_left_by_itself_is_told_to_the_orchestrator_exactly_once() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let first_ticket = b.ticket_to("", Stage::Landing);
    let second_ticket = b.ticket_to("Fix x", Stage::Landing);
    let ws = b.task.workspace_id;
    // The first left as soon as it was ready, and the second waits on the next landing.
    let first = b.svc.store.landing_numbered(ws, 1).unwrap();
    assert_eq!(first.state, LandingState::Gating);
    assert_eq!(first.tickets, vec![first_ticket.id]);

    b.watcher.tell_departures();
    b.watcher.tell_departures();
    Watcher::new(b.svc.clone()).tell_departures();
    let told = b.departure_notices(&[&first_ticket, &second_ticket]);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].starts_with(&format!("Landing 1 is ready for its checks: {}.", first_ticket.key)), "{}", told[0]);
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted().len(), 1, "{}", si.log());

    // The landing ahead lands; the next one leaves, and is told as well.
    b.svc.store.update_landing(first.id, &LandingUpdate { sha: Some("1a1b3275".into()), ..Default::default() }, Actor::Manager).unwrap();
    b.svc.store.update_landing(first.id, &LandingUpdate { state: Some(LandingState::Landed), ..Default::default() }, Actor::Manager).unwrap();
    b.watcher.tell_departures();
    let told = b.departure_notices(&[&first_ticket, &second_ticket]);
    assert_eq!(told.len(), 2, "{told:?}");
    assert!(told[1].starts_with(&format!("Landing 2 is ready for its checks: {}.", second_ticket.key)), "{}", told[1]);
}

/// With no orchestrator running, nothing is sent and nothing is recorded; once
/// there is one, the departure is told, once.
#[tokio::test]
async fn nothing_is_sent_or_recorded_without_a_running_orchestrator() {
    let b = board().await;
    let ticket = b.ticket_to("", Stage::Landing);
    let ws = b.task.workspace_id;
    let landing = b.svc.store.landing_numbered(ws, 1).unwrap();

    b.watcher.tell_departures();
    assert!(b.departure_notices(&[&ticket]).is_empty(), "no orchestrator, no notice");
    let subject = farcooler_store::ticket_lines::LineSubject::Landing(landing.id);
    let told: Vec<_> = b.svc.store.line_events(subject, 0).unwrap().into_iter().filter(|e| e.kind == "told").collect();
    assert!(told.is_empty(), "and no record that it was told");

    let orchestrator = b.orchestrator().await;
    let _si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.watcher.tell_departures();
    b.watcher.tell_departures();
    assert_eq!(b.departure_notices(&[&ticket]).len(), 1, "told once there is someone to tell");
}

/// A landing the orchestrator started by hand never departs by itself, so
/// there is nothing to tell, and a ticket ready behind it waits.
#[tokio::test]
async fn a_landing_started_by_hand_is_not_announced() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let _si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let ws = b.task.workspace_id;
    let new = NewLanding { route: Route::Push, base: String::new(), title: String::new(), lander: None, collecting: false };
    let hand = b.svc.store.start_landing(ws, &new, &[], Actor::Manager).unwrap();
    let ticket = b.ticket_to("", Stage::Landing);
    b.watcher.tell_departures();
    assert_eq!(b.svc.store.landing(hand.id).unwrap().state, LandingState::Open);
    assert!(b.departure_notices(&[&ticket]).is_empty());
}

/// A departure caused by an RPC is told in that call, not on the next backstop
/// tick: a ticket passing review over `line.set` sends the landing off and the
/// orchestrator has the notice when the call returns.
#[tokio::test]
async fn a_departure_caused_by_an_rpc_is_told_in_that_call() {
    use farcooler_protocol::v1::{self as pb, Scope, request};
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let _si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let ticket = b.ticket_to("", Stage::Reviewing);
    assert!(b.departure_notices(&[&ticket]).is_empty());
    let req = pb::Request {
        method: "line.set".into(),
        payload: Some(request::Payload::LineSet(pb::TicketLineSet {
            task_id: bytes::Bytes::copy_from_slice(ticket.id.as_bytes()),
            stage: Some(pb::TicketStage::Landing as i32),
            actor: "manager".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    crate::rpc_plan::dispatch(&b.svc, &b.watcher, Scope::Control, req).await.expect("line.set");
    let told = b.departure_notices(&[&ticket]);
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].starts_with(&format!("Landing 1 is ready for its checks: {}.", ticket.key)), "{}", told[0]);
    assert!(told[0].contains("start your own with `landing start`"), "{}", told[0]);
    assert!(!told[0].to_lowercase().contains("train"), "{}", told[0]);
}

#[path = "notice_wait_tests.rs"]
mod notice_wait_tests;
