//! Messages to and from a member's rep, and a person's own (ov-494), typed
//! into stand-ins on the real tmux server of `tests`.

use super::messages_tests::{agent_actor, refused};
use super::*;
use crate::people::Person;

impl Board {
    /// Alice and Bob as members, Bob's phone his, and Alice's rep running
    /// on this board.
    async fn alices_rep(&self) -> (Terminal, Task) {
        self.svc.store.set_member("alice", "Alice Chen", &[]).unwrap();
        self.svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
        let started = self.svc.start_rep(self.task.workspace_id, "alice", "claude").await.expect("a rep");
        (started.terminal, started.desk)
    }
}

fn bob() -> Person {
    Person::Member("bob".into())
}

/// A rep's message reaches the orchestrator tagged with its person, filed
/// on their desk card unless it names another; the card keeps who it's from.
#[tokio::test]
async fn a_reps_message_reaches_the_orchestrator_with_its_person() {
    let b = board().await;
    let (rep, desk) = b.alices_rep().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let sent = b.send_as(Person::Owner, &agent_actor(&rep), "orchestrator", "Alice asks for dark mode", "").await.unwrap();
    assert_eq!((sent.task_key.as_str(), sent.recipient.as_str()), (desk.key.as_str(), "the orchestrator"));
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from Alice Chen's rep] Alice asks for dark mode"], "{}", si.log());
    let filed = b.svc.store.notes_for(desk.id, Some(NoteKind::Comment)).unwrap();
    assert_eq!(filed[0].extra["message"]["from"], "Alice Chen's rep");

    let named = b.svc.store.create_task(b.task.workspace_id, "Dark mode", Actor::Manager).unwrap();
    let req = pb::Request {
        method: "message.send".into(),
        payload: Some(pb::request::Payload::MessageSend(pb::MessageSend {
            to: "orchestrator".into(),
            text: "Alice answers: yes, Friday".into(),
            actor: agent_actor(&rep),
            task: named.key.clone(),
            workspace_id: b.task.workspace_id.as_bytes().to_vec().into(),
        })),
        ..Default::default()
    };
    let pb::result::Value::MessageSent(about) = b.watcher.message_send(req, Person::Owner).await.unwrap() else { panic!() };
    assert_eq!(about.task_key, named.key, "a card it names");
}

/// The orchestrator asks a person through `rep:<handle>`; the rep gets it.
/// A handle nobody has, or a member with no rep, is refused.
#[tokio::test]
async fn the_orchestrator_asks_a_person_through_their_rep() {
    let b = board().await;
    let (rep, desk) = b.alices_rep().await;
    let si = b.stand_in(&rep, "claude", "claude").await;
    b.doing(rep.id, AgentActivity::Idle).await;
    let sent = b.send_as(Person::Owner, "manager", "rep:alice", "Ship it Friday?", "").await.unwrap();
    assert_eq!((sent.recipient.as_str(), sent.task_key.as_str()), ("Alice Chen's rep", desk.key.as_str()));
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it Friday?"], "{}", si.log());
    assert_eq!(refused(b.send_as(Person::Owner, "manager", "rep:carol", "Hi", "").await), "to");
    assert_eq!(refused(b.send_as(Person::Owner, "manager", "rep:bob", "Hi", "").await), "no_rep");
}

/// Hub and spoke holds for reps: a rep messages its orchestrator only, and
/// no lane messages a rep.
#[tokio::test]
async fn a_rep_messages_nobody_but_its_orchestrator() {
    let b = board().await;
    let (rep, _) = b.alices_rep().await;
    let lane = b.lane_pane("mac-ux").await;
    assert_eq!(refused(b.send_as(Person::Owner, &agent_actor(&rep), "rep:alice", "me", "").await), "hub");
    assert_eq!(refused(b.send_as(Person::Owner, &agent_actor(&rep), "mac-ux", "psst", "").await), "hub");
    assert_eq!(refused(b.send_as(Person::Owner, &agent_actor(&lane), "rep:alice", "psst", "").await), "hub");
    assert!(b.messages().is_empty());
}

/// A person's own message comes from the lead only, except to their own rep;
/// a member's device never speaks as a pane or the orchestrator; and what a
/// member sends is tagged with their name.
#[tokio::test]
async fn only_the_lead_messages_as_a_person() {
    let b = board().await;
    let (_, _) = b.alices_rep().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    assert_eq!(refused(b.send_as(bob(), "user", "orchestrator", "Do it my way", &b.task.key).await), "not_lead");
    assert_eq!(refused(b.send_as(bob(), "", "orchestrator", "Do it my way", &b.task.key).await), "not_lead", "empty is `user`");
    assert_eq!(refused(b.send_as(bob(), "manager", "orchestrator", "I'm the boss", &b.task.key).await), "actor");
    assert_eq!(refused(b.send_as(Person::Member("alice".into()), "user", "rep:bob", "Hi", "").await), "not_lead", "not hers");
    b.svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    let bobs = b.svc.start_rep(b.task.workspace_id, "bob", "claude").await.unwrap();
    assert_eq!(b.send_as(bob(), "user", "rep:bob", "Ask for a dark mode", "").await.unwrap().recipient, "Bob's rep", "his own rep");
    assert_eq!(b.svc.store.message_from(b.messages()[0].note).unwrap().as_deref(), Some("Bob"));
    let _ = bobs;

    b.svc.store.set_workspace_lead(b.task.workspace_id, Some("bob")).unwrap();
    assert_eq!(refused(b.send_as(Person::Owner, "user", "orchestrator", "Me too", &b.task.key).await), "not_lead", "one lead");
    b.send_as(bob(), "user", "orchestrator", "Ship Friday", &b.task.key).await.expect("Bob leads now");
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from Bob] Ship Friday"], "{}", si.log());
    b.send_as(Person::Owner, "manager", "orchestrator", "x", "").await.err().expect("the orchestrator still can't message itself");
}
