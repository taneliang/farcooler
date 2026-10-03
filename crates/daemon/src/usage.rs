//! Recording what each finished agent turn spent, and answering for it.
//!
//! Two ways in, one row shape (`farcooler_store::usage`):
//!
//! - a chat pane's backend says it at the turn's end, as an
//!   `AgentEvent::TurnUsage` that `AgentSupervisor::record` takes out of the
//!   stream before anything is numbered or sent ([`record_chat`]);
//! - a terminal agent's session log says it, read by the same tail the
//!   activity follower already keeps (`watch::PaneLog`) and folded by
//!   `session_log::usage` ([`record_log`]).
//!
//! The two never see one turn: an agent-mode pane is never log-tailed (see
//! `watch::sample`). Each turn is filed where it happened, read from the
//! terminal's row when it ended: its worktree and that worktree's repository,
//! its workspace, and its task by `task_link::task_of`, the link everything
//! else on the board uses.
//!
//! Then two reads for reports: `usage.report`, totals by any mix of task,
//! worktree, workspace, repository, harness, model and period; and
//! `usage.task`, one task's totals for its view.

use farcooler_agent::usage::TurnUsage;
use farcooler_core::session_log::usage::LoggedTurn;
use farcooler_core::usage::{PRICE_TABLE, TokenCounts};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::Store;
use farcooler_store::usage::{GroupBy, NewTurn, Surface, TurnKind, TurnModel, UsageFilter, UsageGroup, UsageTotals};
use uuid::Uuid;

use crate::service::Service;

/// Where a terminal's work is filed.
#[derive(Default)]
struct Place {
    terminal: Option<Uuid>,
    worktree: Option<Uuid>,
    repository: Option<Uuid>,
    workspace: Option<Uuid>,
    task: Option<Uuid>,
    preset: Option<String>,
}

fn place(store: &Store, terminal: Uuid) -> Place {
    let Ok(row) = store.get_terminal(terminal) else {
        // A turn from a terminal already removed is still a turn somebody
        // paid for; it is counted, unfiled.
        return Place { terminal: Some(terminal), ..Default::default() };
    };
    Place {
        terminal: Some(terminal),
        worktree: Some(row.worktree_id),
        repository: store.get_worktree(row.worktree_id).ok().map(|w| w.repository_id),
        workspace: row.workspace_id,
        task: crate::task_link::task_of(store, &row).map(|t| t.id),
        preset: Some(row.command_preset.clone()),
    }
}

/// The harness a preset names: its first word (`claude`, `codex`, or an ACP
/// adapter's name).
fn harness(preset: Option<&str>) -> String {
    preset.and_then(|p| p.split_whitespace().next()).unwrap_or("unknown").to_string()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// A chat pane's turn, as its backend reported it.
pub(crate) fn record_chat(store: &Store, terminal: Uuid, usage: &TurnUsage) {
    let p = place(store, terminal);
    let ended_at = now_ms();
    let active_ms = usage.active_ms.map(|ms| ms as i64);
    let models: Vec<TurnModel> = usage
        .models
        .iter()
        .flatten()
        .map(|m| {
            let tokens = TokenCounts {
                input: m.input,
                output: m.output,
                cache_read: m.cache_read,
                cache_write: m.cache_write,
                cache_write_1h: m.cache_write_1h,
            };
            TurnModel::priced(m.model.clone(), tokens, m.reported_cost_micros)
        })
        .collect();
    let turn = NewTurn {
        key: usage.key.clone(),
        terminal_id: p.terminal,
        worktree_id: p.worktree,
        repository_id: p.repository,
        workspace_id: p.workspace,
        task_id: p.task,
        harness: harness(p.preset.as_deref()),
        surface: Surface::Chat,
        started_at: active_ms.map(|ms| ended_at - ms),
        ended_at,
        active_ms,
        usage: match (&usage.models, usage.partial) {
            (None, _) => "not_reported",
            (Some(_), true) => "partial",
            (Some(_), false) => "reported",
        },
        models,
        kind: TurnKind::Turn,
    };
    write(store, &turn);
}

/// A terminal agent's turns, as its session log told them. `harness` is the
/// log's format (`claude`, `codex`), which is what the log can vouch for.
pub(crate) fn record_log(store: &Store, terminal: Uuid, harness: &str, turns: Vec<LoggedTurn>) {
    if turns.is_empty() {
        return;
    }
    let p = place(store, terminal);
    for t in turns {
        let models = t
            .models
            .into_iter()
            .map(|(model, tokens)| TurnModel::priced(model, tokens, None))
            .collect();
        let turn = NewTurn {
            key: t.key,
            terminal_id: p.terminal,
            worktree_id: p.worktree,
            repository_id: p.repository,
            workspace_id: p.workspace,
            task_id: p.task,
            harness: harness.to_string(),
            surface: Surface::Terminal,
            started_at: t.started_at_ms,
            ended_at: t.ended_at_ms.unwrap_or_else(now_ms),
            active_ms: t.active_ms,
            usage: t.state.as_str(),
            models,
            kind: if t.subagent { TurnKind::Subagent } else { TurnKind::Turn },
        };
        write(store, &turn);
    }
}

fn write(store: &Store, turn: &NewTurn) {
    if let Err(error) = store.record_turn(turn) {
        // Spend is bookkeeping: a failed write costs a report a row, and is
        // never a reason to disturb the agent that did the work.
        tracing::warn!(key = %turn.key, %error, "could not record an agent turn's usage");
    }
}

fn id(bytes: Option<&bytes::Bytes>, what: &'static str) -> Result<Option<Uuid>> {
    match bytes {
        None => Ok(None),
        Some(raw) => crate::wire::parse_id(raw).map(Some).ok_or(DomainError::InvalidArgument { what }),
    }
}

fn dimension(raw: i32) -> Result<GroupBy> {
    use pb::UsageDimension as D;
    Ok(match D::try_from(raw).unwrap_or(D::Unspecified) {
        D::Task => GroupBy::Task,
        D::Worktree => GroupBy::Worktree,
        D::Workspace => GroupBy::Workspace,
        D::Repository => GroupBy::Repository,
        D::Harness => GroupBy::Harness,
        D::Model => GroupBy::Model,
        D::Day => GroupBy::Day,
        D::Week => GroupBy::Week,
        D::Month => GroupBy::Month,
        D::Unspecified => return Err(DomainError::InvalidArgument { what: "group_by" }),
    })
}

/// `usage.report`.
pub fn report(svc: &Service, q: &pb::UsageQuery) -> Result<pb::UsageReport> {
    let filter = UsageFilter {
        since: (q.since_ms != 0).then_some(q.since_ms),
        until: (q.until_ms != 0).then_some(q.until_ms),
        task_id: id(q.task_id.as_ref(), "task_id")?,
        worktree_id: id(q.worktree_id.as_ref(), "worktree_id")?,
        workspace_id: id(q.workspace_id.as_ref(), "workspace_id")?,
        repository_id: id(q.repository_id.as_ref(), "repository_id")?,
        harness: q.harness.clone(),
        model: q.model.clone(),
    };
    let group_by = q.group_by.iter().map(|d| dimension(*d)).collect::<Result<Vec<_>>>()?;
    let groups = svc.store.usage_summary(&filter, &group_by, q.utc_offset_minutes)?;
    let total = svc.store.usage_summary(&filter, &[], q.utc_offset_minutes)?.pop().map(|g| g.totals);
    Ok(pb::UsageReport {
        group_by: q.group_by.clone(),
        groups: groups.iter().map(|g| group(&svc.store, g)).collect(),
        total: Some(totals(&total.unwrap_or_default())),
        price_table: PRICE_TABLE.to_string(),
    })
}

/// `usage.task`.
pub fn task(svc: &Service, q: &pb::TaskUsageRequest) -> Result<pb::TaskUsage> {
    let task = crate::task_ops::required_id(&q.task_id)?;
    let (total, split) = svc.store.task_usage(task)?;
    Ok(pb::TaskUsage {
        task_id: q.task_id.clone(),
        totals: Some(totals(&total)),
        by_harness_model: split.iter().map(|g| group(&svc.store, g)).collect(),
        price_table: PRICE_TABLE.to_string(),
    })
}

fn group(store: &Store, g: &UsageGroup) -> pb::UsageGroup {
    let blob = |id: Option<Uuid>| id.map(|id| bytes::Bytes::copy_from_slice(id.as_bytes()));
    pb::UsageGroup {
        task_id: blob(g.key.task_id),
        worktree_id: blob(g.key.worktree_id),
        workspace_id: blob(g.key.workspace_id),
        repository_id: blob(g.key.repository_id),
        harness: g.key.harness.clone(),
        model: g.key.model.clone(),
        period: g.key.period.clone(),
        task_key: g.key.task_id.and_then(|t| store.get_task(t).ok()).map(|t| t.key).unwrap_or_default(),
        totals: Some(totals(&g.totals)),
    }
}

fn totals(t: &UsageTotals) -> pb::UsageTotals {
    pb::UsageTotals {
        turns: t.turns,
        turns_partial: t.turns_partial,
        turns_not_reported: t.turns_not_reported,
        subagent_runs: t.subagent_runs,
        active_ms: t.active_ms,
        input_tokens: t.tokens.input,
        output_tokens: t.tokens.output,
        cache_read_tokens: t.tokens.cache_read,
        cache_write_tokens: t.tokens.cache_write,
        cost_reported_micros: t.cost_reported_micros,
        cost_estimated_micros: t.cost_estimated_micros,
        unpriced_tokens: t.unpriced_tokens,
        price_tables: t.price_tables.clone(),
        first_ended_at_ms: t.first_ended_at.unwrap_or(0),
        last_ended_at_ms: t.last_ended_at.unwrap_or(0),
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
