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
    scope: Scope,
    daemon_version: String,
}

impl Rpc {
    pub fn new(service: Arc<Service>, scope: Scope) -> Self {
        Self { service, scope, daemon_version: env!("CARGO_PKG_VERSION").to_string() }
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
        "repository.list" | "workspace.list" | "terminal.list" => Scope::Read,
        "repository.register"
        | "workspace.create"
        | "workspace.archive"
        | "terminal.create"
        | "terminal.resize"
        | "terminal.stop"
        | "terminal.dismiss_lost"
        | "terminal.restart" => Scope::Control,
        "repository_root.list" | "repository_root.add" | "workspace.remove_worktree" => {
            Scope::HostAdmin
        }
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
            "host.get" | "host.health" => Ok(result::Value::Host(wire::host(
                &self.daemon_version,
                svc.host_id,
                0,
            ))),

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
                    items.extend(view.terminals.iter().map(wire::terminal));
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

            "workspace.create" => {
                let repository = Self::target(&req)?;
                let Some(request::Payload::WorkspaceCreate(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let ws = svc
                    .create_workspace(repository, &p.task_name, &p.branch, &p.base_revision)
                    .await?;
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }

            "workspace.archive" => {
                let ws = svc.archive_workspace(Self::target(&req)?).await?;
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
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
                return Ok(result::Value::Terminal(wire::terminal(t)));
            }
        }
        Err(DomainError::NotFound)
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
            "repository_root.add",
            "repository.register",
            "workspace.create",
            "workspace.archive",
            "workspace.remove_worktree",
            "terminal.create",
            "terminal.resize",
            "terminal.stop",
            "terminal.dismiss_lost",
            "terminal.restart",
        ] {
            assert!(required_scope(method).is_some(), "{method} has no declared scope");
        }
    }

    #[test]
    fn an_unknown_method_is_refused_rather_than_defaulted() {
        assert_eq!(required_scope("terminal.write"), None);
        assert_eq!(required_scope(""), None);
        assert_eq!(required_scope("host.get "), None, "no fuzzy matching");
    }

    #[test]
    fn the_dangerous_methods_require_host_admin() {
        // Adding a repository root grants access to a directory tree, and
        // removing a worktree deletes files. Neither is a `control` action.
        assert_eq!(required_scope("repository_root.add"), Some(Scope::HostAdmin));
        assert_eq!(required_scope("workspace.remove_worktree"), Some(Scope::HostAdmin));
        // Paths live behind the same gate, so listing roots is admin too.
        assert_eq!(required_scope("repository_root.list"), Some(Scope::HostAdmin));
    }

    #[test]
    fn reads_never_require_more_than_read() {
        for method in ["host.get", "daemon.version", "workspace.list", "terminal.list"] {
            assert_eq!(required_scope(method), Some(Scope::Read), "{method}");
        }
    }
}
