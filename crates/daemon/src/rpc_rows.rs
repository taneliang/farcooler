//! `agent.rows` and `agent.rows_follow` (ov-366): a terminal's projected
//! agent rows, a page at a time and then by revision.
//!
//! A terminal with no projector open gets one here, rebuilt from its
//! transcript on disk (so the first call after a daemon restart is the one
//! that reads it back). The transcript is found from the terminal's record:
//! the session it says it runs (which `HookIngress` keeps current through
//! `/clear`), under claude's project directory for the worktree, else under
//! whichever project directory holds that session's file (an orchestrator
//! runs from its own directory). The rebuild runs on a blocking thread.
//!
//! Behind `FARCOOLER_PROJECTOR=1` (see `session_projectors`): without it
//! both are refused as unsupported.

use std::path::PathBuf;
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, request, result};
use uuid::Uuid;

use crate::service::Service;
use crate::session_projectors::{self, Follow, Page, RowChange};
use crate::wire;

/// Rows in a page a client asked no size for: about a screen and a half.
pub const DEFAULT_PAGE: usize = 100;
/// The most rows one page carries.
pub const MAX_PAGE: usize = 500;
/// The longest a follow is held open.
pub const MAX_WAIT: Duration = Duration::from_secs(25);

pub(crate) async fn dispatch(svc: &Service, req: Request) -> Result<result::Value> {
    // A follow's wait counts from here, so a first follow that rebuilds a
    // transcript still answers inside the client's 30 s deadline.
    let arrived = tokio::time::Instant::now();
    if req.method == "settings.set_projector" {
        return set_projector(req);
    }
    if !session_projectors::shadowing() {
        return Err(DomainError::CapabilityUnsupported { needed: farcooler_protocol::capability::AGENT_ROWS });
    }
    match (req.method.as_str(), req.payload) {
        ("agent.rows", Some(request::Payload::AgentRowsPage(p))) => {
            let terminal = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
            let open = ensure_open(svc, terminal).await?;
            let limit = match p.limit as usize {
                0 => DEFAULT_PAGE,
                n => n.min(MAX_PAGE),
            };
            let page = open.then(|| session_projectors::global().read_page(terminal, p.before, limit)).flatten();
            let page = page.unwrap_or(Page { epoch: 0, rev: 0, rows: Vec::new(), more_before: false });
            Ok(result::Value::AgentRowPage(pb::AgentRowPage {
                terminal_id: wire::id_bytes(terminal),
                epoch: page.epoch,
                rev: page.rev,
                rows: page.rows.iter().map(pb_row).collect(),
                more_before: page.more_before,
            }))
        }
        ("agent.rows_follow", Some(request::Payload::AgentRowsFollow(p))) => {
            let terminal = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
            let follow = follow_answer(session_projectors::global(), terminal, &p, arrived, ensure_open(svc, terminal)).await?;
            Ok(result::Value::AgentRowChanges(pb_changes(terminal, follow)))
        }
        _ => Err(DomainError::InvalidArgument { what: "payload" }),
    }
}

/// `settings.set_projector` (ov-372): `[agents] projector` written to
/// config.toml, then the projector turned on or off in this daemon at once.
/// Refused, with nothing changed, when the file can't be written.
fn set_projector(req: Request) -> Result<result::Value> {
    let Some(request::Payload::HostSettings(p)) = req.payload else {
        return Err(DomainError::InvalidArgument { what: "payload" });
    };
    let path = farcooler_core::config::config_path().ok_or(DomainError::OperationFailed)?;
    farcooler_core::config::write_projector(&path, p.projector).map_err(|e| {
        tracing::warn!(error = %e, "couldn't write the projector setting");
        DomainError::OperationFailed
    })?;
    session_projectors::set_shadowing(p.projector);
    Ok(result::Value::Empty(pb::Empty {}))
}

/// A follow's answer. Its wait is counted from `arrived`, before `open`
/// (which may rebuild a whole transcript) has run, so the call is answered
/// inside the client's deadline however long the rebuild took.
pub(crate) async fn follow_answer(
    projectors: &session_projectors::SessionProjectors,
    terminal: Uuid,
    p: &pb::AgentRowsFollow,
    arrived: tokio::time::Instant,
    open: impl std::future::Future<Output = Result<bool>>,
) -> Result<Follow> {
    let deadline = arrived + Duration::from_millis(u64::from(p.wait_ms)).min(MAX_WAIT);
    let follow = match open.await? {
        true => projectors.follow(terminal, p.epoch, p.after_rev, deadline, session_projectors::MAX_CHANGES).await,
        false => None,
    };
    Ok(match follow {
        Some(follow) => follow,
        // No session to follow: nothing changes, after the wait, so a
        // follower does not spin.
        None => {
            tokio::time::sleep_until(deadline).await;
            Follow::Changes { epoch: 0, rev: 0, changes: Vec::new() }
        }
    })
}

/// Open `terminal`'s projector if it has none, from its transcript on disk.
/// `false` for a terminal that runs no claude session.
async fn ensure_open(svc: &Service, terminal: Uuid) -> Result<bool> {
    let projectors = session_projectors::global();
    if projectors.is_open(terminal) {
        return Ok(true);
    }
    let term = svc.store.get_terminal(terminal)?;
    let Some(session) = term.agent_session_id.filter(|_| term.command_preset.starts_with("claude")) else { return Ok(false) };
    let worktree = svc.store.get_worktree(term.worktree_id)?.worktree_path;
    let Some(config) = crate::claude_registry::config_dir() else { return Ok(false) };
    let built = tokio::task::spawn_blocking(move || {
        let path = transcript_of(&config, &worktree, &session);
        projectors.open(terminal, path);
    })
    .await;
    if let Err(e) = built {
        tracing::warn!(error = %e, "could not rebuild a terminal's agent rows");
    }
    Ok(projectors.is_open(terminal))
}

/// Where claude writes session `session`'s transcript for a pane in
/// `worktree`: under the worktree's project directory, else under whichever
/// one holds the file, else where the worktree's would be once it is written.
pub fn transcript_of(config: &std::path::Path, worktree: &str, session: &str) -> PathBuf {
    let file = format!("{session}.jsonl");
    let projects = config.join("projects");
    let resolved = std::fs::canonicalize(worktree).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| worktree.to_string());
    let own = projects.join(farcooler_core::session_log::claude_slug(&resolved)).join(&file);
    if own.exists() {
        return own;
    }
    let elsewhere = std::fs::read_dir(&projects)
        .into_iter()
        .flatten()
        .flatten()
        .map(|dir| dir.path().join(&file))
        .find(|path| path.exists());
    elsewhere.unwrap_or(own)
}

fn pb_row(row: &farcooler_core::session_log::projector::Row) -> pb::AgentRow {
    pb::AgentRow {
        id: row.id.clone(),
        ord: row.ord,
        rev: row.rev,
        // A row is plain data: strings, numbers and enums. Serializing one
        // cannot fail, and an empty object is what a client skips if it did.
        row_json: serde_json::to_string(row).unwrap_or_else(|_| "{}".into()),
    }
}

fn pb_changes(terminal: Uuid, follow: Follow) -> pb::AgentRowChanges {
    use pb::AgentRowChangeKind as Kind;
    let (epoch, rev, changes, reset) = match follow {
        Follow::Reset { epoch, rev } => (epoch, rev, Vec::new(), true),
        Follow::Changes { epoch, rev, changes } => (epoch, rev, changes, false),
    };
    let changes = changes
        .iter()
        .map(|change| match change {
            RowChange::Insert(row) | RowChange::Update(row) => pb::AgentRowChange {
                kind: if matches!(change, RowChange::Insert(_)) { Kind::Insert } else { Kind::Update } as i32,
                id: row.id.clone(),
                rev: row.rev,
                row: Some(pb_row(row)),
            },
            RowChange::Remove { id, rev } => pb::AgentRowChange { kind: Kind::Remove as i32, id: id.clone(), rev: *rev, row: None },
        })
        .collect();
    pb::AgentRowChanges { terminal_id: wire::id_bytes(terminal), epoch, rev, changes, reset }
}

#[cfg(test)]
#[path = "rpc_rows_tests.rs"]
mod tests;
