use std::sync::{Arc, Mutex};

use farcooler_agent::event::{AgentEvent, EndReason, Sequenced};
use farcooler_agent::usage::{ModelUsage, TurnUsage};
use farcooler_core::session_log::usage::LogUsage;
use farcooler_protocol::v1::TerminalIntent;
use farcooler_store::models::Actor;

use super::*;
use crate::agent_supervisor::AgentSupervisor;

/// A runner with one task, and an agent terminal opened for it on `preset`.
async fn a_task_with_an_agent(
    preset: &str,
) -> (crate::test_support::ScratchDir, Arc<Service>, Uuid, Uuid, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap();
    let task = svc.store.create_task(main.id, "Record spend", Actor::User).unwrap();
    let terminal = svc
        .store
        .create_terminal_for_task(checkout.id, "agent", preset, TerminalIntent::Running, 80, 24, Some(task.id))
        .unwrap();
    svc.store.set_terminal_workspace(terminal.id, main.id).unwrap();
    (dir, svc, task.id, terminal.id, repo)
}

/// The recorded claude turn's usage (`crates/claude/tests/fixtures/turn_basic.jsonl`,
/// as `farcooler_claude::usage` reads it).
fn recorded_claude_turn() -> TurnUsage {
    TurnUsage {
        key: "claude:9e766346-8b18-4f9f-b11d-575d07f891fd".into(),
        models: Some(vec![ModelUsage {
            model: Some("claude-opus-5[1m]".into()),
            input: 2,
            output: 4,
            cache_read: 19912,
            cache_write: 7018,
            cache_write_1h: 0,
            reported_cost_micros: Some(80_246),
        }]),
        active_ms: Some(3948),
        partial: false,
    }
}

/// A chat pane's spend is filed under its task, and no client ever sees the
/// event that carried it; a shim replaying it files nothing new.
#[tokio::test]
async fn a_chat_turns_spend_is_filed_under_its_task_and_never_sent() {
    let (_dir, svc, task, terminal, _) = a_task_with_an_agent("claude").await;
    let supervisor = AgentSupervisor::with_records(svc.store.clone());
    let sent: Arc<Mutex<Vec<Sequenced>>> = Default::default();
    let tap = sent.clone();
    let on_events = move |_: Uuid, batch: Vec<Sequenced>| tap.lock().unwrap().extend(batch);
    let usage = AgentEvent::TurnUsage { usage: recorded_claude_turn() };
    supervisor.record(terminal, vec![AgentEvent::TurnEnded { reason: EndReason::EndTurn }, usage.clone()], &on_events);
    supervisor.record(terminal, vec![usage], &on_events);

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "only the turn's end reaches a client: {sent:?}");
    assert!(matches!(sent[0].event, AgentEvent::TurnEnded { .. }));

    let reply = task_usage(&svc, task);
    let totals = reply.totals.unwrap();
    assert_eq!(totals.turns, 1, "heard twice, recorded once");
    assert_eq!(totals.active_ms, 3948);
    assert_eq!((totals.cost_reported_micros, totals.cost_estimated_micros), (80_246, 0));
    assert_eq!(totals.cache_read_tokens, 19912);
    let split: Vec<_> = reply.by_harness_model.iter().map(|g| (g.harness.clone(), g.model.clone())).collect();
    assert_eq!(split, vec![(Some("claude".into()), Some("claude-opus-5[1m]".into()))]);
}

/// A chat turn that reported nothing is counted as a turn, with no tokens and
/// no cost, and said to be unreported.
#[tokio::test]
async fn a_chat_turn_with_no_usage_is_counted_as_not_reported() {
    let (_dir, svc, task, terminal, _) = a_task_with_an_agent("gemini").await;
    let silent = TurnUsage { key: "acp:s:1".into(), models: None, active_ms: None, partial: false };
    record_chat(&svc.store, terminal, &silent);
    let totals = task_usage(&svc, task).totals.unwrap();
    assert_eq!((totals.turns, totals.turns_not_reported), (1, 1));
    assert_eq!(totals.input_tokens + totals.output_tokens, 0);
    assert_eq!(totals.cost_reported_micros + totals.cost_estimated_micros, 0);
}

/// A terminal agent's turn, read from its recorded session log: filed under
/// the task, priced from the table with the table's date, and reportable by
/// task, harness, model and day.
#[tokio::test]
async fn a_terminal_agents_logged_turn_is_estimated_and_reportable() {
    let (_dir, svc, task, terminal, repo) = a_task_with_an_agent("claude").await;
    let recorded = concat!(env!("CARGO_MANIFEST_DIR"), "/../core/fixtures/session-logs/claude-complete-turn.jsonl");
    let mut fold = LogUsage::default();
    std::fs::read_to_string(recorded).unwrap().lines().for_each(|l| fold.claude_line(l));
    record_log(&svc.store, terminal, "claude", fold.take());

    let query = pb::UsageQuery {
        repository_id: Some(bytes::Bytes::copy_from_slice(repo.as_bytes())),
        group_by: vec![
            pb::UsageDimension::Task as i32,
            pb::UsageDimension::Harness as i32,
            pb::UsageDimension::Model as i32,
            pb::UsageDimension::Day as i32,
        ],
        ..Default::default()
    };
    let reply = report(&svc, &query).unwrap();
    assert_eq!(reply.groups.len(), 1);
    let g = &reply.groups[0];
    assert_eq!(g.task_id.as_deref(), Some(task.as_bytes().as_slice()));
    assert!(!g.task_key.is_empty(), "the task's key, for a report to print");
    assert_eq!((g.harness.as_deref(), g.model.as_deref()), (Some("claude"), Some("claude-opus-5")));
    assert_eq!(g.period.as_deref(), Some("2026-08-16"));
    let t = g.totals.as_ref().unwrap();
    assert_eq!((t.output_tokens, t.active_ms), (868, 14681));
    assert_eq!(t.cost_reported_micros, 0);
    // 4 in, 868 out, 55,595 cache reads, 11,792 one-hour writes at claude-opus-5's rates.
    assert_eq!(t.cost_estimated_micros, 167_438);
    assert_eq!(t.price_tables, vec![PRICE_TABLE.to_string()]);
    assert_eq!(reply.total.unwrap().turns, 1);
}

/// A terminal claude's subagent, read from its own transcript, is filed with
/// the pane that started it: under its task, as a run and not a turn, kept
/// current as it grows.
#[tokio::test]
async fn a_subagents_spend_is_filed_under_its_parents_task() {
    let (_dir, svc, task, terminal, _) = a_task_with_an_agent("claude").await;
    let recorded = concat!(env!("CARGO_MANIFEST_DIR"), "/../core/fixtures/session-logs/claude-subagent-transcript.jsonl");
    let lines: Vec<String> = std::fs::read_to_string(recorded).unwrap().lines().map(str::to_string).collect();
    let mut fold = LogUsage::default();
    lines[..4].iter().for_each(|l| fold.subagent_line("a1", l));
    record_log(&svc.store, terminal, "claude", fold.take());
    lines[4..].iter().for_each(|l| fold.subagent_line("a1", l));
    record_log(&svc.store, terminal, "claude", fold.take());

    let totals = task_usage(&svc, task).totals.unwrap();
    assert_eq!((totals.turns, totals.subagent_runs, totals.active_ms), (0, 1, 0));
    assert_eq!((totals.output_tokens, totals.cache_read_tokens), (681, 110_128));
    assert!(totals.cost_estimated_micros > 0, "priced from the table like any claude call");
}

#[tokio::test]
async fn an_unknown_dimension_is_refused() {
    let (_dir, svc, _, _, _) = a_task_with_an_agent("claude").await;
    let query = pb::UsageQuery { group_by: vec![0], ..Default::default() };
    assert!(matches!(report(&svc, &query), Err(DomainError::InvalidArgument { what: "group_by" })));
}

fn task_usage(svc: &Service, task: Uuid) -> pb::TaskUsage {
    task_fn(svc, &pb::TaskUsageRequest { task_id: bytes::Bytes::copy_from_slice(task.as_bytes()) }).unwrap()
}

use super::task as task_fn;
