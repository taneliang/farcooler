//! `terminal.agent_answer` with `answers` (ov-370 review 1 M3): a claude
//! question a hook holds, answered over the RPC with the wire's
//! `AgentAnswer.answers = 4`, as every app and the CLI send it.

use std::collections::HashMap;

use farcooler_agent_hooks::wire::{Decision, LONGEST_HOLD};
use farcooler_protocol::v1::{AgentAnswer, Request, Scope, TerminalIntent, response};
use farcooler_transport::Handler;

use super::*;
use crate::hook_asks::AskShape;

fn question() -> serde_json::Value {
    serde_json::json!({ "questions": [
        { "question": "Which color?", "header": "Color", "options": [{ "label": "Red" }, { "label": "Blue" }], "multiSelect": false },
        { "question": "Which sizes?", "header": "Sizes", "options": [{ "label": "S" }, { "label": "L" }], "multiSelect": true },
    ] })
}

/// A runner with one claude pane holding a question, and what its hook is
/// told.
async fn a_held_question() -> (tempfile::TempDir, Arc<Service>, Uuid, String, tokio::task::JoinHandle<Option<Decision>>) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap());
    let host = Uuid::now_v7();
    let root = svc.store.create_repository_root(host, "/repos/q", 1_000).unwrap();
    let repo = svc.store.create_repository(host, root.id, "repo", "/repos/q/.git", "").unwrap();
    let wt = svc.store.create_worktree(repo.id, "main", "/repos/q", true).unwrap();
    let terminal = svc.store.create_terminal(wt.id, "pane", "claude", TerminalIntent::Running, 80, 24).unwrap().id;
    let shape = AskShape::Question { input: question() };
    let (id, rx) = svc.hooks().asks().hold_shaped(terminal, Some("AskUserQuestion"), shape, LONGEST_HOLD);
    let hook = tokio::spawn(async move {
        let settled = rx.await.ok()?;
        if let Some(ack) = settled.ack {
            let _ = ack.send(());
        }
        settled.decision
    });
    (dir, svc, terminal, id, hook)
}

fn an_answer(terminal: Uuid, id: &str, answers: &[(&str, &str)]) -> Request {
    Request {
        method: "terminal.agent_answer".into(),
        payload: Some(request::Payload::AgentAnswer(AgentAnswer {
            terminal_id: crate::wire::id_bytes(terminal),
            request_id: id.into(),
            option_id: "answer".into(),
            answers: answers.iter().map(|(q, a)| (q.to_string(), a.to_string())).collect::<HashMap<_, _>>(),
        })),
        ..Default::default()
    }
}

async fn handle(svc: &Arc<Service>, req: Request) -> Option<(i32, String)> {
    let factory = RpcFactory::new(
        svc.clone(),
        crate::watch::Watcher::new(svc.clone()),
        Arc::new(tokio::sync::Notify::new()),
        Peer { client_id: None, scope: Scope::Control },
    );
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(_)) => None,
        Some(response::Outcome::Error(e)) => Some((e.code, e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

#[tokio::test]
async fn a_question_is_answered_over_the_rpc_with_its_answers() {
    let (_dir, svc, terminal, id, hook) = a_held_question().await;
    let answers = [("Which color?", "Blue"), ("Which sizes?", "S, L")];
    assert_eq!(handle(&svc, an_answer(terminal, &id, &answers)).await, None, "answered");
    let Some(Decision::Allow { updated_input: Some(input) }) = hook.await.unwrap() else { panic!("not an allow with an input") };
    assert_eq!(input["answers"], serde_json::json!({ "Which color?": "Blue", "Which sizes?": "S, L" }));
    assert_eq!(input["questions"], question()["questions"]);
}

/// A partial answer is refused naming `answers`, which the apps say as
/// "Answer every question first.", and the question stays held.
#[tokio::test]
async fn a_partial_answer_is_refused_by_name_and_the_question_stays_held() {
    let (_dir, svc, terminal, id, _hook) = a_held_question().await;
    let refused = handle(&svc, an_answer(terminal, &id, &[("Which color?", "Blue")])).await;
    let expected = DomainError::InvalidArgument { what: "answers" };
    assert_eq!(refused, Some((expected.wire().0 as i32, expected.what().to_string())));
    assert!(svc.hooks().asks().is_holding(terminal));
}
