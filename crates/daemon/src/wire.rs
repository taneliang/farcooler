//! Turning domain models into protocol messages.
//!
//! Two rules hold everywhere in this file.
//!
//! **Paths are `host_admin` only.** Every path-bearing field is optional in the
//! proto and is populated only for a client that holds `host_admin`. Ordinary
//! clients get an opaque token instead. A phone on someone else's network has
//! no business learning the directory layout of the runner it is driving, and
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

/// Runner facts, including whether the runtime is answering.
///
/// Health is reported by the daemon rather than sampled by the client. A remote
/// client has no tmux to look at, and a local one that looked would be a second
/// authority on the same question — which is the whole failure the derivation
/// model exists to prevent.
///
/// Still `Host` on the wire, deliberately. `proto/farcooler.proto` is a
/// versioned interface with negotiation on both sides — see
/// `docs/superpowers/specs/2026-08-11-api-versioning-design.md`. Renaming a
/// message is a protocol change that needs a version and a compatibility
/// window, not a side effect of a vocabulary decision. The word a person sees
/// is "runner"; the word on the wire changes when someone plans that change on
/// purpose. That is why `host_id`, `HostSettings` and `Scope::HostAdmin` still
/// read as they do here — every one of them is the generated type or a direct
/// projection of it, and renaming the projection alone would only hide which
/// names are load-bearing.
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
        // Still `task_name` on the wire, computed from the worktree path rather
        // than read from a column. Every shipped client decodes this field, so
        // it keeps its name and they keep working against a daemon that no
        // longer stores one.
        task_name: ws.name(),
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
        turn_started_at: None,
        blocked_question: None,
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
        // Also the watcher's: the feed is built by reading the agent's session
        // log line by line, and only the sampling loop holds that file offset.
        // A converter that guessed would report an empty feed over a real one.
        feed: Vec::new(),
        // The watcher's for the identical reason: a spawned agent is found by
        // reading the same file at the same offset.
        subagents: Vec::new(),
        // The compact ladder is derived from `activity`, `blocked_question`
        // and `feed` above — all of them the watcher's to decide, and none of
        // them known yet at this point in the conversion. `apply_rungs` fills
        // these in once every other field on the message is set.
        glyph: String::new(),
        headline: String::new(),
        line: String::new(),
        rank: 0,
        // The watcher's as well, and for the strongest version of the reason:
        // it is read out of the agent's own session log, which only the
        // sampling loop holds a file offset into. `false` here is "nothing has
        // claimed this turn went badly", not "the turn went well".
        turn_failed: false,
    }
}

/// `terminal`, plus the ACP-facing fields only `AgentSupervisor` knows.
///
/// A second function rather than widening `terminal()` itself: `watch::
/// Watcher`'s broadcast path builds a `Terminal` for every state change on
/// the runner from a bare view, with no supervisor at hand, and giving it one
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

/// This terminal's compact ladder, from the facts already assembled on it.
///
/// Called after `activity`, `blocked_question`, `current_command`,
/// `turn_started_at`, `activity_changed_at`, `exit_status` and `feed` are all
/// set — from `Watcher::announce` and `Rpc::with_activity`, the two places
/// that finish building a live `Terminal` for a client, per this module's own
/// header comment. Not folded into `terminal()` or `terminal_with_agent_state`
/// themselves: both run before any of those fields are known, for the reasons
/// given on each of them above, so the ladder has to be computed after, from
/// the same message those callers are about to send — which is also what
/// keeps a Mac's reading and a phone's reading of the same terminal from ever
/// being built from different facts.
///
/// `signal` is the one fact that arrives BESIDE the message rather than on it:
/// where the agent is, in one line, already composed by
/// `farcooler_core::feed::signal` out of the task list the watcher folds. It
/// is an argument rather than a field because the composed line and the parts
/// it is composed from would be two things on the wire that must agree, and
/// every field this file's history is a list of went missing exactly that way.
/// Both callers hold the watcher, so both can supply it; a caller with no
/// watcher passes `None` and gets the ladder a pane with no session log has.
pub fn apply_rungs(message: &mut wire::Terminal, signal: Option<&str>) {
    let subject = rung_subject(message, crate::review::now_millis(), signal);
    message.glyph = farcooler_core::feed::glyph(&subject).to_string();
    message.headline = farcooler_core::feed::headline(&subject);
    message.line = farcooler_core::feed::line(&subject);
    message.rank = farcooler_core::feed::rank(&subject);
}

/// `message`'s Unix-millisecond timestamp fields, as milliseconds.
///
/// The same arithmetic `client::session::activity_since` and `cli::main::
/// activity_since` already carry — not shared with them because those live in
/// crates this one is not on the dependency graph of, and three copies of one
/// multiply-and-divide is a cheaper disagreement to avoid than a new shared
/// crate would be to add.
fn millis_of(ts: &Option<prost_types::Timestamp>) -> Option<i64> {
    ts.as_ref().map(|t| t.seconds * 1000 + i64::from(t.nanos) / 1_000_000)
}

/// How long ago `since` was, clamped to never-negative: a clock that jumped
/// backward between the sample and this read must not hand `rank` a duration
/// it would have to panic on.
fn elapsed_since(since: Option<i64>, now: i64) -> std::time::Duration {
    let since = since.unwrap_or(now);
    std::time::Duration::from_millis(now.saturating_sub(since).max(0) as u64)
}

/// Build the ladder's one argument from a `Terminal` that already carries
/// everything it needs.
///
/// `message.activity` is the discriminator between the two shapes: `None`
/// (not an agent at all — a plain shell) and `Unspecified` (the screen could
/// not be read this tick) both fall through to `Subject::Command`, because
/// neither has a session log or a turn clock to report — the same "we could
/// not tell, so we are not saying" honesty `TerminalState::Unknown` already
/// practices for process liveness.
fn rung_subject(
    message: &wire::Terminal,
    now: i64,
    signal: Option<&str>,
) -> farcooler_core::feed::Subject {
    use farcooler_core::feed::{AgentState, Exit, Subject};

    let state_age = elapsed_since(millis_of(&message.activity_changed_at), now);
    let activity = wire::AgentActivity::try_from(message.activity).unwrap_or(wire::AgentActivity::Unspecified);

    if matches!(activity, wire::AgentActivity::None | wire::AgentActivity::Unspecified) {
        return Subject::Command {
            command: message.current_command.clone(),
            exit: message.exit_status.as_ref().map(|e| Exit { code: e.code, signal: e.signal }),
            state_age,
        };
    }

    Subject::Agent {
        name: message.current_command.clone(),
        state: failure_narrowed(AgentState::from(activity), message.turn_failed),
        turn_elapsed: millis_of(&message.turn_started_at).map(|since| elapsed_since(Some(since), now)),
        state_age,
        question: message.blocked_question.clone(),
        signal: signal.map(str::to_string),
    }
}

/// `state`, reading a finished turn that DIED as the failure it was.
///
/// `Done` only, and that narrowness is the decision. `Done` is finished and
/// unseen, which is exactly the window where the news is worth an alarm: a
/// pane that has been looked at has moved to `Idle`, and a row still flying a
/// red mark hours after somebody read it is how a mark stops meaning anything.
/// `Working` and `Blocked` are about the turn happening NOW, which the last
/// turn's outcome says nothing about — and `Blocked` in particular must never
/// be overwritten by anything (see `watch::resolved_activity`).
fn failure_narrowed(
    state: farcooler_core::feed::AgentState,
    turn_failed: bool,
) -> farcooler_core::feed::AgentState {
    use farcooler_core::feed::AgentState;
    match (state, turn_failed) {
        (AgentState::Done, true) => AgentState::Failed,
        (state, _) => state,
    }
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
        models::PaneMode::Changes => wire::PaneMode::Changes as i32,
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

    // ---------------------------------------------------------------------
    // `apply_rungs`: the ladder, computed from a `Terminal` already carrying
    // everything `Watcher::announce` and `Rpc::with_activity` would have set.
    // ---------------------------------------------------------------------

    fn ago(secs: i64) -> Option<prost_types::Timestamp> {
        Some(timestamp(crate::review::now_millis() - secs * 1000))
    }

    #[test]
    fn apply_rungs_computes_the_ladder_for_a_blocked_agent() {
        let mut message = wire::Terminal {
            activity: wire::AgentActivity::Blocked as i32,
            activity_changed_at: ago(90),
            blocked_question: Some("Run: cargo test?".to_string()),
            current_command: "codex".to_string(),
            ..Default::default()
        };
        // A signal line is passed, and outranked. The question wins outright
        // for a blocked pane, which is the one priority in the whole ladder
        // that must never be negotiable.
        apply_rungs(&mut message, Some("3/7 · Designing test matrix"));
        assert_eq!(message.glyph, "?");
        assert_eq!(message.headline, "codex needs you");
        assert_eq!(message.line, "Run: cargo test?");
    }

    #[test]
    fn apply_rungs_computes_the_ladder_for_a_working_agent_on_a_task_list() {
        let mut message = wire::Terminal {
            activity: wire::AgentActivity::Working as i32,
            activity_changed_at: ago(240),
            turn_started_at: ago(240),
            current_command: "overnight-fix".to_string(),
            // The transcript is beside the signal line, not the source of it.
            // A row used to read its own last feed step back as `line`, which
            // is how `says Done.` became the most prominent string on a lock
            // screen.
            feed: vec!["I'll create fruit.txt, then verify it.".into()],
            subagents: vec!["Auditing the redaction rules".into()],
            ..Default::default()
        };
        apply_rungs(&mut message, Some("3/7 · Designing test matrix · 2 agents"));
        assert_eq!(message.glyph, "●");
        assert_eq!(message.headline, "overnight-fix 4m");
        assert_eq!(message.line, "3/7 · Designing test matrix · 2 agents");
    }

    #[test]
    fn apply_rungs_computes_the_ladder_for_a_finished_agent() {
        let mut message = wire::Terminal {
            activity: wire::AgentActivity::Done as i32,
            activity_changed_at: ago(30),
            current_command: "cursor".to_string(),
            feed: vec!["Done. `fruit.txt` now contains `banana`.".into()],
            ..Default::default()
        };
        apply_rungs(&mut message, Some("Writing fruit.txt"));
        assert_eq!(message.glyph, "✓");
        assert_eq!(message.headline, "cursor done");
        assert_eq!(message.line, "Writing fruit.txt");
    }

    /// The finding: a turn that DIED reached every client as a clean `done`.
    ///
    /// `turn_failed` is the only difference between this terminal and the one
    /// above, and every rung has to change with it — otherwise a cursor turn
    /// that came back "Named models unavailable" says `✓ cursor done` on the
    /// lock screen of somebody who now has to notice, unaided, that their
    /// fleet is one agent short.
    #[test]
    fn apply_rungs_reports_a_turn_that_died_as_failed_rather_than_done() {
        let failed = |turn_failed| {
            let mut message = wire::Terminal {
                activity: wire::AgentActivity::Done as i32,
                activity_changed_at: ago(30),
                current_command: "cursor".to_string(),
                turn_failed,
                ..Default::default()
            };
            apply_rungs(&mut message, Some("Running cargo test"));
            message
        };

        let died = failed(true);
        assert_eq!(died.glyph, "✗", "the mark a failed command already uses");
        assert_eq!(died.headline, "cursor failed");
        assert_eq!(died.line, "Running cargo test", "the last thing it did before it died");
        // Same tier as an ordinary `Done`: news that has already happened, not
        // an open question. What changes is what the row SAYS, not where it
        // sorts.
        assert_eq!(died.rank, failed(false).rank);

        // And a turn that ended well is untouched by any of this.
        assert_eq!(failed(false).glyph, "✓");
        assert_eq!(failed(false).headline, "cursor done");
    }

    /// A failure belongs to the turn that ended, so nothing about a turn still
    /// in flight may be repainted by it — least of all `Blocked`, which is the
    /// one state this whole layer is built to protect.
    #[test]
    fn a_turn_still_running_is_never_painted_by_the_last_turns_failure() {
        for activity in [wire::AgentActivity::Blocked, wire::AgentActivity::Working] {
            let mut message = wire::Terminal {
                activity: activity as i32,
                activity_changed_at: ago(30),
                turn_started_at: ago(30),
                current_command: "codex".to_string(),
                blocked_question: Some("Run: cargo test?".to_string()),
                turn_failed: true,
                ..Default::default()
            };
            apply_rungs(&mut message, None);
            assert_ne!(message.glyph, "✗", "{activity:?} was repainted as a failure");
            assert!(!message.headline.contains("failed"), "{}", message.headline);
        }
    }

    #[test]
    fn apply_rungs_computes_the_ladder_for_a_failed_non_agent_command() {
        // `activity: None` — "not an agent at all", per the proto — is what
        // routes this to `Subject::Command` instead of `Subject::Agent`.
        let mut message = wire::Terminal {
            activity: wire::AgentActivity::None as i32,
            activity_changed_at: ago(132),
            current_command: "cargo test".to_string(),
            exit_status: Some(wire::ExitStatus { code: Some(101), signal: None }),
            ..Default::default()
        };
        apply_rungs(&mut message, None);
        assert_eq!(message.glyph, "✗");
        assert_eq!(message.headline, "cargo test failed");
        assert_eq!(message.line, "cargo test · exit 101 · 2m 12s");
    }

    #[test]
    fn a_blocked_agent_outranks_a_finished_one_on_the_wire() {
        // The same ordering `farcooler_core::feed::rank` is tested against
        // directly, proved here end to end through `apply_rungs` so a future
        // change to `rung_subject` cannot silently stop passing the right
        // facts through even while the pure function underneath stays right.
        let mut blocked =
            wire::Terminal { activity: wire::AgentActivity::Blocked as i32, activity_changed_at: ago(1), ..Default::default() };
        let mut done =
            wire::Terminal { activity: wire::AgentActivity::Done as i32, activity_changed_at: ago(1), ..Default::default() };
        apply_rungs(&mut blocked, None);
        apply_rungs(&mut done, None);
        assert!(blocked.rank < done.rank, "blocked ({}) must outrank done ({})", blocked.rank, done.rank);
    }

    #[test]
    fn an_unspecified_activity_is_treated_as_a_non_agent_pane() {
        // A screen that could not be read this tick (`Unspecified`) has no
        // more to say than a plain shell does -- both fall through to
        // `Subject::Command` rather than `apply_rungs` guessing at a turn
        // clock that was never observed.
        let mut message = wire::Terminal {
            activity: wire::AgentActivity::Unspecified as i32,
            current_command: "zsh".to_string(),
            ..Default::default()
        };
        apply_rungs(&mut message, None);
        assert_eq!(message.glyph, "●", "still running, so still the in-progress glyph");
        assert_eq!(message.headline, "zsh running");
    }
}
