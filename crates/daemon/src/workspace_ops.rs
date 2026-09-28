//! Workspace operations, one function per RPC method.
//!
//! Separate from `rpc.rs` so the dispatch table stays a table, and from
//! `farcooler_store::workspaces` so the wire shapes stay out of the store.
//! The rules themselves (a prefix unique on the runner, Main never deleted,
//! a claim that sticks, one live orchestrator) are the store's; this file
//! reads requests, calls it, and says what changed.
//!
//! **Every mutation announces `fleet_changed`.** A workspace, its worktrees
//! and its terminals are what a sidebar draws, and a change nobody announces
//! leaves every other client drawing the old one. `task.move` also announces
//! `task_changed` per task that moved, naming both boards.
//!
//! **Every id here is a WORKSPACE id where the store asks for one.**
//! `Store::create_task` and `Store::next_task_key` take a bare `Uuid` that
//! means a workspace; a repository id handed to either type-checks and fails
//! as `NotFound`.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Scope};
use farcooler_store::models::Workspace;
use uuid::Uuid;

use crate::service::Service;
use crate::task_ops::{actor_from_wire, pb_task, required_id};
use crate::watch::Watcher;
use crate::wire;

/// One workspace as the wire carries it, with its live orchestrator
/// (`Service::live_orchestrator`: a lost one isn't named).
fn pb_workspace(svc: &Service, workspace: &Workspace, scope: Scope) -> Result<pb::Workspace> {
    let orchestrator = svc.live_orchestrator(workspace.id)?.map(|t| t.id);
    Ok(wire::workspace(workspace, orchestrator, &svc.workspace_home(workspace.id), scope))
}

/// The version a caller read, or the one there now when it named none.
fn version(svc: &Service, id: Uuid, expected: Option<u64>) -> Result<u64> {
    match expected {
        Some(version) => Ok(version),
        None => Ok(svc.store.get_workspace(id)?.resource_version),
    }
}

/// `workspace.list`: the workspaces in one repository, Main first, or every
/// workspace on the runner when no repository is named.
pub fn list(svc: &Service, repository: Option<Uuid>, scope: Scope) -> Result<pb::WorkspaceList> {
    if let Some(repository) = repository {
        // So a repository that isn't there is `NotFound`, not an empty list.
        svc.store.get_repository(repository)?;
    }
    let items = svc
        .store
        .list_workspaces(repository)?
        .iter()
        .map(|w| pb_workspace(svc, w, scope))
        .collect::<Result<Vec<_>>>()?;
    Ok(pb::WorkspaceList { items })
}

/// `workspace.create`: a new workspace in `repository`, after the others.
///
/// Its home is made here, with a charter copied from Main's when Main has
/// one. A home that can't be made doesn't undo the workspace: it's logged,
/// and the daemon's next start makes it (`prepare_workspace_homes`).
pub fn create(
    svc: &Service,
    watcher: &Watcher,
    repository: Uuid,
    req: &pb::WorkspaceCreate,
    scope: Scope,
) -> Result<pb::Workspace> {
    let workspace = svc.store.create_workspace(repository, &req.name, &req.task_prefix)?;
    if let Err(e) = svc.ensure_workspace_home(&workspace) {
        tracing::warn!(workspace = %workspace.id, error = %e, "could not make a new workspace's home");
    }
    watcher.announce_fleet_changed();
    pb_workspace(svc, &workspace, scope)
}

/// `workspace.rename`.
pub fn rename(
    svc: &Service,
    watcher: &Watcher,
    id: Uuid,
    req: &pb::WorkspaceRename,
    scope: Scope,
) -> Result<pb::Workspace> {
    let expected = version(svc, id, req.expected_version)?;
    let workspace = svc.store.rename_workspace(id, expected, &req.name)?;
    watcher.announce_fleet_changed();
    pb_workspace(svc, &workspace, scope)
}

/// `workspace.set_prefix`. Keys already issued keep resolving.
pub fn set_prefix(
    svc: &Service,
    watcher: &Watcher,
    id: Uuid,
    req: &pb::WorkspaceSetPrefix,
    scope: Scope,
) -> Result<pb::Workspace> {
    let expected = version(svc, id, req.expected_version)?;
    let workspace = svc.store.set_workspace_prefix(id, expected, &req.task_prefix)?;
    watcher.announce_fleet_changed();
    pb_workspace(svc, &workspace, scope)
}

/// `workspace.delete`: refused for Main and for a workspace anything still
/// belongs to. Its home is left on disk: the charter in it is the user's
/// own words, and deleting a workspace shouldn't delete those. Its
/// orchestrator's settings file, which is only Far Cooler's, goes
/// (`remove_orchestrator_settings`).
pub fn delete(svc: &Service, watcher: &Watcher, id: Uuid) -> Result<pb::Empty> {
    svc.store.delete_workspace(id)?;
    crate::service::remove_orchestrator_settings(svc.root_dir(), id);
    watcher.announce_fleet_changed();
    Ok(pb::Empty {})
}

/// `task.move`: put tasks on another board in the same repository.
///
/// All or nothing, in the store. Each task that actually moved is announced
/// naming both boards, so the board it left re-reads as well as the one it
/// arrived on; a task that was already there is not announced, because
/// nothing about it changed.
pub fn move_tasks(svc: &Service, watcher: &Watcher, req: &pb::TaskMove) -> Result<pb::TaskList> {
    let to = required_id(&req.workspace_id)?;
    let actor = actor_from_wire(&req.actor)?;
    let ids = req.task_ids.iter().map(|id| required_id(id)).collect::<Result<Vec<_>>>()?;
    if ids.is_empty() {
        return Err(DomainError::InvalidArgument { what: "task_ids" });
    }
    // Where each one was, read before the move so the announce can say it.
    let before = ids
        .iter()
        .map(|&id| svc.store.get_task(id).map(|t| t.workspace_id))
        .collect::<Result<Vec<_>>>()?;
    let moved = svc.store.move_tasks(&ids, to, actor)?;
    let mut any = false;
    for (task, from) in moved.iter().zip(before) {
        if from != task.workspace_id {
            watcher.announce_task_changed(task, Some(from), actor);
            any = true;
        }
    }
    watcher.announce_fleet_changed();
    // An item takes its task's board as its workspace.
    if any {
        watcher.announce_needs_you();
    }
    Ok(pb::TaskList { items: moved.iter().map(pb_task).collect() })
}

/// `worktree.assign`: give a worktree to a workspace, whoever owned it.
/// Answers with the worktree as a client now sees it.
pub async fn assign_worktree(
    svc: &Service,
    watcher: &Watcher,
    worktree: Uuid,
    req: &pb::WorktreeAssign,
    scope: Scope,
) -> Result<pb::Worktree> {
    let workspace = required_id(&req.workspace_id)?;
    let assigned = svc.store.assign_worktree(worktree, workspace)?;
    watcher.announce_fleet_changed();
    let view = svc.worktree_view(&assigned).await?;
    Ok(wire::worktree(&view, scope))
}

/// `terminal.set_role`: shell, agent, or its workspace's orchestrator.
/// UNSPECIFIED is refused (`role`) rather than read as a default. Answers
/// with nothing; the caller re-reads the terminal, as `rpc.rs` does.
pub async fn set_role(
    svc: &Service,
    watcher: &Watcher,
    terminal: Uuid,
    req: &pb::TerminalSetRole,
) -> Result<()> {
    let role =
        wire::terminal_role_from_wire(req.role).ok_or(DomainError::InvalidArgument { what: "role" })?;
    svc.set_terminal_role(terminal, role).await?;
    watcher.announce_fleet_changed();
    // An orchestrator's items are about its own terminal, never a task's.
    watcher.announce_needs_you();
    Ok(())
}

/// `workspace.start_orchestrator`: open the workspace's orchestrator
/// (`Service::start_orchestrator`). Answers with the new terminal's id; the
/// caller reads it back, as `rpc.rs` does.
///
/// A new pane is a new window, so the worktree's layouts are published, as
/// `terminal.create` does, and the workspace's orchestrator changed, so the
/// fleet is announced.
pub async fn start_orchestrator(
    svc: &Service,
    watcher: &Watcher,
    workspace: Uuid,
    req: &pb::WorkspaceStartOrchestrator,
) -> Result<Uuid> {
    let handoff = Some(req.handoff_task.trim()).filter(|k| !k.is_empty());
    let terminal = svc.start_orchestrator(workspace, &req.harness, req.replace, handoff).await?;
    if let Ok(groups) = svc.layout(terminal.worktree_id).await {
        watcher.publish_layout(terminal.worktree_id, &groups);
    }
    watcher.announce_fleet_changed();
    Ok(terminal.id)
}

#[cfg(test)]
mod tests {
    use farcooler_protocol::v1::Scope;

    use super::*;
    use crate::test_support::fixture;

    /// Where a workspace's home and charter are is a path on the runner, and
    /// paths go to `host_admin` only. A paired phone at Control or Read sees
    /// the workspace and not where it lives.
    #[tokio::test]
    async fn a_workspaces_paths_are_for_host_admin_only() {
        let (_dir, svc, repo) = fixture().await;
        for scope in [Scope::Read, Scope::Control] {
            let listed = list(&svc, Some(repo), scope).unwrap();
            let main = &listed.items[0];
            assert!(main.is_main, "{main:?}");
            assert_eq!((main.home.as_deref(), main.charter_path.as_deref()), (None, None), "{scope:?}");
        }
        let listed = list(&svc, Some(repo), Scope::HostAdmin).unwrap();
        let main = &listed.items[0];
        assert!(main.home.is_some() && main.charter_path.is_some(), "{main:?}");
    }

    /// Deleting a workspace removes its orchestrator's settings file, and
    /// leaves its home, charter included, and every other workspace's file.
    #[tokio::test]
    async fn a_deleted_workspace_takes_its_orchestrators_settings_with_it() {
        let (_dir, svc, repo) = fixture().await;
        let watcher = crate::watch::Watcher::new(svc.clone());
        let req = pb::WorkspaceCreate { name: "Billing".into(), task_prefix: "bil".into() };
        let made = create(&svc, &watcher, repo, &req, Scope::HostAdmin).unwrap();
        let billing = Uuid::from_slice(&made.id).unwrap();
        let main = svc.store.main_workspace(repo).unwrap().id;
        let settings = |ws: Uuid| svc.root_dir().join(format!("orchestrator-{ws}.json"));

        for ws in [billing, main] {
            let term = svc.start_orchestrator(ws, "claude", false, None).await.expect("started");
            assert!(settings(ws).is_file(), "the launch wrote {}", settings(ws).display());
            svc.stop_terminal(term.id).await.unwrap();
            svc.remove_terminal(term.id).await.unwrap();
        }
        let charter = crate::workspace_home::charter_path(svc.root_dir(), billing);
        std::fs::write(&charter, "Billing's words").unwrap();

        delete(&svc, &watcher, billing).expect("deleted");
        assert!(!settings(billing).exists(), "left behind: {}", settings(billing).display());
        assert!(settings(main).is_file(), "another workspace's file stays");
        assert_eq!(std::fs::read_to_string(&charter).unwrap(), "Billing's words", "the home stays");
    }
}
