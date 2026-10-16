//! Messages to and from a member's rep, and a person's own (ov-494), typed
//! into stand-ins on the real tmux server of `tests`.

use super::messages_tests::{agent_actor, refused};
use super::*;
use crate::people::Person;

impl Board {
    /// Alice and Bob as members, Bob's phone his, and Alice's rep running
    /// on this board.
    pub(super) async fn alices_rep(&self) -> (Terminal, Task) {
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
    let pb::result::Value::MessageSent(about) = b.watcher.message_send(req, Person::Owner, pb::Scope::HostAdmin).await.unwrap() else { panic!() };
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
    // From a rep, refused as no messenger's (ov-527), whatever the connection's scope.
    assert_eq!(refused(b.send_as(Person::Owner, &agent_actor(&rep), "rep:alice", "me", "").await), "rep_messenger");
    assert_eq!(refused(b.send_as(Person::Owner, &agent_actor(&rep), "mac-ux", "psst", "").await), "rep_messenger");
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
    b.send_as(bob(), "user", "orchestrator", "Ship Friday", &b.task.key).await.expect("Bob leads now");
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from Bob] Ship Friday"], "{}", si.log());
    b.send_as(Person::Owner, "user", "orchestrator", "Me too", &b.task.key).await.expect("the owner always may (R-54)");
    assert_eq!(refused(b.send_as(Person::Released, "user", "orchestrator", "Still me", &b.task.key).await), "not_lead", "a released phone");
    assert_eq!(refused(b.send_as(Person::Released, "manager", "orchestrator", "x", &b.task.key).await), "actor");
    assert_eq!(refused(b.send_as(Person::Owner, "manager", "orchestrator", "x", "").await), "self", "the orchestrator isn't a person");
}

/// What tmux was told to run in `terminal`'s pane.
async fn launched(b: &Board, terminal: Uuid) -> String {
    let snapshot = b.svc.inventory.refresh().await;
    let pane = snapshot.claimants(terminal).into_iter().next().expect("a pane").clone();
    let out = b.svc.tmux.run(&["display-message", "-p", "-t", &pane.pane_id, "#{pane_start_command}"]).await.unwrap();
    out.stdout.trim().to_string()
}

/// Write `pane`'s conversation where a restart looks for it
/// (`Service::agent_home`), so there's one to resume.
fn keep_conversation(b: &Board, pane: &Terminal) {
    let dir = b.svc.store.get_worktree(pane.worktree_id).unwrap().worktree_path;
    let dir = std::fs::canonicalize(dir).unwrap();
    let sid = pane.agent_session_id.as_deref().expect("a session");
    let file = b.svc.agent_home().unwrap().join(".claude/projects").join(crate::session_discovery::project_dir_name(&dir)).join(format!("{sid}.jsonl"));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, "{}\n").unwrap();
}

/// A message to a stopped rep resumes it, then reaches it (R-66: no idle
/// parking, a rep comes back on demand). The send itself brings the pane
/// back before anything is typed: on its own conversation when there is
/// one, and on its first message, so on its skill, when there isn't.
#[tokio::test]
async fn a_message_to_a_stopped_rep_wakes_it_then_arrives() {
    let b = board().await;
    let (rep, desk) = b.alices_rep().await;
    keep_conversation(&b, &rep);
    b.svc.stop_terminal(rep.id).await.unwrap();
    assert!(!b.svc.is_running(&b.svc.store.get_terminal(rep.id).unwrap()), "stopped");

    let sent = b.send_as(Person::Owner, "manager", "rep:alice", "Ship it Friday?", "").await.unwrap();
    assert_eq!((sent.recipient.as_str(), sent.task_key.as_str()), ("Alice Chen's rep", desk.key.as_str()));
    let now = b.svc.store.get_terminal(rep.id).unwrap();
    assert!(b.svc.is_running(&now), "the message resumed the rep");
    assert_eq!(now.agent_session_id, rep.agent_session_id, "on its own conversation");
    let command = launched(&b, rep.id).await;
    assert!(command.contains(&format!("--resume {}", rep.agent_session_id.as_deref().unwrap())), "{command}");
    assert!(!command.contains("/farcooler:rep"), "{command}");

    // The resumed claude, as a stand-in, takes the message the pump types.
    let si = b.stand_in(&rep, "claude", "claude").await;
    b.doing(rep.id, AgentActivity::Idle).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it Friday?"], "{}", si.log());

    // With no conversation on disk, it comes back on its first message.
    b.svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    let bobs = b.svc.start_rep(b.task.workspace_id, "bob", "claude").await.unwrap().terminal;
    b.svc.stop_terminal(bobs.id).await.unwrap();
    b.send_as(Person::Owner, "manager", "rep:bob", "Ship it Friday?", "").await.unwrap();
    assert!(b.svc.is_running(&b.svc.store.get_terminal(bobs.id).unwrap()), "woken");
    let command = launched(&b, bobs.id).await;
    assert!(!command.contains("--resume") && command.contains("/farcooler:rep You represent Bob (bob)"), "{command}");
}

/// Only someone who could have started a rep wakes it with a message
/// (`rep.start`'s rule): the workspace's lead, who isn't its member, is
/// queued but leaves it stopped; its own member wakes it.
#[tokio::test]
async fn only_who_may_start_a_rep_wakes_it() {
    let b = board().await;
    let (rep, _) = b.alices_rep().await;
    b.svc.store.set_workspace_lead(b.task.workspace_id, Some("bob")).unwrap();
    b.svc.stop_terminal(rep.id).await.unwrap();

    b.send_as(bob(), "user", "rep:alice", "Alice, ship Friday?", "").await.expect("the lead may message her rep");
    assert!(!b.svc.is_running(&b.svc.store.get_terminal(rep.id).unwrap()), "but not wake it");
    assert_eq!(b.messages().len(), 1, "it waits for her rep's next start");

    b.send_as(Person::Member("alice".into()), "user", "rep:alice", "I'm back", "").await.expect("her own rep");
    assert!(b.svc.is_running(&b.svc.store.get_terminal(rep.id).unwrap()), "she wakes her own");
}

/// A rep messages only on its own board: one naming another workspace is
/// refused (ov-527).
#[tokio::test]
async fn a_rep_messages_only_on_its_own_board() {
    let b = board().await;
    let (rep, _) = b.alices_rep().await;
    let repo = b.svc.store.get_workspace(b.task.workspace_id).unwrap().repository_id;
    let billing = b.svc.store.create_workspace(repo, "Billing", "bil").unwrap().id;
    let req = pb::Request {
        method: "message.send".into(),
        payload: Some(pb::request::Payload::MessageSend(pb::MessageSend {
            to: "orchestrator".into(),
            text: "Alice asks, over there".into(),
            actor: agent_actor(&rep),
            task: String::new(),
            workspace_id: billing.as_bytes().to_vec().into(),
        })),
        ..Default::default()
    };
    assert_eq!(refused(b.watcher.message_send(req, Person::Owner, pb::Scope::HostAdmin).await), "rep_messenger");
    assert!(b.messages().is_empty());
}

/// A rep's message wakes no rep: to another rep it's refused before any
/// wake, even on a runner admin's connection, and that rep stays stopped.
#[tokio::test]
async fn a_rep_never_wakes_a_rep() {
    let b = board().await;
    let (alices, _) = b.alices_rep().await;
    b.svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    let bobs = b.svc.start_rep(b.task.workspace_id, "bob", "claude").await.unwrap().terminal;
    b.svc.stop_terminal(bobs.id).await.unwrap();
    let req = pb::Request {
        method: "message.send".into(),
        payload: Some(pb::request::Payload::MessageSend(pb::MessageSend {
            to: "rep:bob".into(),
            text: "Wake up".into(),
            actor: agent_actor(&alices),
            task: String::new(),
            workspace_id: b.task.workspace_id.as_bytes().to_vec().into(),
        })),
        ..Default::default()
    };
    assert_eq!(refused(b.watcher.message_send(req, Person::Owner, pb::Scope::HostAdmin).await), "rep_messenger");
    assert!(!b.svc.is_running(&b.svc.store.get_terminal(bobs.id).unwrap()), "Bob's rep stays stopped");
}
