//! Turning domain models into protocol messages.
//!
//! Two rules hold everywhere in this file.
//!
//! **Paths are `host_admin` only.** Every path-bearing field is optional in the
//! proto and is populated only for a client that holds `host_admin`. Ordinary
//! clients get an opaque token instead. A phone on someone else's network has
//! no business learning the directory layout of the machine it is driving, and
//! making the redaction a property of the conversion — rather than something
//! each handler remembers — is what stops one forgotten call site leaking it.
//!
//! **Terminal and workspace `state` is derived, never read from storage.** The
//! converters take a derived view rather than a stored row, so there is no way
//! to write a handler that reports a stale `running` for a pane that died an
//! hour ago.

use overnight_protocol::v1::{self as wire, Scope};
use overnight_store::models;
use uuid::Uuid;

use crate::service::{TerminalView, WorkspaceView};

pub fn id_bytes(id: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(id.as_bytes())
}

pub fn parse_id(bytes: &[u8]) -> Option<Uuid> {
    Uuid::from_slice(bytes).ok()
}

fn admin(scope: Scope) -> bool {
    scope == Scope::HostAdmin
}

/// A stable, opaque stand-in for a path.
///
/// Derived from the resource's own id rather than from the path text: a hash of
/// the path would still be a fixed function of the secret we are withholding,
/// and a client that guesses a candidate path could confirm it. The id reveals
/// nothing and is equally stable.
fn path_token(id: Uuid) -> String {
    let hex = id.simple().to_string();
    format!("p-{}", &hex[hex.len() - 12..])
}

pub fn host(daemon_version: &str, host_id: Uuid, replay_bytes: u64) -> wire::Host {
    wire::Host {
        id: id_bytes(host_id),
        resource_version: 1,
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        daemon_version: daemon_version.to_string(),
        protocol_version: overnight_protocol::PROTOCOL_VERSION,
        self_health: wire::SelfHealth::Healthy as i32,
        self_health_reasons: Vec::new(),
        replay_bytes_retained: replay_bytes,
    }
}

pub fn repository_root(
    model: &models::RepositoryRoot,
    repository_count: u32,
    scope: Scope,
) -> wire::RepositoryRoot {
    wire::RepositoryRoot {
        id: id_bytes(model.id),
        resource_version: model.resource_version,
        host_id: id_bytes(model.host_id),
        path_token: path_token(model.id),
        display_path: admin(scope).then(|| model.path.clone()),
        created_at: Some(timestamp(model.created_at)),
        repository_count,
    }
}

pub fn repository(model: &models::Repository, scope: Scope) -> wire::Repository {
    wire::Repository {
        id: id_bytes(model.id),
        resource_version: model.resource_version,
        host_id: id_bytes(model.host_id),
        repository_root_id: id_bytes(model.repository_root_id),
        display_name: model.display_name.clone(),
        canonical_git_dir: admin(scope).then(|| model.canonical_git_dir.clone()),
        remote_summary: model.remote_summary.clone(),
    }
}

/// A workspace with its DERIVED state.
///
/// Taking a view rather than a row is the point: `models::Workspace` has no
/// state field to accidentally report.
pub fn workspace(view: &WorkspaceView, scope: Scope) -> wire::Workspace {
    let ws = &view.workspace;
    wire::Workspace {
        id: id_bytes(ws.id),
        resource_version: ws.resource_version,
        repository_id: id_bytes(ws.repository_id),
        task_name: ws.task_name.clone(),
        branch: ws.branch.clone(),
        worktree_path_token: path_token(ws.id),
        worktree_path: admin(scope).then(|| ws.worktree_path.clone()),
        state: view.state as i32,
    }
}

/// A terminal with its DERIVED state.
pub fn terminal(view: &TerminalView) -> wire::Terminal {
    let t = &view.terminal;
    let exit_status = (t.exit_code.is_some() || t.exit_signal.is_some()).then(|| wire::ExitStatus {
        code: t.exit_code,
        signal: t.exit_signal,
    });

    wire::Terminal {
        id: id_bytes(t.id),
        resource_version: t.resource_version,
        lease_generation: t.lease_generation,
        workspace_id: id_bytes(t.workspace_id),
        title: t.title.clone(),
        command_preset: t.command_preset.clone(),
        intent: t.intent as i32,
        state: view.state() as i32,
        exit_status,
        // Writer leases are not yet enforced, so there is no holder to report.
        // Reporting a fabricated one would be worse than reporting none.
        writer_client_id: None,
        columns: t.columns,
        rows: t.rows,
        size_controller_client_id: None,
        epoch: t.epoch,
    }
}

fn timestamp(unix_millis: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: unix_millis.div_euclid(1000),
        nanos: (unix_millis.rem_euclid(1000) * 1_000_000) as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> models::RepositoryRoot {
        models::RepositoryRoot {
            id: Uuid::now_v7(),
            host_id: Uuid::now_v7(),
            path: "/Users/someone/secret-project".into(),
            created_at: 1_700_000_000_123,
            resource_version: 4,
        }
    }

    #[test]
    fn a_read_client_never_learns_a_path() {
        for scope in [Scope::Read, Scope::Control] {
            let w = repository_root(&root(), 2, scope);
            assert_eq!(w.display_path, None, "{scope:?} must not receive a path");
            assert!(!w.path_token.is_empty(), "but it still needs a stable handle");
        }
    }

    #[test]
    fn host_admin_gets_the_path() {
        let model = root();
        let w = repository_root(&model, 2, Scope::HostAdmin);
        assert_eq!(w.display_path.as_deref(), Some(model.path.as_str()));
    }

    #[test]
    fn a_path_token_reveals_nothing_about_the_path() {
        // Two roots at paths sharing every component still get unrelated
        // tokens, because the token is a function of the id and not the text.
        let mut a = root();
        let mut b = root();
        a.path = "/Users/someone/project".into();
        b.path = "/Users/someone/project".into();
        let ta = repository_root(&a, 0, Scope::Read).path_token;
        let tb = repository_root(&b, 0, Scope::Read).path_token;
        assert_ne!(ta, tb);
        assert!(!ta.contains("someone") && !ta.contains("project"));
    }

    #[test]
    fn a_path_token_is_stable_for_the_same_resource() {
        // The client uses it as an identity across polls, so it must not move.
        let model = root();
        assert_eq!(
            repository_root(&model, 0, Scope::Read).path_token,
            repository_root(&model, 9, Scope::Read).path_token
        );
    }

    #[test]
    fn a_repository_git_dir_is_a_path_and_is_gated_too() {
        let model = models::Repository {
            id: Uuid::now_v7(),
            host_id: Uuid::now_v7(),
            repository_root_id: Uuid::now_v7(),
            display_name: "overnight".into(),
            canonical_git_dir: "/Users/someone/overnight/.git".into(),
            remote_summary: "github".into(),
            resource_version: 1,
        };
        assert_eq!(repository(&model, Scope::Read).canonical_git_dir, None);
        assert!(repository(&model, Scope::HostAdmin).canonical_git_dir.is_some());
        // The display name is not a path and is how a user recognises the repo.
        assert_eq!(repository(&model, Scope::Read).display_name, "overnight");
    }

    #[test]
    fn timestamps_survive_the_epoch_boundary() {
        // rem_euclid, not %, or a pre-1970 millisecond yields a negative nanos
        // field that violates the protobuf contract.
        let before = timestamp(-1);
        assert_eq!(before.seconds, -1);
        assert!(before.nanos >= 0, "nanos must never be negative");
        assert_eq!(timestamp(1500).seconds, 1);
        assert_eq!(timestamp(1500).nanos, 500_000_000);
    }
}
