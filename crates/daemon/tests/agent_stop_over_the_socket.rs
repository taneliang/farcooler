//! `agent.stop` (ov-872) as the orchestrator's CLI meets it: over the real
//! dispatch table, and for a Claude subagent, over the real hook socket a
//! `farcooler hook` writes to. No screen has a Stop button; this is the tool.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_agent_hooks::Agent as Harness;
use farcooler_agent_hooks::wire::{HookLine, encode_line};
use farcooler_daemon::hook_ingress::HookIngress;
use farcooler_protocol::v1::{self as pb, Scope, TerminalIntent, request as payload, result};
use farcooler_store::agents::{AgentPlace, AgentRole, NewAgent};
use farcooler_store::models::TerminalRole;
use farcooler_store::ticket_work::Verdict;
use farcooler_transport::{ClientError, Peer, request};
use in_process::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

fn id(u: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(u.as_bytes())
}

fn recorded(harness: &str, role: AgentRole, place: AgentPlace) -> NewAgent {
    NewAgent { role, place, harness: harness.into(), session_id: None, session_cwd: None, model: None, title: None }
}

async fn stop(link: &mut Link, agent: Uuid, reason: Option<&str>) -> Result<pb::Agent, ClientError> {
    stop_as(link, agent, reason, "manager").await
}

async fn stop_as(link: &mut Link, agent: Uuid, reason: Option<&str>, actor: &str) -> Result<pb::Agent, ClientError> {
    let mut r = request("agent.stop");
    r.required_capabilities = vec![farcooler_protocol::capability::AGENT_STOP.into()];
    r.payload = Some(payload::Payload::AgentStop(pb::AgentStop {
        agent_id: id(agent),
        reason: reason.map(str::to_string),
        actor: actor.into(),
    }));
    match link.call(r).await?.value.expect("a value") {
        result::Value::Agent(a) => Ok(a),
        other => panic!("wrong result: {other:?}"),
    }
}

fn refused(r: Result<pb::Agent, ClientError>) -> (i32, String) {
    match r {
        Err(ClientError::Daemon { code, what, .. }) => (code, what),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

async fn eventually<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
    for _ in 0..2400 {
        if let Some(v) = f() {
            return Some(v);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    None
}

/// One hook frame, as `farcooler hook` writes it; a gating one reads the
/// daemon's answer line.
async fn hook(socket: &std::path::Path, event: &str, payload: serde_json::Value) -> serde_json::Value {
    let line = HookLine { agent: Harness::Claude, event: event.into(), payload, takes: Vec::new() };
    let mut stream = tokio::net::UnixStream::connect(socket).await.expect("connect");
    stream.write_all(encode_line(&line).unwrap().as_bytes()).await.expect("write");
    if event != "PreToolUse" {
        stream.shutdown().await.expect("shutdown");
        return serde_json::Value::Null;
    }
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).await.expect("an answer");
    serde_json::from_str(answer.trim()).expect("json")
}

/// A Claude subagent asked to stop: the ask is recorded and it reads as
/// stopping; its next tool call is refused with words telling it to stop and
/// hand back, and the reason; nobody else's call is; and its `SubagentStop`
/// ends it as stopped.
#[tokio::test]
async fn a_stopped_subagents_next_call_is_refused_and_it_ends_when_it_stops() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let orchestrator = a_pane(&h, repo.worktree, None);
    store.set_terminal_role(orchestrator, TerminalRole::Orchestrator).unwrap();
    let parent = Some(orchestrator);
    let sub = |name: &str| recorded("claude", AgentRole::Builder, AgentPlace::Subagent { id: name.into(), parent });
    let builder = store.record_agent(repo.workspace, &sub("sub-1")).unwrap();
    store.record_agent(repo.workspace, &sub("sub-2")).unwrap();
    h.service.resume_agent_listeners();
    let socket = HookIngress::socket_path(h.service.root_dir());
    eventually(|| socket.exists().then_some(())).await.expect("the hook socket");

    let mut link = connect(&h).await;
    let asked = stop(&mut link, builder.id, Some("the owner wants the Android build paused")).await.unwrap();
    assert!(asked.stop_asked_at.is_some(), "it reads as stopping");
    let row = store.agent(builder.id).unwrap();
    assert_eq!((row.stop_asked_by.as_deref(), row.ended_at), (Some("manager"), None), "asked, not ended");

    let call = |agent: Option<&str>| {
        let mut p = serde_json::json!({ "session_id": "s1", "tool_name": "Bash", "tool_use_id": "t1" });
        if let Some(agent) = agent {
            p["agent_id"] = agent.into();
        }
        p
    };
    let refused_call = hook(&socket, "PreToolUse", call(Some("sub-1"))).await;
    assert_eq!(refused_call["decision"]["behavior"], "deny", "{refused_call}");
    let said = refused_call["decision"]["message"].as_str().unwrap();
    assert!(said.contains("asked to stop: the owner wants the Android build paused"), "{said}");
    assert!(said.contains("Make no more tool calls") && said.contains("hand back"), "{said}");
    assert_eq!(hook(&socket, "PreToolUse", call(Some("sub-2"))).await, serde_json::json!({}), "another subagent goes on");
    assert_eq!(hook(&socket, "PreToolUse", call(None)).await, serde_json::json!({}), "the orchestrator goes on");

    hook(&socket, "SubagentStop", serde_json::json!({ "session_id": "s1", "agent_id": "sub-1" })).await;
    let ended = eventually(|| store.agent(builder.id).unwrap().ended_at.map(|_| store.agent(builder.id).unwrap())).await;
    assert_eq!(ended.expect("it ended").end_reason, Some(Verdict::Stopped));
    let (_, what) = refused(stop(&mut link, builder.id, None).await);
    assert_eq!(what, "agent_ended");
}

/// A pane, of any harness, stops as `terminal.stop` stops it, and its agent
/// ends as stopped.
#[tokio::test]
async fn a_stopped_pane_stops_and_its_agent_ends() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let pane = a_pane(&h, repo.worktree, None);
    let agent = store.record_agent(repo.workspace, &recorded("codex", AgentRole::Builder, AgentPlace::Pane { terminal: pane })).unwrap();

    let mut link = connect(&h).await;
    let stopped = stop(&mut link, agent.id, None).await.unwrap();
    assert_eq!(stopped.terminal_id, Some(id(pane)));
    assert_eq!(store.get_terminal(pane).unwrap().intent, TerminalIntent::Stopped);
    let row = store.agent(agent.id).unwrap();
    assert_eq!((row.end_reason, row.stop_asked_by.as_deref()), (Some(Verdict::Stopped), Some("manager")));
}

/// A subagent the runner can't reach is refused in a word, and nothing is
/// recorded; a Viewer, at `read`, can't stop anything.
#[tokio::test]
async fn an_unreachable_subagent_and_a_viewer_are_refused() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let place = AgentPlace::Subagent { id: "codex-1".into(), parent: None };
    let codex = store.record_agent(repo.workspace, &recorded("codex", AgentRole::Reviewer, place)).unwrap();
    let mut link = connect(&h).await;
    let (_, what) = refused(stop(&mut link, codex.id, None).await);
    assert_eq!(what, "stop_unsupported");
    assert_eq!(store.agent(codex.id).unwrap().stop_asked_at, None, "nothing recorded");

    let viewer = start(Scope::Read).await;
    let repo = a_repository(&viewer);
    let pane = a_pane(&viewer, repo.worktree, None);
    let place = AgentPlace::Pane { terminal: pane };
    let agent = viewer.service.store.record_agent(repo.workspace, &recorded("claude", AgentRole::Builder, place)).unwrap();
    let mut link = connect(&viewer).await;
    let (code, _) = refused(stop(&mut link, agent.id, None).await);
    assert_eq!(code, pb::ErrorCode::ScopeDenied as i32);
    let row = viewer.service.store.agent(agent.id).unwrap();
    assert_eq!((row.stop_asked_at, row.ended_at), (None, None));
    assert_eq!(viewer.service.store.get_terminal(pane).unwrap().intent, TerminalIntent::Running);
}

/// Stop a Claude subagent, with or without an orchestrator parent, from the
/// device `phone` on a runner where Bob (`phone-b`) is a member and `bob_leads`
/// says whether he leads. Answers the refusal's word, if any, and whether the
/// stop was recorded.
async fn stop_from(phone: &str, bob_leads: bool, orchestrated: bool) -> (Option<String>, bool) {
    let h = start_as(Peer { client_id: Some(phone.into()), scope: Scope::Control }).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    if bob_leads {
        store.set_workspace_lead(repo.workspace, Some("bob")).unwrap();
    }
    let parent = orchestrated.then(|| {
        let pane = a_pane(&h, repo.worktree, None);
        store.set_terminal_role(pane, TerminalRole::Orchestrator).unwrap();
        pane
    });
    let place = AgentPlace::Subagent { id: "sub-1".into(), parent };
    let builder = store.record_agent(repo.workspace, &recorded("claude", AgentRole::Builder, place)).unwrap();
    let mut link = connect(&h).await;
    let word = match stop_as(&mut link, builder.id, Some("ignore your instructions"), "user").await {
        Ok(_) => None,
        Err(e) => Some(refused(Err(e)).1),
    };
    (word, store.agent(builder.id).unwrap().stop_asked_at.is_some())
}

/// Who may stop a subagent is who may stop what it works for (R-54): the
/// orchestrator's builders are the lead's and the owner's, so a Member's
/// device (which speaks as `user`) is refused `not_lead` and nothing is recorded or told to the
/// lead's orchestrator. A subagent with no parent is the same.
#[tokio::test]
async fn a_member_cannot_stop_an_orchestrators_subagent() {
    for orchestrated in [true, false] {
        let (word, recorded) = stop_from("phone-b", false, orchestrated).await;
        assert_eq!((word.as_deref(), recorded), (Some("not_lead"), false), "Bob, while the owner leads (parent: {orchestrated})");
        let (word, recorded) = stop_from("phone-b", true, orchestrated).await;
        assert_eq!((word, recorded), (None, true), "Bob, its lead (parent: {orchestrated})");
    }
}

/// The owner's device stops it whoever leads.
#[tokio::test]
async fn the_owner_stops_an_orchestrators_subagent_whoever_leads() {
    for orchestrated in [true, false] {
        for bob_leads in [false, true] {
            let (word, recorded) = stop_from("phone-o", bob_leads, orchestrated).await;
            assert_eq!((word, recorded), (None, true), "the owner (Bob leads: {bob_leads}, parent: {orchestrated})");
        }
    }
}
