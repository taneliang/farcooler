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

use farcooler_core::session_log::projector::{HINT_ID, RowKind, prompt_images};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, request, result};
use uuid::Uuid;

use crate::service::Service;
use crate::session_projectors::{self, Follow, Page, RowChange};
use crate::subagent_rows;
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
            let page = match p.agent_id.is_empty() {
                true => open.then(|| session_projectors::global().read_page(terminal, p.before, limit)).flatten(),
                false => match subagent_path(open, terminal, &p.agent_id)? {
                    Some(path) => {
                        let (agent, before) = (p.agent_id.clone(), p.before);
                        tokio::task::spawn_blocking(move || subagent_rows::global().page(terminal, &agent, path, before, limit)).await.ok()
                    }
                    None => None,
                },
            };
            let mut page = page.unwrap_or(Page { epoch: 0, rev: 0, rows: Vec::new(), more_before: false });
            keep_hint_for(&mut page.rows, p.hint_rows);
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
            let follow = match p.agent_id.is_empty() {
                true => follow_answer(session_projectors::global(), terminal, &p, arrived, ensure_open(svc, terminal)).await?,
                false => subagent_follow(terminal, &p, arrived, ensure_open(svc, terminal)).await?,
            };
            Ok(result::Value::AgentRowChanges(pb_changes(terminal, follow, p.hint_rows)))
        }
        _ => Err(DomainError::InvalidArgument { what: "payload" }),
    }
}

/// `agent.image` (ov-454): a piece of one image a prompt carried, read back
/// from its record in the transcript (`prompt_images::read`). The image is
/// decoded once and held for the pieces after it (`RecentImages`). Refused
/// as not found for a row that isn't a turn, an index it has no image at,
/// or a record that can't be read back.
pub(crate) async fn image(svc: &Service, req: Request) -> Result<result::Value> {
    if !session_projectors::shadowing() {
        return Err(DomainError::CapabilityUnsupported { needed: farcooler_protocol::capability::AGENT_ROWS });
    }
    let Some(request::Payload::AgentImage(p)) = req.payload else { return Err(DomainError::InvalidArgument { what: "payload" }) };
    let terminal = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
    ensure_open(svc, terminal).await?;
    let key = (terminal, p.row_id.clone(), p.index);
    let held = RECENT.lock().unwrap_or_else(|e| e.into_inner()).get(&key);
    let (mime, bytes) = match held {
        Some(held) => held,
        None => {
            let source = session_projectors::global()
                .with_session(terminal, |s| match s.projection().row(&p.row_id).map(|r| &r.kind) {
                    Some(RowKind::Turn(turn)) if (p.index as usize) < turn.images.len() => turn.source.clone(),
                    _ => None,
                })
                .flatten()
                .ok_or(DomainError::NotFound)?;
            let prompt = p.row_id.strip_prefix("turn:").unwrap_or_default().to_string();
            let index = p.index as usize;
            let read = tokio::task::spawn_blocking(move || prompt_images::read(&source, &prompt, index))
                .await
                .ok()
                .flatten()
                .ok_or(DomainError::NotFound)?;
            let held = (read.0, std::sync::Arc::new(read.1));
            RECENT.lock().unwrap_or_else(|e| e.into_inner()).put(key, held.clone());
            held
        }
    };
    let start = (p.offset as usize).min(bytes.len());
    let end = (start + farcooler_protocol::MAX_AGENT_IMAGE_CHUNK).min(bytes.len());
    Ok(result::Value::AgentImage(pb::AgentImage {
        mime_type: mime,
        total_size: bytes.len() as u64,
        offset: start as u64,
        chunk: bytes::Bytes::copy_from_slice(&bytes[start..end]),
    }))
}

/// The images `agent.image` decoded last, by terminal, row and index: a
/// client asks for one a piece at a time, and a view shows a few at once.
struct RecentImages(Vec<(ImageKey, Decoded)>);

type ImageKey = (Uuid, String, u32);

/// An image's MIME type and bytes, shared by the pieces served from it.
type Decoded = (String, std::sync::Arc<Vec<u8>>);

/// How many decoded images are held: a message's worth.
const RECENT_IMAGES: usize = 4;

impl RecentImages {
    fn get(&mut self, key: &ImageKey) -> Option<Decoded> {
        let at = self.0.iter().position(|(k, _)| k == key)?;
        let entry = self.0.remove(at);
        let held = entry.1.clone();
        self.0.push(entry);
        Some(held)
    }

    fn put(&mut self, key: ImageKey, held: Decoded) {
        self.0.retain(|(k, _)| *k != key);
        self.0.push((key, held));
        let over = self.0.len().saturating_sub(RECENT_IMAGES);
        self.0.drain(..over);
    }
}

static RECENT: std::sync::Mutex<RecentImages> = std::sync::Mutex::new(RecentImages(Vec::new()));

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

/// Where subagent `agent` of `terminal` writes its own transcript, beside
/// the pane's (ov-453). `None` while the pane has no session open; refused
/// for an id claude never writes.
fn subagent_path(open: bool, terminal: Uuid, agent: &str) -> Result<Option<PathBuf>> {
    let Some(main) = open.then(|| session_projectors::global().transcript(terminal)).flatten() else { return Ok(None) };
    farcooler_core::session_log::projector::subagent_transcript(&main, agent).map(Some).ok_or(DomainError::InvalidArgument { what: "agent_id" })
}

/// A follow of a subagent's own rows, its wait counted from `arrived` as
/// `follow_answer`'s is.
async fn subagent_follow(
    terminal: Uuid,
    p: &pb::AgentRowsFollow,
    arrived: tokio::time::Instant,
    open: impl std::future::Future<Output = Result<bool>>,
) -> Result<Follow> {
    let deadline = arrived + Duration::from_millis(u64::from(p.wait_ms)).min(MAX_WAIT);
    match subagent_path(open.await?, terminal, &p.agent_id)? {
        Some(path) => {
            Ok(subagent_rows::global().follow(terminal, &p.agent_id, path, p.epoch, p.after_rev, deadline, session_projectors::MAX_CHANGES).await)
        }
        None => {
            tokio::time::sleep_until(deadline).await;
            Ok(Follow::Changes { epoch: 0, rev: 0, changes: Vec::new() })
        }
    }
}

/// Open `terminal`'s projector if it has none, from its transcript on disk.
/// `false` for a terminal that runs no claude session. A codex pane's is
/// opened by the watcher's tick, on the rollout its process holds open
/// (`registry_join::feed_codex_projector`, ov-378): until then, `false`, and
/// a follower that waited pages again once it has one.
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

/// `rows` as a client that said whether it draws the `Hint` row may see them.
fn keep_hint_for(rows: &mut Vec<farcooler_core::session_log::projector::Row>, hint_rows: bool) {
    if !hint_rows {
        rows.retain(|row| row.id != HINT_ID);
    }
}

/// A follow on the wire. The `Hint` row goes only to a client that said it
/// draws one (`hint_rows`): an older client counts it as a row it cannot draw,
/// and a fresh session would lose its empty state.
fn pb_changes(terminal: Uuid, follow: Follow, hint_rows: bool) -> pb::AgentRowChanges {
    use pb::AgentRowChangeKind as Kind;
    let (epoch, rev, changes, reset) = match follow {
        Follow::Reset { epoch, rev } => (epoch, rev, Vec::new(), true),
        Follow::Changes { epoch, rev, changes } => (epoch, rev, changes, false),
    };
    let changes = changes
        .iter()
        .filter(|change| {
            let id = match change {
                RowChange::Insert(row) | RowChange::Update(row) => &row.id,
                RowChange::Remove { id, .. } => id,
            };
            hint_rows || id != HINT_ID
        })
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
