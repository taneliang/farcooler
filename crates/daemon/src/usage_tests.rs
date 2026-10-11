use std::sync::Arc;

use farcooler_core::session_log::usage::LogUsage;
use farcooler_protocol::v1::TerminalIntent;
use farcooler_store::models::Actor;

use super::*;

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
