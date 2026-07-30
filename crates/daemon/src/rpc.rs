//! Method dispatch: the seam where a `Request` becomes domain work.
//!
//! The transport owns framing, the handshake, request correlation and
//! backpressure. This owns exactly three things and nothing else:
//!
//! 1. **Scope.** Every method declares the scope it needs, in one table, and
//!    the check happens before the payload is even read. A method that forgets
//!    to declare one does not exist.
//! 2. **Method to service call.** A thin mapping. Business rules live in
//!    `service`, so a second transport cannot acquire different behaviour.
//! 3. **Errors to wire codes.** Through `DomainError::wire()`, which is
//!    exhaustively matched, so an unmapped variant fails the build rather than
//!    reaching a phone as a generic failure at the moment the user needs to
//!    know what actually went wrong.

use std::sync::Arc;

use overnight_core::{DomainError, Result};
use overnight_protocol::v1::{
    Empty, Error as WireError, Request, Response, Result as WireResult, Scope, request, response,
    result,
};
use overnight_transport::Handler;
use uuid::Uuid;

use crate::service::Service;
use crate::wire;

pub struct Rpc {
    service: Arc<Service>,
    watcher: Arc<crate::watch::Watcher>,
    scope: Scope,
    daemon_version: String,
}

impl Rpc {
    pub fn new(
        service: Arc<Service>,
        watcher: Arc<crate::watch::Watcher>,
        scope: Scope,
    ) -> Self {
        Self { service, watcher, scope, daemon_version: env!("CARGO_PKG_VERSION").to_string() }
    }
}

/// The scope each method requires.
///
/// Exhaustive by construction: an unknown method is rejected rather than
/// defaulted, so adding a handler arm without adding a row here makes the
/// method unreachable instead of silently unguarded.
fn required_scope(method: &str) -> Option<Scope> {
    Some(match method {
        "host.get" | "host.health" | "daemon.version" => Scope::Read,
        "repository.list" | "workspace.list" | "terminal.list" | "branch.list" => Scope::Read,
        "layout.list" => Scope::Read,
        // Discovery reveals paths, which live behind the same gate as every
        // other path in this protocol.
        "worktree.list" => Scope::HostAdmin,
        "repository.register"
        | "workspace.create"
        | "workspace.archive"
        | "workspace.restore"
        | "terminal.create"
        | "terminal.resize"
        | "terminal.stop"
        | "terminal.dismiss_lost"
        | "terminal.restart"
        | "terminal.seen"
        | "terminal.remove" => Scope::Control,
        // Tiling is `control`, not `host_admin`. It touches no files and stops
        // no process — the worst a wrong one does is show you the wrong pane —
        // and it has to be reachable by an agent for any of this to be
        // automatable.
        "layout.tile"
        | "layout.add"
        | "layout.drop"
        | "layout.preset"
        | "layout.cycle"
        | "layout.focus"
        | "layout.zoom"
        | "layout.swap"
        | "layout.shift"
        | "layout.group.new"
        | "layout.group.select"
        | "layout.group.close" => Scope::Control,
        "repository_root.list"
        | "repository_root.add"
        | "repository_root.remove"
        | "workspace.remove_worktree"
        // Importing touches no git data — it writes a record pointing at a
        // directory that already exists — but it needs a path to name, and
        // naming one is the admin part.
        | "workspace.import" => Scope::HostAdmin,
        _ => return None,
    })
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Unspecified => "none",
        Scope::Read => "read",
        Scope::Control => "control",
        Scope::HostAdmin => "host_admin",
    }
}

/// Scopes are ordered: `host_admin` can do anything `control` can.
fn satisfies(granted: Scope, required: Scope) -> bool {
    fn rank(s: Scope) -> u8 {
        match s {
            Scope::Unspecified => 0,
            Scope::Read => 1,
            Scope::Control => 2,
            Scope::HostAdmin => 3,
        }
    }
    rank(granted) >= rank(required)
}

impl Handler for Rpc {
    async fn handle(&self, req: Request) -> Response {
        let request_id = req.request_id.clone();
        let outcome = match required_scope(&req.method) {
            None => Err(DomainError::NotFound),
            Some(required) if !satisfies(self.scope, required) => Err(DomainError::ScopeDenied { needed: scope_name(required) }),
            Some(_) => self.dispatch(req).await,
        };

        match outcome {
            Ok(value) => Response {
                request_id,
                outcome: Some(response::Outcome::Result(WireResult { value: Some(value) })),
            },
            Err(err) => {
                let (code, retryable) = err.wire();
                Response {
                    request_id,
                    outcome: Some(response::Outcome::Error(WireError {
                        code: code as i32,
                        retryable,
                        // Redacted by construction: never a path, terminal
                        // byte, command, or session id.
                        message: err.redacted_message(),
                    })),
                }
            }
        }
    }
}

impl Rpc {
    /// The envelope's target, which every single-resource mutation needs.
    fn target(req: &Request) -> Result<Uuid> {
        req.target_resource_id
            .as_deref()
            .and_then(wire::parse_id)
            .ok_or(DomainError::NotFound)
    }

    async fn dispatch(&self, req: Request) -> Result<result::Value> {
        let svc = &self.service;
        let scope = self.scope;

        match req.method.as_str() {
            // ---- reads ----
            "host.get" | "host.health" => {
                // Refresh before answering: a client asking about health wants
                // the answer now, not the one cached at connect time.
                svc.inventory.refresh().await;
                Ok(result::Value::Host(wire::host(
                    &self.daemon_version,
                    svc.host_id,
                    &svc.inventory_snapshot(),
                    0,
                )))
            }

            "daemon.version" => Ok(result::Value::DaemonVersion(
                overnight_protocol::v1::DaemonVersion {
                    daemon_version: self.daemon_version.clone(),
                    protocol_versions: vec![overnight_protocol::PROTOCOL_VERSION],
                    capabilities: vec!["workspaces".into(), "terminals".into()],
                },
            )),

            "repository_root.list" => {
                let repositories = svc.list_repositories()?;
                let items = svc
                    .list_roots()?
                    .iter()
                    .map(|root| {
                        let count = repositories
                            .iter()
                            .filter(|r| r.repository_root_id == root.id)
                            .count() as u32;
                        wire::repository_root(root, count, scope)
                    })
                    .collect();
                Ok(result::Value::RepositoryRootList(
                    overnight_protocol::v1::RepositoryRootList { items },
                ))
            }

            "repository.list" => {
                let items =
                    svc.list_repositories()?.iter().map(|r| wire::repository(r, scope)).collect();
                Ok(result::Value::RepositoryList(overnight_protocol::v1::RepositoryList { items }))
            }

            "workspace.list" => {
                let items =
                    svc.fleet().await?.iter().map(|view| wire::workspace(view, scope)).collect();
                Ok(result::Value::WorkspaceList(overnight_protocol::v1::WorkspaceList { items }))
            }

            "terminal.list" => {
                // An absent target lists every terminal; a present one filters
                // to that workspace.
                let filter = req.target_resource_id.as_deref().and_then(wire::parse_id);
                let mut items = Vec::new();
                for view in svc.fleet().await? {
                    if filter.is_some_and(|id| id != view.workspace.id) {
                        continue;
                    }
                    for terminal in &view.terminals {
                        items.push(self.with_activity(terminal).await);
                    }
                }
                Ok(result::Value::TerminalList(overnight_protocol::v1::TerminalList { items }))
            }

            // ---- mutations ----
            "repository_root.add" => {
                let Some(request::Payload::RepositoryRootAdd(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let root = svc.add_root(std::path::Path::new(&p.absolute_path)).await?;
                Ok(result::Value::RepositoryRoot(wire::repository_root(&root, 0, scope)))
            }

            "repository.register" => {
                let Some(request::Payload::RepositoryRegister(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let repo = svc.register_repository(std::path::Path::new(&p.relative_path)).await?;
                Ok(result::Value::Repository(wire::repository(&repo, scope)))
            }

            "branch.list" => {
                let repository = Self::target(&req)?;
                let items = svc
                    .list_branches(repository)
                    .await?
                    .into_iter()
                    .map(|b| overnight_protocol::v1::Branch {
                        name: b.name,
                        local: b.local,
                        remote: b.remote,
                        checked_out: b.checked_out,
                        updated_at: Some(wire::timestamp(b.updated_at * 1000)),
                        subject: b.subject,
                    })
                    .collect();
                Ok(result::Value::BranchList(overnight_protocol::v1::BranchList { items }))
            }

            "worktree.list" => {
                let repository = Self::target(&req)?;
                let items = svc
                    .discover_worktrees(repository)
                    .await?
                    .into_iter()
                    .map(|w| {
                        let name = std::path::Path::new(&w.path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| w.head.clone());
                        overnight_protocol::v1::ExistingWorktree {
                            path: w.path.clone(),
                            branch: w.branch,
                            head: w.head,
                            suggested_name: name,
                            locked: w.locked,
                        }
                    })
                    .collect();
                Ok(result::Value::WorktreeList(overnight_protocol::v1::WorktreeList { items }))
            }

            "workspace.import" => {
                let repository = Self::target(&req)?;
                let Some(request::Payload::WorktreeImport(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let name = (!p.task_name.trim().is_empty()).then(|| p.task_name.trim());
                let ws = svc.import_worktree(repository, &p.path, name).await?;
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }

            "workspace.create" => {
                let repository = Self::target(&req)?;
                let Some(request::Payload::WorkspaceCreate(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let ws = if p.adopt_existing {
                    svc.adopt_branch(repository, &p.task_name, &p.branch).await?
                } else {
                    svc.create_workspace(repository, &p.task_name, &p.branch, &p.base_revision)
                        .await?
                };
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }

            "workspace.archive" => {
                let ws = svc.archive_workspace(Self::target(&req)?).await?;
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }

            "workspace.restore" => {
                let ws = svc.restore_workspace(Self::target(&req)?).await?;
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }

            "repository_root.remove" => {
                let id = Self::target(&req)?;
                // Removing a root revokes Overnight's permission to operate
                // under a whole directory tree, so it is confirmed by name for
                // the same reason deleting a worktree is.
                let Some(request::Payload::TypedConfirmation(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let root = svc
                    .list_roots()?
                    .into_iter()
                    .find(|r| r.id == id)
                    .ok_or(DomainError::NotFound)?;
                let expected = std::path::Path::new(&root.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| root.path.clone());
                if p.typed_confirmation.trim() != expected {
                    return Err(DomainError::ConfirmationRequired);
                }
                let removed = svc.remove_root(id).await?;
                Ok(result::Value::RepositoryRoot(wire::repository_root(&removed, 0, scope)))
            }

            "workspace.remove_worktree" => {
                let id = Self::target(&req)?;
                // The typed confirmation is checked HERE rather than in the
                // client, because a client that skips the dialog must still be
                // refused.
                let Some(request::Payload::TypedConfirmation(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let ws = svc
                    .list_workspaces()?
                    .into_iter()
                    .find(|w| w.id == id)
                    .ok_or(DomainError::NotFound)?;
                if p.typed_confirmation.trim() != ws.task_name {
                    return Err(DomainError::ConfirmationRequired);
                }
                svc.remove_worktree(id).await?;
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }

            "terminal.create" => {
                let workspace = Self::target(&req)?;
                let Some(request::Payload::TerminalCreate(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let term = svc.create_terminal(workspace, &p.title, &p.command_preset).await?;
                if p.join_active_group {
                    // A no-op when the workspace has no layout, which is what
                    // makes it safe to pass unconditionally from a `%` binding.
                    if let Ok(groups) = svc.layout_add(workspace, None, &[term.id]).await {
                        self.watcher.publish_layout(workspace, &groups);
                    }
                }
                self.terminal_result(term.id).await
            }

            "terminal.resize" => {
                let id = Self::target(&req)?;
                let Some(request::Payload::TerminalResize(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                svc.resize_terminal(id, p.columns, p.rows).await?;
                self.terminal_result(id).await
            }

            "terminal.stop" => {
                let id = Self::target(&req)?;
                svc.stop_terminal(id).await?;
                self.terminal_result(id).await
            }

            // Opening a terminal is what ends `Done`, which is defined as
            // idle-and-unseen. Deliberately its own method rather than a side
            // effect of listing: appearing in a list is not reading it, and
            // clearing a notification nobody read is worse than not sending one.
            "terminal.seen" => {
                let id = Self::target(&req)?;
                self.watcher.mark_seen(id).await;
                self.terminal_result(id).await
            }

            "terminal.remove" => {
                let id = Self::target(&req)?;
                svc.remove_terminal(id).await?;
                // No terminal to return: it is gone. An empty workspace list is
                // the honest shape for "this succeeded and there is nothing to
                // show", rather than echoing back a record that no longer
                // exists.
                Ok(result::Value::TerminalList(overnight_protocol::v1::TerminalList {
                    items: Vec::new(),
                }))
            }

            "terminal.dismiss_lost" => {
                let id = Self::target(&req)?;
                svc.dismiss_lost(id).await?;
                self.terminal_result(id).await
            }

            "terminal.restart" => {
                let id = Self::target(&req)?;
                svc.restart_terminal(id).await?;
                self.terminal_result(id).await
            }

            // ---- tiling ----
            //
            // The workspace is always the envelope target and the group is
            // always in the payload, so every one of these reads the same two
            // things and differs only in what it does with them.
            "layout.list" => {
                let workspace = Self::target(&req)?;
                Ok(result::Value::PaneGroupList(wire::pane_group_list(
                    workspace,
                    &svc.layout(workspace)?,
                )))
            }

            method if method.starts_with("layout.") => {
                let workspace = Self::target(&req)?;
                let p = match req.payload {
                    Some(request::Payload::LayoutUpdate(p)) => p,
                    // An empty payload is legal for the verbs that need no
                    // arguments: cycle, zoom-toggle, group-close.
                    Some(request::Payload::Empty(_)) | None => Default::default(),
                    _ => return Err(DomainError::InvalidArgument { what: "payload" }),
                };
                let group = p.group_id.as_deref().and_then(wire::parse_id);
                let terminals: Vec<Uuid> =
                    p.terminals.iter().filter_map(|t| wire::parse_id(t)).collect();
                let preset = p.preset.and_then(|raw| {
                    overnight_protocol::v1::LayoutPreset::try_from(raw).ok()
                });

                let step = p.step.unwrap_or(1) as i64;

                let groups = match method {
                    "layout.tile" => {
                        svc.layout_tile(workspace, group, &terminals, preset).await?
                    }
                    "layout.add" => svc.layout_add(workspace, group, &terminals).await?,
                    "layout.drop" => svc.layout_drop(workspace, &terminals).await?,
                    "layout.preset" => {
                        let name = (!p.name.is_empty()).then_some(p.name.as_str());
                        svc.layout_configure(workspace, group, preset, p.ratio, name).await?
                    }
                    "layout.cycle" => svc.layout_cycle(workspace, group).await?,
                    "layout.focus" => match (
                        p.focus.as_deref().and_then(wire::parse_id),
                        p.pane,
                    ) {
                        (Some(terminal), _) => svc.layout_focus(workspace, terminal).await?,
                        (None, Some(index)) => {
                            svc.layout_focus_index(workspace, group, index as usize).await?
                        }
                        (None, None) => svc.layout_focus_step(workspace, group, step).await?,
                    },
                    "layout.zoom" => {
                        let terminal = p.zoom.as_deref().and_then(wire::parse_id);
                        svc.layout_zoom(workspace, group, terminal, p.unzoom).await?
                    }
                    "layout.swap" => {
                        let [a, b] = terminals.as_slice() else {
                            return Err(DomainError::InvalidArgument { what: "two terminals" });
                        };
                        svc.layout_swap(workspace, *a, *b).await?
                    }
                    "layout.shift" => svc.layout_shift(workspace, group, step).await?,
                    "layout.group.new" => svc.layout_group_new(workspace, &p.name).await?,
                    "layout.group.select" => match group {
                        Some(id) => svc.layout_group_select(workspace, id).await?,
                        None => svc.layout_group_step(workspace, step).await?,
                    },
                    "layout.group.close" => svc.layout_group_close(workspace, group).await?,
                    other => {
                        tracing::error!(method = %other, "layout method has no handler");
                        return Err(DomainError::NotFound);
                    }
                };

                // Announced from here rather than from each service method: the
                // service is also called by tests and by the local CLI path,
                // neither of which has a client to tell.
                self.watcher.publish_layout(workspace, &groups);
                Ok(result::Value::PaneGroupList(wire::pane_group_list(workspace, &groups)))
            }

            // `required_scope` already rejected anything not listed there, so
            // reaching here means the two lists disagree.
            other => {
                tracing::error!(method = %other, "method passed the scope table but has no handler");
                Err(DomainError::NotFound)
            }
        }
    }

    /// Re-read a terminal so the reply carries its DERIVED state rather than
    /// the intent that was just written.
    async fn terminal_result(&self, id: Uuid) -> Result<result::Value> {
        for view in self.service.fleet().await? {
            if let Some(t) = view.terminals.iter().find(|t| t.terminal.id == id) {
                return Ok(result::Value::Terminal(self.with_activity(t).await));
            }
        }
        Err(DomainError::NotFound)
    }

    /// Attach what the watcher decided the agent is doing.
    ///
    /// One place, so a terminal in a list and the same terminal in a mutation
    /// reply cannot disagree about whether its agent is waiting for you.
    async fn with_activity(
        &self,
        view: &crate::service::TerminalView,
    ) -> overnight_protocol::v1::Terminal {
        let mut message = wire::terminal(view);
        let (activity, changed_at) = self.watcher.activity(view.terminal.id).await;
        message.activity = activity as i32;
        message.activity_changed_at = changed_at.map(wire::timestamp);
        if let Some(command) = self.watcher.command(view.terminal.id).await {
            message.current_command = command;
        }
        message
    }
}

/// A request with no payload, for the read methods.
pub fn empty_payload() -> Option<request::Payload> {
    Some(request::Payload::Empty(Empty {}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_is_ordered_so_admin_can_do_everything() {
        assert!(satisfies(Scope::HostAdmin, Scope::Read));
        assert!(satisfies(Scope::HostAdmin, Scope::Control));
        assert!(satisfies(Scope::Control, Scope::Read));
        assert!(satisfies(Scope::Read, Scope::Read));
    }

    #[test]
    fn a_lower_scope_cannot_reach_a_higher_method() {
        assert!(!satisfies(Scope::Read, Scope::Control));
        assert!(!satisfies(Scope::Control, Scope::HostAdmin));
        // An unnegotiated scope reaches nothing at all.
        assert!(!satisfies(Scope::Unspecified, Scope::Read));
    }

    #[test]
    fn every_method_the_design_lists_for_mvp_declares_a_scope() {
        // The table is the guard, so an unlisted method must be unreachable
        // rather than reachable-and-unguarded.
        for method in [
            "host.get",
            "daemon.version",
            "repository_root.list",
            "repository.list",
            "workspace.list",
            "terminal.list",
            "branch.list",
            "worktree.list",
            "workspace.import",
            "repository_root.add",
            "repository.register",
            "workspace.create",
            "workspace.archive",
            "workspace.restore",
            "terminal.seen",
            "terminal.remove",
            "repository_root.remove",
            "workspace.remove_worktree",
            "terminal.create",
            "terminal.resize",
            "terminal.stop",
            "terminal.dismiss_lost",
            "terminal.restart",
            "layout.list",
            "layout.tile",
            "layout.add",
            "layout.drop",
            "layout.preset",
            "layout.cycle",
            "layout.focus",
            "layout.zoom",
            "layout.swap",
            "layout.shift",
            "layout.group.new",
            "layout.group.select",
            "layout.group.close",
        ] {
            assert!(required_scope(method).is_some(), "{method} has no declared scope");
        }
    }

    #[test]
    fn an_unknown_method_is_refused_rather_than_defaulted() {
        assert_eq!(required_scope("terminal.write"), None);
        // The `layout.` handler arm is prefix-matched, so an unlisted layout
        // method must still be stopped by the table before it gets there.
        assert_eq!(required_scope("layout.nonsense"), None);
        assert_eq!(required_scope(""), None);
        assert_eq!(required_scope("host.get "), None, "no fuzzy matching");
    }

    #[test]
    fn the_dangerous_methods_require_host_admin() {
        // Adding a repository root grants access to a directory tree, and
        // removing a worktree deletes files. Neither is a `control` action.
        assert_eq!(required_scope("repository_root.add"), Some(Scope::HostAdmin));
        assert_eq!(required_scope("workspace.remove_worktree"), Some(Scope::HostAdmin));
        assert_eq!(required_scope("repository_root.remove"), Some(Scope::HostAdmin));
        // Paths live behind the same gate, so listing roots is admin too.
        assert_eq!(required_scope("repository_root.list"), Some(Scope::HostAdmin));
    }

    #[test]
    fn tiling_is_control_not_admin() {
        // An agent has to be able to place its own panes, and none of this
        // touches a file or stops a process.
        for method in ["layout.tile", "layout.zoom", "layout.group.new"] {
            assert_eq!(required_scope(method), Some(Scope::Control), "{method}");
        }
        assert_eq!(required_scope("layout.list"), Some(Scope::Read));
    }

    #[test]
    fn reads_never_require_more_than_read() {
        for method in ["host.get", "daemon.version", "workspace.list", "terminal.list"] {
            assert_eq!(required_scope(method), Some(Scope::Read), "{method}");
        }
    }
}
