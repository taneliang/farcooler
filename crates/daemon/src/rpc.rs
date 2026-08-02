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

use overnight_agent::link::DaemonMessage;
use overnight_core::{DomainError, Result};
use overnight_protocol::v1::{
    Empty, Error as WireError, Request, Response, Result as WireResult, Scope, request, response,
    result,
};
use overnight_store::models;
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
        Self { service, watcher, scope, daemon_version: overnight_protocol::BUILD.to_string() }
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
        | "terminal.remove"
        // Reading a screen is `control`, not `read`.
        //
        // A screen is the most sensitive thing this protocol carries — it is
        // whatever the agent has on it, which routinely includes source, paths
        // and tokens — and `read` is the scope handed to something that should
        // only see the shape of the fleet.
        | "terminal.screen"
        | "terminal.write" => Scope::Control,
        // A pane's agent channel is exactly as sensitive as its screen — it is
        // the same conversation, just structured — so it sits at the same
        // scope rather than behind `host_admin`. Search returns
        // worktree-relative paths only, never a host path, so it belongs here
        // too rather than beside `worktree.list`.
        "terminal.set_pane_mode"
        | "terminal.agent_subscribe"
        | "terminal.agent_prompt"
        | "terminal.agent_answer"
        | "terminal.agent_set_mode" | "terminal.agent_set_model" | "terminal.agent_set_config"
        | "terminal.agent_cancel"
        | "worktree.file_search" => Scope::Control,
        // Tiling is `control`, not `host_admin`. It touches no files and stops
        // no process — the worst a wrong one does is show you the wrong pane —
        // and it has to be reachable by an agent for any of this to be
        // automatable.
        "layout.split"
        | "layout.move"
        | "layout.resize"
        | "layout.break"
        | "layout.rename"
        | "layout.viewport"
        | "layout.preset"
        | "layout.cycle"
        | "layout.focus"
        | "layout.zoom"
        | "layout.swap"
        | "layout.group.select" => Scope::Control,
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
                // Joining the active layout is a SPLIT of the focused pane,
                // not a new window.
                //
                // This used to call `layout_add`, which went away when the
                // layout model moved into tmux — a window IS a layout and a
                // pane IS a terminal. The flag survived in the proto and in
                // the CLI's `--tile`, but nothing honoured it any more, so
                // every new terminal opened outside the layout it was asked to
                // join.
                //
                // A no-op when there is no layout to join, which is what makes
                // it safe to pass unconditionally from a `%` binding.
                if p.join_active_group {
                    let anchor = svc.layout(workspace).await.ok().and_then(|views| {
                        let view = views.iter().find(|v| v.window.active).or(views.first())?;
                        let pane =
                            view.panes.iter().find(|pane| pane.pane_active).or(view.panes.first())?;
                        Some(pane.terminal_id)
                    });
                    if let Some(anchor) = anchor {
                        let term = svc
                            .split_terminal(
                                workspace,
                                anchor,
                                overnight_protocol::v1::SplitSide::Right,
                                &p.title,
                                &p.command_preset,
                            )
                            .await?;
                        return self.terminal_result(term.id).await;
                    }
                }
                let term = svc.create_terminal(workspace, &p.title, &p.command_preset).await?;
                // A new terminal is a new tmux window, which IS a new layout —
                // so the workspace's set of layouts just changed and every
                // watcher has to be told.
                //
                // Clients read layouts once at startup and rely on events for
                // everything after, so without this the tab simply does not
                // appear. It shows up minutes later when some unrelated action
                // happens to refresh, which reads as the pane arriving nowhere
                // and then teleporting into a tab.
                if let Ok(groups) = svc.layout(workspace).await {
                    self.watcher.publish_layout(workspace, &groups);
                }
                self.terminal_result(term.id).await
            }

            // A screen, for clients that cannot read tmux themselves.
            //
            // The CLI and the Mac app go straight to tmux because they are on the
            // host; a phone over ssh cannot, so without this it can list
            // terminals and act on them but never show one.
            "terminal.screen" => {
                let id = Self::target(&req)?;
                let known = match req.payload {
                    Some(request::Payload::TerminalScreenRequest(p)) => p.known_revision,
                    _ => 0,
                };

                let (contents, columns, rows) = svc.screen(id).await?;
                let (cursor_column, cursor_row) = svc.cursor(id).await.unwrap_or((0, 0));
                // Sent with every screen, including an unchanged one: a client
                // that rebuilt its emulator needs these even when the contents
                // it already holds are still current.
                let modes = svc.pane_modes(id).await.unwrap_or_default();

                // The cursor is part of the identity, not just the contents: a
                // caret moving along a line changes nothing else on screen, and a
                // client told "unchanged" would draw it in the old cell.
                let revision = screen_revision(&contents, cursor_column, cursor_row);
                if known != 0 && known == revision {
                    return Ok(result::Value::TerminalScreen(
                        overnight_protocol::v1::TerminalScreen {
                            contents: bytes::Bytes::new(),
                            columns,
                            rows,
                            cursor_column,
                            cursor_row,
                            revision,
                            unchanged: true,
                            modes: modes.clone(),
                        },
                    ));
                }

                Ok(result::Value::TerminalScreen(overnight_protocol::v1::TerminalScreen {
                    contents: bytes::Bytes::from(contents.into_bytes()),
                    columns,
                    rows,
                    cursor_column,
                    cursor_row,
                    revision,
                    unchanged: false,
                    modes,
                }))
            }

            "terminal.write" => {
                let id = Self::target(&req)?;
                let Some(request::Payload::TerminalWrite(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                svc.send_bytes(id, &p.payload).await?;
                self.terminal_result(id).await
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

            // ---- agent channel ----
            //
            // Every payload here names its own `terminal_id` rather than
            // relying on the envelope's `target_resource_id`. The envelope
            // convention is for a mutation of an existing versioned resource;
            // `AgentSubscribe` in particular legitimately targets a terminal
            // that holds no session yet, which is not that shape.
            "terminal.set_pane_mode" => {
                let Some(request::Payload::SetPaneMode(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                let mode = match overnight_protocol::v1::PaneMode::try_from(p.pane_mode) {
                    Ok(overnight_protocol::v1::PaneMode::Agent) => models::PaneMode::Agent,
                    Ok(overnight_protocol::v1::PaneMode::Terminal) => models::PaneMode::Terminal,
                    // A client that sends UNSPECIFIED is asking for a mode
                    // that does not exist, not for a default — guessing one
                    // would silently switch a pane nobody asked to switch.
                    _ => return Err(DomainError::InvalidArgument { what: "pane_mode" }),
                };
                svc.set_pane_mode(id, mode, p.force).await?;
                self.terminal_result(id).await
            }

            "terminal.agent_subscribe" => {
                let Some(request::Payload::AgentSubscribe(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                // Accepted even with no session: a client attaches to a PANE,
                // not to a session, and an empty batch is the honest answer
                // for one that has not run an agent yet.
                let (epoch, events) = svc.agents().replay(id, p.from_seq, p.epoch);
                Ok(result::Value::AgentEventBatch(wire::agent_batch(id, events, epoch)))
            }

            // These four send to the shim and reply with the terminal read
            // back, the same shape every other terminal mutation replies
            // with — `Result` has no empty variant, and re-reading also means
            // a client sees the activity its own send just caused (a prompt
            // moves the row to `Working`) without a second round trip.
            "terminal.agent_prompt" => {
                let Some(request::Payload::AgentPrompt(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                svc.agents().send(id, DaemonMessage::Prompt { text: wire::prompt_text(&p.blocks) });
                self.terminal_result(id).await
            }

            "terminal.agent_answer" => {
                let Some(request::Payload::AgentAnswer(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                svc.agents().send(
                    id,
                    DaemonMessage::Answer { request_id: p.request_id, option_id: p.option_id },
                );
                // The same call `terminal.seen` makes: answering is only
                // reachable by having looked, so it ends `Done` the same way.
                svc.agents().seen(id);
                self.terminal_result(id).await
            }

            "terminal.agent_set_mode" => {
                let Some(request::Payload::AgentSetMode(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                svc.agents().send(id, DaemonMessage::SetMode { agent_mode: p.agent_mode });
                self.terminal_result(id).await
            }

            "terminal.agent_set_model" => {
                let Some(request::Payload::AgentSetModel(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                svc.agents().send(id, DaemonMessage::SetModel { model: p.model });
                self.terminal_result(id).await
            }

            "terminal.agent_set_config" => {
                let Some(request::Payload::AgentSetConfig(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                svc.agents().send(id, DaemonMessage::SetConfig { id: p.config_id, value: p.value });
                self.terminal_result(id).await
            }

            "terminal.agent_edit_queued" => {
                let Some(request::Payload::AgentEditQueued(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                svc.agents()
                    .send(id, DaemonMessage::EditQueued { id: p.queued_id, text: p.text });
                self.terminal_result(id).await
            }

            "terminal.agent_cancel_queued" => {
                let Some(request::Payload::AgentCancelQueued(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                svc.agents().send(id, DaemonMessage::CancelQueued { id: p.queued_id });
                self.terminal_result(id).await
            }

            "terminal.agent_steer_queued" => {
                let Some(request::Payload::AgentSteerQueued(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                svc.agents().send(id, DaemonMessage::SteerQueued { id: p.queued_id });
                self.terminal_result(id).await
            }

            "terminal.agent_cancel" => {
                let Some(request::Payload::AgentCancel(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                svc.agents().send(id, DaemonMessage::Cancel);
                self.terminal_result(id).await
            }

            "worktree.file_search" => {
                let Some(request::Payload::WorktreeFileSearch(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.workspace_id).ok_or(DomainError::NotFound)?;
                let paths = svc.search_worktree_files(id, &p.query, p.limit).await?;
                Ok(result::Value::WorktreeFileList(overnight_protocol::v1::WorktreeFileList {
                    paths,
                }))
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
                    &svc.layout(workspace).await?,
                )))
            }

            method if method.starts_with("layout.") => {
                let workspace = Self::target(&req)?;
                let p = match req.payload {
                    Some(request::Payload::LayoutUpdate(p)) => p,
                    // Legal for the verbs that need no arguments.
                    Some(request::Payload::Empty(_)) | None => Default::default(),
                    _ => return Err(DomainError::InvalidArgument { what: "payload" }),
                };
                let group = (!p.group_id.is_empty()).then_some(p.group_id.as_str());
                let step = p.step.unwrap_or(1) as i64;
                let side = p.side();
                let terminals: Vec<Uuid> =
                    p.terminals.iter().filter_map(|t| wire::parse_id(t)).collect();
                let target = p.target.as_deref().and_then(wire::parse_id);

                let groups = match method {
                    // A new pane beside an existing one: `%`, `"`, and a drop on
                    // an edge. The only layout verb that creates a terminal.
                    "layout.split" => {
                        let preset = if p.command_preset.is_empty() {
                            "shell"
                        } else {
                            p.command_preset.as_str()
                        };
                        let anchor = match target {
                            Some(id) => id,
                            None => svc
                                .active_layout(workspace)
                                .await?
                                .and_then(|l| l.focused().map(|f| f.terminal_id))
                                .ok_or(DomainError::NotFound)?,
                        };
                        let title = if p.name.is_empty() { preset } else { p.name.as_str() };
                        svc.split_terminal(workspace, anchor, side, title, preset).await?;
                        svc.layout(workspace).await?
                    }
                    // An existing pane moved against another, on an edge. The
                    // drag half of drag and drop, and it works across layouts.
                    "layout.move" => {
                        let [dragged] = terminals.as_slice() else {
                            return Err(DomainError::InvalidArgument { what: "one terminal" });
                        };
                        let onto = target.ok_or(DomainError::InvalidArgument { what: "target" })?;
                        svc.layout_move(workspace, *dragged, onto, side).await?
                    }
                    "layout.preset" => {
                        let preset = p
                            .preset
                            .and_then(|raw| overnight_protocol::v1::LayoutPreset::try_from(raw).ok())
                            .unwrap_or(overnight_protocol::v1::LayoutPreset::Tiled);
                        svc.layout_preset(workspace, group, preset).await?
                    }
                    "layout.cycle" => svc.layout_cycle(workspace, group).await?,
                    "layout.focus" => match (p.focus.as_deref().and_then(wire::parse_id), p.pane) {
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
                    "layout.resize" => {
                        let terminal = target.ok_or(DomainError::InvalidArgument {
                            what: "target",
                        })?;
                        svc.layout_resize(workspace, terminal, side, p.resize.unwrap_or(2)).await?
                    }
                    // Out into a layout of its own, tmux's break-pane.
                    "layout.break" => {
                        let terminal = match target.or(terminals.first().copied()) {
                            Some(id) => id,
                            None => svc
                                .active_layout(workspace)
                                .await?
                                .and_then(|l| l.focused().map(|f| f.terminal_id))
                                .ok_or(DomainError::NotFound)?,
                        };
                        svc.layout_break(workspace, terminal).await?
                    }
                    "layout.rename" => svc.layout_rename(workspace, group, &p.name).await?,
                    "layout.group.select" => match group {
                        Some(id) => svc.layout_group_select(workspace, id).await?,
                        None => svc.layout_group_step(workspace, step).await?,
                    },
                    // The viewport, so tmux lays out for the size actually on
                    // screen rather than for whatever the window last had.
                    "layout.viewport" => {
                        svc.layout_resize_window(
                            workspace,
                            group,
                            p.columns.unwrap_or(0),
                            p.rows.unwrap_or(0),
                        )
                        .await?
                    }
                    other => {
                        tracing::error!(method = %other, "layout method has no handler");
                        return Err(DomainError::NotFound);
                    }
                };

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
        let mut message = wire::terminal_with_agent_state(view, self.service.agents());
        let (activity, changed_at) = self.watcher.activity(view.terminal.id).await;
        message.activity = activity as i32;
        message.activity_changed_at = changed_at.map(wire::timestamp);
        if let Some(command) = self.watcher.command(view.terminal.id).await {
            message.current_command = command;
        }
        message.chat_capable = self.watcher.chat_capable(view.terminal.id).await;
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
            "terminal.screen",
            "terminal.write",
            "layout.list",
            "layout.split",
            "layout.move",
            "layout.resize",
            "layout.break",
            "layout.rename",
            "layout.viewport",
            "layout.preset",
            "layout.cycle",
            "layout.focus",
            "layout.zoom",
            "layout.swap",
            "layout.group.select",
        ] {
            assert!(required_scope(method).is_some(), "{method} has no declared scope");
        }
    }

    #[test]
    fn an_unknown_method_is_refused_rather_than_defaulted() {
        // Deliberately a name nothing will ever take: this used to be
        // `terminal.write`, which became real, and a test asserting a method does
        // not exist has to name one that cannot.
        assert_eq!(required_scope("terminal.telepathy"), None);
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
        for method in ["layout.split", "layout.zoom", "layout.move"] {
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


/// A cheap identity for a screen.
///
/// FNV-1a over the capture and the cursor. Not a checksum anyone relies on for
/// correctness — a collision means one stale frame until the next change, which
/// is a redraw, not corruption — and it is compared only against a value this
/// same host produced moments earlier.
fn screen_revision(contents: &str, cursor_column: u32, cursor_row: u32) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
    };
    eat(contents.as_bytes());
    eat(&cursor_column.to_le_bytes());
    eat(&cursor_row.to_le_bytes());
    // Zero means "I have nothing" on the wire, so it must never be a real value.
    if hash == 0 { 1 } else { hash }
}

#[cfg(test)]
mod revision_tests {
    use super::screen_revision;

    #[test]
    fn the_same_screen_has_the_same_revision() {
        assert_eq!(screen_revision("hello", 1, 2), screen_revision("hello", 1, 2));
    }

    #[test]
    fn a_moved_cursor_is_a_different_screen() {
        // Nothing else changed, and a client told "unchanged" would leave the
        // caret in the wrong cell.
        assert_ne!(screen_revision("hello", 1, 2), screen_revision("hello", 2, 2));
        assert_ne!(screen_revision("hello", 1, 2), screen_revision("hello", 1, 3));
    }

    #[test]
    fn different_contents_differ() {
        assert_ne!(screen_revision("hello", 0, 0), screen_revision("hellp", 0, 0));
    }

    #[test]
    fn zero_is_never_a_real_revision() {
        // The wire uses it to mean "I hold nothing", so a screen that hashed to
        // it would be resent forever.
        for text in ["", "a", "the quick brown fox"] {
            assert_ne!(screen_revision(text, 0, 0), 0);
        }
    }
}
