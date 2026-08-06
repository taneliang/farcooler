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

use farcooler_agent::event::{AgentEvent, AgentGapReason, Sequenced};
use farcooler_protocol::v1::{self as wire, Scope};
use farcooler_store::models;
use uuid::Uuid;

use crate::agent_supervisor::AgentSupervisor;
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

/// Host facts, including whether the runtime is answering.
///
/// Health is reported by the daemon rather than sampled by the client. A remote
/// client has no tmux to look at, and a local one that looked would be a second
/// authority on the same question — which is the whole failure the derivation
/// model exists to prevent.
pub fn host(
    daemon_version: &str,
    host_id: Uuid,
    runtime: &farcooler_core::inventory::RuntimeSnapshot,
    replay_bytes: u64,
) -> wire::Host {
    let healthy = runtime.inventory_healthy;
    wire::Host {
        id: id_bytes(host_id),
        resource_version: 1,
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        daemon_version: daemon_version.to_string(),
        protocol_version: farcooler_protocol::PROTOCOL_VERSION,
        self_health: if healthy {
            wire::SelfHealth::Healthy as i32
        } else {
            wire::SelfHealth::Degraded as i32
        },
        self_health_reasons: if healthy {
            Vec::new()
        } else {
            // Named, because "degraded" alone tells a user nothing about what
            // to do, and this particular degradation makes every terminal
            // derive as lost.
            vec!["tmux inventory unavailable; every terminal derives lost".into()]
        },
        replay_bytes_retained: replay_bytes,
        live_terminal_count: runtime.panes.len() as u32,
        // Read on every call rather than cached on the service, so editing
        // config.toml — or a settings editor writing it — takes effect without
        // a daemon restart. Same reasoning as `theme.list`, and a few hundred
        // bytes of TOML is not a cost worth a staleness bug.
        settings: Some(wire::HostSettings {
            branch_prefix: farcooler_core::config::load_branch_prefix(),
        }),
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
        is_main_checkout: ws.is_main_checkout,
    }
}

/// A terminal with its DERIVED state.
pub fn terminal(view: &TerminalView) -> wire::Terminal {
    let t = &view.terminal;
    let exit_status = (t.exit_code.is_some() || t.exit_signal.is_some()).then_some(wire::ExitStatus {
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
        // Left unset here on purpose. Activity is the watcher's to decide, and
        // it is the only thing that knows the previous observation — which is
        // what `Done` is made of. A converter that guessed would erase it.
        activity: wire::AgentActivity::Unspecified as i32,
        activity_changed_at: None,
        current_command: String::new(),
        pane_mode: pane_mode(t.pane_mode),
        agent_session_id: t.agent_session_id.clone(),
        // Left unset here for the same reason as `activity`: both describe a
        // live ACP session, and only the supervisor holding that session knows
        // them. A converter that guessed would report a mode the agent is not
        // in.
        agent_mode: None,
        available_agent_modes: Vec::new(),
        // The watcher's to decide, for the same reason as `activity`: it takes
        // a screen read, and only the sampling loop does those.
        chat_capable: false,
    }
}

/// `terminal`, plus the ACP-facing fields only `AgentSupervisor` knows.
///
/// A second function rather than widening `terminal()` itself: `watch::
/// Watcher`'s broadcast path builds a `Terminal` for every state change on
/// the host from a bare view, with no supervisor at hand, and giving it one
/// only to feed two fields it does not yet report is a call site this change
/// has no cause to touch. `Rpc::with_activity` — the one place terminal
/// replies and list entries are built — calls this instead.
pub fn terminal_with_agent_state(view: &TerminalView, agents: &AgentSupervisor) -> wire::Terminal {
    let mut message = terminal(view);
    let id = view.terminal.id;
    message.agent_mode = agents.agent_mode(id);
    message.available_agent_modes = agents.available_modes(id);
    // The conversation's own name, when it has one.
    //
    // A terminal's stored title is what it was created as — usually the task,
    // sometimes "Terminal 19". The agent names the conversation after what is
    // actually being worked on and revises it as that changes, which is a
    // better answer to "which pane is which" than anything Far Cooler knows.
    // Only ever an upgrade: without a title the stored one stands.
    if let Some(title) = agents.title(id) {
        message.title = title;
    }
    message
}

/// A replay or fast-attach batch, for one terminal.
///
/// Each event is JSON because the shape is `farcooler_agent::event::
/// AgentEvent`, the single Rust definition both apps decode; see
/// `AgentEventFrame` in the proto for why. A value that fails to serialize is
/// never simply skipped: skipping would leave a hole between two seq numbers
/// that looks, to a client counting on contiguous history, like a bug in its
/// own bookkeeping rather than a bad event. Reusing `link::encode_line` here
/// rather than calling `serde_json` directly keeps the daemon crate off a
/// dependency it needs for nothing else — the shim already carries it for the
/// exact same serialization.
pub fn agent_batch(
    terminal: Uuid,
    events: Vec<Sequenced>,
    epoch: u64,
) -> wire::AgentEventBatch {
    wire::AgentEventBatch {
        terminal_id: id_bytes(terminal),
        epoch,
        events: events
            .into_iter()
            .map(|s| {
                let payload_json = farcooler_agent::link::encode_line(&s.event)
                    .map(|line| line.trim_end().to_string())
                    .unwrap_or_else(|_| {
                        farcooler_agent::link::encode_line(&AgentEvent::Gap {
                            reason: AgentGapReason::Unparsed,
                        })
                        .map(|line| line.trim_end().to_string())
                        .expect("a fixed Gap literal always serializes")
                    });
                wire::AgentEventFrame { seq: s.seq, payload_json }
            })
            .collect(),
    }
}

/// The text an ACP `prompt` turn carries, from the blocks a client sent.
///
/// A `file_mention` renders as `@path` — the same syntax a user would have
/// typed by hand, so the adapter needs no separate representation for it. An
/// `image` block contributes no text: multimodal prompts are a later slice,
/// and dropping the bytes here rather than inventing placeholder text keeps
/// that boundary honest instead of pretending the block was handled.
/// The images in a prompt, ready for the shim.
///
/// These used to be dropped on the floor here — `prompt_text` matched
/// `Image(_) => None` — so a picture attached in either app travelled as far as
/// the daemon and no further. The protocol has carried `ImageBlock` all along.
pub fn prompt_images(blocks: &[wire::AgentPromptBlock]) -> Vec<farcooler_agent::event::PromptImage> {
    blocks
        .iter()
        .filter_map(|b| match &b.content {
            Some(wire::agent_prompt_block::Content::Image(image)) => {
                Some(farcooler_agent::event::PromptImage {
                    mime: image.mime_type.clone(),
                    base64: farcooler_core::base64::encode(&image.data),
                })
            }
            _ => None,
        })
        .collect()
}

pub fn prompt_text(blocks: &[wire::AgentPromptBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match &b.content {
            Some(wire::agent_prompt_block::Content::Text(t)) => Some(t.clone()),
            Some(wire::agent_prompt_block::Content::FileMention(p)) => Some(format!("@{p}")),
            Some(wire::agent_prompt_block::Content::Image(_)) | None => None,
        })
        .collect::<Vec<_>>()
        .concat()
}

/// Durable intent to the wire.
///
/// `UNSPECIFIED` is never produced: a client that cannot tell which mode a
/// pane is in would not know which surface to draw, so every terminal names
/// one.
pub fn pane_mode(mode: models::PaneMode) -> i32 {
    match mode {
        models::PaneMode::Terminal => wire::PaneMode::Terminal as i32,
        models::PaneMode::Agent => wire::PaneMode::Agent as i32,
    }
}

pub fn timestamp(unix_millis: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: unix_millis.div_euclid(1000),
        nanos: (unix_millis.rem_euclid(1000) * 1_000_000) as i32,
    }
}

/// A workspace's tiling, whole.
///
/// Sent whole and never as a diff. Layout is edited by the app, by the CLI, by
/// agents driving the CLI, and by anyone attached to the tmux session directly —
/// so a client that missed one event converges on the next rather than applying a
/// delta to a state it may not hold.
pub fn pane_group_list(
    workspace: Uuid,
    layouts: &[crate::layout::LayoutView],
) -> farcooler_protocol::v1::PaneGroupList {
    farcooler_protocol::v1::PaneGroupList {
        workspace_id: id_bytes(workspace),
        items: layouts.iter().map(pane_group).collect(),
    }
}

pub fn pane_group(view: &crate::layout::LayoutView) -> farcooler_protocol::v1::PaneGroup {
    let (columns, rows) = view.size();
    farcooler_protocol::v1::PaneGroup {
        id: view.window.window_id.clone(),
        workspace_id: id_bytes(view.window.workspace_id),
        name: view.window.name.clone(),
        active: view.window.active,
        columns,
        rows,
        layout: view.window.layout.clone(),
        panes: view
            .panes
            .iter()
            .map(|p| farcooler_protocol::v1::PaneRect {
                terminal_id: id_bytes(p.terminal_id),
                left: p.left,
                top: p.top,
                columns: p.columns,
                rows: p.rows,
                focused: p.pane_active,
                zoomed: p.zoomed,
            })
            .collect(),
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
            display_name: "farcooler".into(),
            canonical_git_dir: "/Users/someone/farcooler/.git".into(),
            remote_summary: "github".into(),
            resource_version: 1,
        };
        assert_eq!(repository(&model, Scope::Read).canonical_git_dir, None);
        assert!(repository(&model, Scope::HostAdmin).canonical_git_dir.is_some());
        // The display name is not a path and is how a user recognizes the repo.
        assert_eq!(repository(&model, Scope::Read).display_name, "farcooler");
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
