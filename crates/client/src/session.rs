//! An Far Cooler session, over whichever transport reached the runner.
//!
//! Above this line nothing knows whether it is talking to a local Unix socket
//! or to a daemon on the other side of an SSH connection. That is the property
//! worth protecting: a remote runner is not a different product with its own
//! subtly different behavior.
//!
//! Answers come back as JSON rather than protobuf. Not because JSON is better —
//! it is not, and the wire stays protobuf — but because the alternative is a
//! protobuf runtime and generated types in Swift and again in Kotlin, to
//! describe messages this crate has already decoded. JSON at the boundary means
//! the iOS app decodes the same shapes the Mac app already decodes from the
//! CLI, with the model types it already has.

use farcooler_protocol::v1::{
    AgentEventBatch, Repository, RepositoryRoot, Terminal, TerminalState, Worktree,
    WorktreeState, request, result,
};
use farcooler_transport::{Client, ClientError, CodecError};
use serde_json::json;
use uuid::Uuid;

use crate::changes_json::stack_json;
use crate::ssh;

mod board_reads;
mod changes;
mod web_board;
mod web_review;
mod board_order;
pub use board_order::task_reorder_request;
mod work;
pub mod workflow;
mod link;
use link::{Reader, Writer};
pub mod draft_prompt;
mod terminal_ids;
mod terminal_writer;
mod terminal_json;
use terminal_ids::{split_of, task_of, with_draft_hold};
mod files;
mod bring_draft;
mod interrupt;
mod members;
pub(crate) mod step_up;
mod enroll_wire;
use enroll_wire::{enroll_json, enroll_payload};
pub mod teammate;
mod approval;
mod node_key;
mod rekey;
mod pr_actions;
mod crash_report;
mod notice_task;
mod plan;
mod results;
mod rows;
pub use board_reads::{MarkRead, reads_json};
pub use files::FilesPlace;
use results::wrong;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Ssh(#[from] ssh::SshError),
    #[error("{0}")]
    Protocol(String),
    #[error("the daemon returned {got} where {expected} was expected")]
    WrongResult { expected: &'static str, got: &'static str },
    /// Names the binary this client actually asked for.
    ///
    /// A preview client asks for `farcoolerd-preview`, so a message naming
    /// `farcoolerd` would send someone to check the wrong thing — and finding
    /// it installed would make the error look like a lie.
    #[error(
        "connected, but `{daemon} --stdio` did not answer. Is Far Cooler installed on that runner?"
    )]
    DaemonMissing { daemon: &'static str },
    #[error("the runner runs protocol {daemon}; this client speaks {client}")]
    VersionMismatch { daemon: u32, client: u32 },
    /// A refusal the runner NAMED, carrying the stable code it named it by.
    ///
    /// Deliberately distinct from `Protocol`, which is this client failing to
    /// make sense of an answer. The code is the whole reason for the variant:
    /// it is the only part of a refusal a client can act on, and folding it
    /// into a string here is what made every refusal on both phones the same
    /// generic apology. `Display` is still just the message, so nothing that
    /// only wants the prose has to change.
    ///
    /// `what` is the runner's `Error.what`, empty when it named nothing:
    /// which argument it refused, or which conflict. `terminal.agent_answer`'s
    /// `not_held` and `not_delivered` are one code, and a phone's Needs You
    /// says "Someone already answered this." for one and "Try again." for the
    /// other.
    #[error("{message}")]
    Refused { code: i32, retryable: bool, message: String, what: String },
    /// The link underneath this session is gone.
    ///
    /// Deliberately distinct from `Protocol`, which is the far side saying
    /// something we could not make sense of — that is a session still worth
    /// talking on, and a client that reconnected over it would be reconnecting
    /// over a bug.
    #[error("the connection to the runner was lost: {0}")]
    Disconnected(String),
    /// No answer by the call's deadline (see `deadlines`). Not a disconnect:
    /// one slow answer is not a dead link, and the ssh keepalive is what
    /// decides that one. The message crosses the FFI as `error`, so it is a
    /// sentence; `method` and `after` are for logs.
    #[error("The runner took too long to answer. Try again.")]
    TimedOut { method: String, after: std::time::Duration },
    /// Input that was queued but never written: the link ended before any of
    /// it reached the wire (ov-250). Still a dropped link, so it answers
    /// `word` and `is_disconnect` as `Disconnected` does; the phones keep it
    /// for Try Again because it provably did not arrive.
    #[error(transparent)]
    NotSent(Box<SessionError>),
}

impl SessionError {
    /// The stable word for a failed connect, which crosses the FFI beside the
    /// message as `trouble` (ov-127). See `SshError::word`, whose words an
    /// `Ssh` failure keeps, and for why the phones read this and never the
    /// prose.
    pub fn word(&self) -> &'static str {
        match self {
            SessionError::Ssh(e) => e.word(),
            SessionError::Protocol(_) => "protocol",
            SessionError::WrongResult { .. } => "wrong_result",
            SessionError::DaemonMissing { .. } => "daemon_missing",
            SessionError::VersionMismatch { .. } => "version_mismatch",
            SessionError::Refused { .. } => "refused",
            SessionError::Disconnected(_) => "disconnected",
            SessionError::TimedOut { .. } => "timed_out",
            SessionError::NotSent(inner) => inner.word(),
        }
    }

    /// Whether this means the link is gone, rather than that the request was
    /// refused.
    ///
    /// Only meaningful for a call on an ESTABLISHED session. At connect time
    /// these same variants answer a different question — what could not be
    /// reached in the first place — and `DaemonMissing` in particular means
    /// something specific and actionable there ("go install it"), which is why
    /// `connect_ssh` keeps that reading and this does not. Once a session
    /// exists, a closed pipe is a closed pipe.
    pub fn is_disconnect(&self) -> bool {
        match self {
            SessionError::Ssh(_)
            | SessionError::Disconnected(_)
            | SessionError::DaemonMissing { .. } => true,
            SessionError::NotSent(inner) => inner.is_disconnect(),
            SessionError::Protocol(_)
            | SessionError::Refused { .. }
            | SessionError::WrongResult { .. }
            | SessionError::VersionMismatch { .. }
            | SessionError::TimedOut { .. } => false,
        }
    }
}

impl From<ClientError> for SessionError {
    fn from(error: ClientError) -> Self {
        match error {
            // Silence from the far side almost always means the command did not
            // exist, not that the protocol went wrong.
            ClientError::Closed | ClientError::NoHello => {
                SessionError::DaemonMissing { daemon: daemon_binary() }
            }
            ClientError::VersionMismatch { daemon, client } => {
                SessionError::VersionMismatch { daemon, client }
            }
            // The socket, not the conversation on it. `Framing` is deliberately
            // not here: bytes that do not decode are a bug on one side or the
            // other, and calling that a dropped link would send a client into a
            // reconnect loop that reproduces it on every attempt.
            ClientError::Connect(e) => SessionError::Disconnected(e.to_string()),
            ClientError::Codec(CodecError::Io(e)) => SessionError::Disconnected(e.to_string()),
            ClientError::Codec(e @ CodecError::Truncated) => {
                SessionError::Disconnected(e.to_string())
            }
            // The runner said no, and said why. Kept whole: `Protocol` would
            // throw the code away, and the code is the only thing an app can
            // write a useful sentence from.
            //
            // `what` crosses too, since Needs You: an answer refused as
            // `not_held` and one refused as `not_delivered` are the same code,
            // and a phone owes them different sentences (spec §2.5).
            ClientError::Daemon { code, retryable, message, what } => {
                SessionError::Refused { code, retryable, message, what }
            }
            ClientError::TimedOut { method, after } => SessionError::TimedOut { method, after },
            ClientError::NotWritten(cause) => SessionError::NotSent(Box::new((*cause).into())),
            other => SessionError::Protocol(other.to_string()),
        }
    }
}

/// A connected Far Cooler runner.
pub struct Session {
    client: Client<Reader, Writer>,
    /// The SSH connection, held so it lives as long as the session does, and
    /// what every further channel is opened on. `None` on a local socket.
    link: Option<Box<dyn ssh::Exec>>,
    /// What this session's hellos call it: see `link::NATIVE_CLIENT`.
    client_name: &'static str,
    /// Where fleet news goes, once somebody has asked for it.
    ///
    /// `None` until `subscribe` is called, which is the honest default: this
    /// type is also the CLI's and the tests', and neither wants a callback
    /// firing on a runtime thread. Set by `subscribe` BEFORE it opens its
    /// channel, so a runner where the dedicated channel could not be opened
    /// still forwards whatever fleet news arrives on a terminal attachment —
    /// see `attach_stream`, which is subscribed to the same broadcast for free.
    events: std::sync::Mutex<Option<EventSink>>,
    /// The socket this session was opened on, when it was opened on one.
    ///
    /// Only `subscribe` reads it, and only so the push path can be proven
    /// against a real daemon without an ssh server in the test — the same
    /// argument `tests/against_a_real_daemon.rs` already makes for everything
    /// else it covers: ssh is a byte pipe, and what can go wrong here is above
    /// it. A desktop client on a local runner gets the same channel for free.
    #[cfg(feature = "native")]
    socket: Option<std::path::PathBuf>,
    /// Admin actions built and waiting for their signature (`step_up`).
    step_ups: std::sync::Mutex<Vec<step_up::Prepared>>,
}

/// What a client is told when the runner says something changed.
///
/// **These carry no delta on purpose.** Every variant means "re-read it", and
/// that is the whole contract: the daemon's own `fleet_changed` says so in the
/// proto ("a client re-reads the fleet rather than trying to apply it as a
/// delta"), reconciliation both creates and deletes rows in one pass, and three
/// editors — the app, the CLI and an agent driving the CLI — move the same
/// state. A client that applied deltas would need to be right about every one
/// of those; a client that re-reads needs only to be told that it should.
///
/// The consequence is the one that makes the whole push path cheap: losing one
/// of these is harmless as long as the client learns it missed SOMETHING, so
/// the boundary is free to drop and coalesce rather than buffer. See
/// `EventQueue` in `ffi.rs`, which is where that policy is written down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FleetEvent {
    /// The daemon dropped events addressed to this connection, and cannot
    /// say which: re-read everything. The same notice the FFI's queue sends
    /// on its own overflow (`EventQueue`).
    Resync,
    /// Something in the fleet moved: a terminal's state, a worktree, a
    /// layout, an operation, or the set of worktrees itself.
    ///
    /// One variant for all five because they have one answer — re-read
    /// `fleet` — and a client that told them apart would make the same call
    /// five times. It is also what makes coalescing work at all: a busy runner
    /// samples every second (`SAMPLE_INTERVAL` in `crates/daemon/src/watch.rs`)
    /// and a working agent can move a row on every sample, so the events that
    /// arrive in a burst have to collapse to one notice or the "push" is just
    /// a faster poll.
    Fleet,
    /// One worktree's diff moved. The set itself is not carried — see
    /// `announce_change_set` in `crates/daemon/src/watch.rs:2511` for why the
    /// daemon does not fan thousands of file records out to every device.
    ChangeSet { worktree: Uuid },
    /// A repository's pull request state was re-read, because somebody's
    /// `stack.get` found a cache nobody had filled.
    ///
    /// The event the phone's pull request row was invented around: the daemon
    /// fills that cache in the BACKGROUND and answers the read from cache at
    /// local speed, so without this the only delivery mechanism was asking
    /// again on the next poll. See `crates/daemon/src/watch.rs:2538`.
    Stack { repository: Uuid },
    /// A repository's board moved: a task was made, revised, moved, blocked,
    /// or had a note appended to it.
    ///
    /// The repository and not the task, because that is the read a client
    /// makes. `task.list` answers a whole board in one call and a board is the
    /// thing on screen; a client told only which task moved would still have to
    /// ask for the board to know where the row goes now. Which also keeps the
    /// coalescing in `EventQueue` doing useful work — a manager moving six
    /// tasks in a burst is one re-read, not six.
    ///
    /// **`actor` is carried, and it is the reason this variant is not just
    /// `Task { repository }`.** Every board write names who made it — `user`,
    /// `manager`, or `agent:<uuid>`, the word `Actor`'s `Display` produces and
    /// the same word `TaskNote.actor` stores — and a client that writes to a
    /// board hears its own write come back. Without the actor the only way to
    /// tell "somebody else moved this" from "you moved this" is to diff the
    /// board you just wrote, which is the loop this field exists to let a
    /// client skip. Whether a given client SHOULD skip it is that client's
    /// call: the Mac board deliberately does not, because its own writes and a
    /// person's writes from the CLI in another window are both `user`, and a
    /// board that dropped `user` would go blind to the second.
    ///
    /// **`workspace` is the board, now that a repository has several.** A
    /// board view is keyed by workspace, so the repository alone would make
    /// every board in it re-read on every write to any of them. `None` is a
    /// runner without `workstreams`, which has one board per repository, and a
    /// client reads it as "any board in `repository` may have moved".
    ///
    /// `from_workspace` is set only by `task.move`, which changes two boards
    /// in one event: without it the board the task LEFT would keep drawing a
    /// card that is no longer on it.
    Task {
        repository: Uuid,
        workspace: Option<Uuid>,
        from_workspace: Option<Uuid>,
        actor: String,
    },
    /// Something a person has to act on appeared, changed or went away:
    /// re-read `needs_you.list`. Its own notice rather than `Fleet`, because
    /// answering a decision or a chat ask changes no terminal and no
    /// worktree, so nothing else would say the list moved.
    NeedsYou,
    /// A board's read state moved on another device (ov-113). Carries the
    /// whole state, which a client merges as it is: every part only rises.
    /// Not a re-read notice, and not coalesced into one by the FFI's queue
    /// except against an identical line.
    Reads { workspace: Uuid, floor_ms: i64, opened: Vec<(Uuid, i64)> },
    /// A board's plan moved (ov-274): a theme, a lane or the queued order was
    /// written. A client showing that board's Plan re-reads `plan.get`. The
    /// board only, so two writes to one board are one notice.
    Plan { workspace: Uuid },
    /// The runner took this device's access away (ov-820): the last thing
    /// its connection says before the runner closes it. Kept by the app for
    /// that runner until a dial gets in again.
    Released,
}

impl FleetEvent {
    /// Read one wire event as news, or as nothing.
    ///
    /// `None` for the arms that are not fleet news: `TerminalFrame` is one
    /// connection's terminal bytes and has a reader of its own, and
    /// `AgentEvents` is a transcript batch which `agent.events` pages by
    /// cursor — turning either into "re-read the fleet" would spend a round
    /// trip re-reading state that did not change.
    fn of(payload: farcooler_protocol::v1::event::Payload) -> Option<Self> {
        use farcooler_protocol::v1::event::Payload;
        match payload {
            Payload::TerminalChanged(_)
            | Payload::WorktreeChanged(_)
            | Payload::OperationChanged(_)
            | Payload::LayoutChanged(_)
            | Payload::FleetChanged(_) => Some(FleetEvent::Fleet),
            Payload::ChangeSetChanged(c) => {
                Some(FleetEvent::ChangeSet { worktree: uuid_of(&c.worktree_id) })
            }
            Payload::StackChanged(s) => {
                Some(FleetEvent::Stack { repository: uuid_of(&s.repository_id) })
            }
            Payload::TerminalFrame(_) | Payload::AgentEvents(_) | Payload::TerminalWriterLost(_) => None,
            // A task notice (ov-94): the phones hear it as a push from the
            // relay, which their notification code draws; nothing to re-read.
            Payload::Notice(_) => None,
            // The plan layer (ov-268): the phones' Plan view re-reads that
            // board's plan (ov-274). The actor isn't carried: the plan has one
            // writer, the orchestrator, and a phone never writes it.
            Payload::PlanChanged(p) => Some(FleetEvent::Plan { workspace: uuid_of(&p.workspace_id) }),
            // Reserved arms no daemon emits yet. Named one by one rather than
            // swept up by `_` so that the day a NEW variant is added to this
            // oneof, this match stops compiling (E0004) and somebody decides
            // what a client should re-read — which for these is the
            // repository list, not the fleet, and so is not a decision to
            // make by default. That is what naming them individually buys:
            // exhaustiveness is checked against the enum's variant set, not
            // against what the daemon actually sends, so it says nothing
            // about an already-named arm that starts being sent — see
            // `TaskChanged` below, which is exactly that case and was handled
            // on its own rather than filed here.
            Payload::HostChanged(_)
            | Payload::RepositoryRootChanged(_)
            | Payload::RepositoryChanged(_) => None,
            // The daemon dropped events addressed to this connection. What
            // they were is gone, so everything is re-read.
            Payload::EventsMissed(_) => Some(FleetEvent::Resync),
            // Already debounced on the runner, and coalesced again by the
            // FFI's queue, since the line carries nothing to tell two apart.
            Payload::NeedsYouChanged(_) => Some(FleetEvent::NeedsYou),
            Payload::DeviceReleased(_) => Some(FleetEvent::Released),
            Payload::BoardReadsChanged(r) => Some(FleetEvent::Reads {
                workspace: uuid_of(&r.workspace_id),
                floor_ms: r.floor_ms,
                opened: r.opened.iter().map(|m| (uuid_of(&m.task_id), m.opened_ms)).collect(),
            }),
            // This arm used to return `None`, with a note saying to revisit it
            // when a board existed to hear it. The Mac board is that board, so
            // here it is. Kept as a sentence rather than deleted because the
            // shape of the mistake is worth leaving behind: the daemon had
            // been emitting `TaskChanged` since `task_ops` was wired to
            // `Watcher::announce_task_changed`, every client-side test still
            // passed, and the only symptom was a board that rendered once and
            // never moved — a bug no test in the plan could see, because every
            // board test was a model test.
            Payload::TaskChanged(t) => Some(FleetEvent::Task {
                repository: uuid_of(&t.repository_id),
                workspace: some_uuid(t.workspace_id.as_deref()),
                from_workspace: some_uuid(t.from_workspace_id.as_deref()),
                // Verbatim. This is a word the daemon composed and a client
                // compares; parsing it here into a kind and an id would give
                // two fields that can disagree, which is the exact thing the
                // proto's own comment on `TaskChanged.actor` refuses.
                actor: t.actor,
            }),
        }
    }
}

/// Where fleet news is delivered.
///
/// A plain callback rather than a channel, and that is the design rather than
/// a shortcut. A bounded channel would make the daemon's reader wait on a slow
/// consumer — the one thing a push path must never do — and an unbounded one
/// would grow without limit behind a consumer that stopped. A callback pushes
/// the decision to the layer that can actually make it: the FFI takes a lock,
/// coalesces into a fixed-size queue, and returns. Nothing here ever blocks.
///
/// It is called from a runtime thread, so it must not block and must not panic.
pub type EventSink = std::sync::Arc<dyn Fn(FleetEvent) + Send + Sync>;

/// The daemon this client is built to talk to.
///
/// One place rather than at each call site: the two `exec` invocations below
/// and the error that names the binary must agree, and a client that asked for
/// one daemon and then reported another would be actively misleading.
fn daemon_binary() -> &'static str {
    farcooler_protocol::CHANNEL.daemon_binary_name()
}

impl Session {
    /// Whether the connection has ended, as the reader has seen it.
    pub fn link_ended(&self) -> bool {
        self.client.has_ended()
    }

    pub fn daemon_version(&self) -> &str {
        &self.client.server_hello().daemon_version
    }

    /// What the runner on the other end can do, by name.
    ///
    /// Learned in the handshake, so it is available before the first request.
    /// A client that is newer than the runner it reached uses this to DIM the
    /// controls that runner cannot serve, with a reason — rather than hiding
    /// them, which makes the same app look different on two runners for no
    /// stated cause, or letting them fail on use, which teaches people that
    /// buttons sometimes do nothing.
    pub fn capabilities(&self) -> &[String] {
        &self.client.server_hello().capabilities
    }

    /// What THIS session is allowed to ask for, as the word `authorized_keys`
    /// spells it.
    ///
    /// The pair to `capabilities`, and neither substitutes for the other: a
    /// capability is what the RUNNER can serve, a scope is what this connection
    /// may ask it for, and a control needs both. Learned in the same handshake,
    /// so it is available before the first request at no round trip —
    /// `crates/transport`'s `handshake` has always computed it from the session's
    /// real grant and sent it in `ServerHello`. Nothing above the transport read
    /// it, which is why both phones ask and report the refusal.
    ///
    /// Deliberately NOT read out of `client.list`, which looks like it answers
    /// this and does not, three ways over: it reports what is written in the
    /// fence rather than what sshd applied to THIS session; a key somebody added
    /// by hand has no client id to match on and reads back `unspecified` while
    /// the real grant is host_admin; and it is stale by construction, because
    /// sshd reads `authorized_keys` once, at authentication.
    ///
    /// `unspecified` in two cases, and neither is "no grant". The wire's own
    /// default is zero, so a daemon that did not set the field reads as this —
    /// though there is no such daemon in this tree's history: `granted_scope` has
    /// been `ServerHello` field 5 since the first vertical slice, and every
    /// daemon since has set it from the session's real grant. The case that will
    /// actually happen is the other one: a NEWER runner naming a scope this build
    /// has no word for. Either way a caller must read it as "no answer" and keep
    /// offering what it offers today — see `can` above for the same shape.
    pub fn granted_scope(&self) -> &'static str {
        scope_label(self.client.server_hello().granted_scope)
    }

    /// Whether the runner can do something, by name.
    ///
    /// Empty means a daemon too old to answer the question at all — every one
    /// of those predates capabilities, so it has exactly the feature set that
    /// existed then. Reporting `true` for the two floor capabilities and
    /// `false` for the rest is the honest reading, and it is what lets a new
    /// app talk to an old runner without a special case at every call site.
    pub fn can(&self, capability: &str) -> bool {
        advertises(self.capabilities(), capability)
    }

    // ---- reads ----

    /// Everything a fleet view needs, in one round trip per resource.
    ///
    /// Shaped identically to the CLI's `worktree list --json`, so a client
    /// decodes one set of types no matter which one it is talking to.
    pub async fn fleet(&self) -> Result<serde_json::Value, SessionError> {
        let worktrees = self.worktrees().await?;
        // The whole list, not `self.terminals()`, because the FLEET's trace
        // hangs off the wrapper rather than off any one terminal — and
        // `terminals()` throws the wrapper away. The fleet's row is summed by
        // the daemon on purpose: each terminal's row snaps to the shortest
        // window that holds its own activity, so adding bucket 4 of a
        // five-minute row to bucket 4 of a two-hour row would add two different
        // spans of time. A phone holding only the rendered rows cannot do it.
        let list = self.terminal_list().await?;
        let terminals = &list.items;
        let host = self.host().await?;
        let healthy =
            host.self_health != farcooler_protocol::v1::SelfHealth::Degraded as i32;
        // The workstreams, from a runner that has them. `None` from one that
        // does not, and the envelope then carries no `workspaces` key at all:
        // an app reads that as one implicit workspace per repository, which
        // is today's layout. An empty list would say something else — a
        // runner with workstreams and no repositories.
        let workspaces = if self.can(farcooler_protocol::capability::WORKSTREAMS) {
            Some(self.workspaces().await?)
        } else {
            None
        };
        let known = workspaces.as_deref().unwrap_or(&[]);

        let items: Vec<_> = worktrees
            .iter()
            .map(|w| {
                let row = json!({
                    "id": uuid_of(&w.id).to_string(),
                    "short": short(&w.id),
                    // Which repository this worktree belongs to.
                    //
                    // Carried on the wire message all along and dropped here,
                    // which is what made a phone unable to ask anything
                    // repository-scoped about a worktree it was looking at —
                    // `stack.get` and `pr.refresh` both take a repository id,
                    // and the fleet was the only place a client learned about
                    // worktrees at all.
                    "repository": uuid_of(&w.repository_id).to_string(),
                    "task": w.task_name,
                    "branch": w.branch,
                    "worktree": w.worktree_path,
                    "state": worktree_label(w.state()),
                    // Offering to remove the repository's own checkout would
                    // offer to delete the directory the repository itself
                    // lives in — the client keeps the button off the menu
                    // entirely rather than relying only on the daemon's own
                    // refusal, same reasoning macOS's sidebar already uses.
                    "isMainCheckout": w.is_main_checkout,
                    // Where this card sits, so a client can put a worktree it
                    // learned about from an event in the right place without
                    // re-reading the whole fleet, and can send the order back
                    // when somebody drags one.
                    //
                    // A runner too old to have a rank sends none, and prost
                    // decodes that as 0 for every worktree — so a client must
                    // tie-break on something of its own rather than trust this
                    // alone. `workspace_order` in `daemon.version` is how it
                    // tells the two apart.
                    "ordinal": w.ordinal,
                    "terminals": terminals.iter()
                        .filter(|t| t.worktree_id == w.id)
                        .map(terminal_json::terminal_json)
                        .collect::<Vec<_>>(),
                });
                with_workspaces(row, w, terminals, known)
            })
            .collect();

        let workspaces = workspaces
            .map(|all| all.iter().map(crate::workspaces_json::workspace_json).collect());
        Ok(fleet_json(healthy, json!(host.live_terminal_count), &list, items, workspaces))
    }

    /// Every workstream on the runner, Main first within each repository.
    ///
    /// Refused here, without a round trip, on a runner that does not advertise
    /// `workstreams`, for `tasks`' reason: an old runner answers
    /// `workspace.list` with `CAPABILITY_UNSUPPORTED` anyway.
    pub async fn workspaces(
        &self,
    ) -> Result<Vec<farcooler_protocol::v1::Workspace>, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::WORKSTREAMS, "workspace.list")?;
        match self.value("workspace.list", None, None).await? {
            result::Value::WorkspaceList(l) => Ok(l.items),
            other => Err(wrong("workspace_list", &other)),
        }
    }

    pub async fn host(&self) -> Result<farcooler_protocol::v1::Host, SessionError> {
        match self.value("host.health", None, None).await? {
            result::Value::Host(h) => Ok(h),
            other => Err(wrong("host", &other)),
        }
    }

    pub async fn worktrees(&self) -> Result<Vec<Worktree>, SessionError> {
        match self.value("worktree.list", None, None).await? {
            result::Value::WorktreeList(l) => Ok(l.items),
            other => Err(wrong("worktrees", &other)),
        }
    }

    pub async fn terminals(&self) -> Result<Vec<Terminal>, SessionError> {
        Ok(self.terminal_list().await?.items)
    }

    /// The terminal list with its wrapper intact.
    ///
    /// `terminals()` is the right shape for every caller that wants panes, and
    /// throws away the one field that belongs to the LIST rather than to any
    /// pane in it: `fleet_trace`. `fleet` needs both, so it asks for both here
    /// rather than making a second round trip for one field.
    async fn terminal_list(
        &self,
    ) -> Result<farcooler_protocol::v1::TerminalList, SessionError> {
        match self.value("terminal.list", None, None).await? {
            result::Value::TerminalList(l) => Ok(l),
            other => Err(wrong("terminals", &other)),
        }
    }

    pub async fn repositories(&self) -> Result<Vec<Repository>, SessionError> {
        match self.value("repository.list", None, None).await? {
            result::Value::RepositoryList(l) => Ok(l.items),
            other => Err(wrong("repositories", &other)),
        }
    }

    pub async fn roots(&self) -> Result<Vec<RepositoryRoot>, SessionError> {
        match self.value("repository_root.list", None, None).await? {
            result::Value::RepositoryRootList(l) => Ok(l.items),
            other => Err(wrong("roots", &other)),
        }
    }

    /// The color schemes this runner defines.
    ///
    /// Only the runner's own — the built-ins are compiled into every client, so
    /// sending eleven fixed palettes down an ssh link on every connection
    /// would be a round trip spent to be told what the client already knows.
    pub async fn themes(&self) -> Result<Vec<farcooler_protocol::v1::Theme>, SessionError> {
        match self.value("theme.list", None, None).await? {
            result::Value::ThemeList(l) => Ok(l.items),
            other => Err(wrong("themes", &other)),
        }
    }

    // ---- mutations ----

    /// Create a worktree and branch, optionally with a terminal already in it.
    ///
    /// `terminal_preset` empty means no terminal — which is what a caller about
    /// to create its own agent terminal wants, and what keeps this compatible
    /// with every caller that predates the parameter.
    /// `adopt` takes `branch` over instead of creating it — the case where work
    /// ARRIVED on a branch rather than starting on one: pushed from another
    /// machine, handed over, or produced by a cloud agent. Creating is still
    /// the default, because it is still the common case.
    pub async fn create_worktree(
        &self,
        repository: Uuid,
        task: &str,
        branch: &str,
        base: &str,
        terminal_preset: &str,
        adopt: bool,
    ) -> Result<Worktree, SessionError> {
        self.create_worktree_in(repository, task, branch, base, terminal_preset, adopt, None).await
    }

    /// `create_worktree`, claimed for `workspace` when one is named.
    ///
    /// A phone making a worktree from a workspace's screen means that
    /// workspace, so the worktree is claimed for it (ruling 8). A runner
    /// without `workstreams` has no workspace but the implicit one, so the
    /// name is dropped there rather than refused, as `task_list_request`
    /// drops it.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_worktree_in(
        &self,
        repository: Uuid,
        task: &str,
        branch: &str,
        base: &str,
        terminal_preset: &str,
        adopt: bool,
        workspace: Option<Uuid>,
    ) -> Result<Worktree, SessionError> {
        let workstreams = self.can(farcooler_protocol::capability::WORKSTREAMS);
        let chosen = workspace.filter(|_| workstreams);
        // Otherwise claimed for the repository's Main, where the runner has
        // workspaces: a pane opened in an unclaimed worktree has no
        // workspace, so nothing it does there could ever claim it, and the
        // worktree would sit in Unclaimed for good. A phone making one from
        // nowhere more specific than the repository means Main.
        //
        // A list that can't be read makes the worktree unclaimed rather than
        // refusing it: the claim is a convenience, and the create is what
        // was asked for. A named workspace needs no list at all.
        let workspaces = if workstreams && chosen.is_none() {
            match self.workspaces().await {
                Ok(listed) => Some(listed),
                Err(e) => {
                    tracing::warn!(error = %e, "could not list workspaces; the new worktree is unclaimed");
                    None
                }
            }
        } else {
            None
        };
        let (workspace_id, required) = claim(repository, chosen, workspaces.as_deref());
        let payload = request::Payload::WorktreeCreate(farcooler_protocol::v1::WorktreeCreate {
            task_name: task.into(),
            branch: branch.into(),
            base_revision: base.into(),
            terminal_preset: terminal_preset.into(),
            adopt_existing: adopt,
            fork_only: false,
            workspace_id,
        });
        match self.value_requiring("worktree.create", Some(repository), Some(payload), required).await? {
            result::Value::Worktree(w) => Ok(w),
            other => Err(wrong("worktree", &other)),
        }
    }

    pub async fn create_terminal(
        &self,
        worktree: Uuid,
        title: &str,
        preset: &str,
        join_active_group: bool,
    ) -> Result<Terminal, SessionError> {
        let payload = request::Payload::TerminalCreate(farcooler_protocol::v1::TerminalCreate {
            title: title.into(),
            command_preset: preset.into(),
            join_active_group,
            prompt: None,
            task_key: None,
            agent: None,
        });
        match self.value("terminal.create", Some(worktree), Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    pub async fn hide_worktree(&self, worktree: Uuid) -> Result<(), SessionError> {
        Ok(crate::actions::hide_worktree(&self.client, worktree).await?)
    }

    /// Try again to download a worktree's large files (ov-199).
    pub async fn hydrate_lfs(&self, worktree: Uuid) -> Result<(), SessionError> {
        Ok(crate::actions::hydrate_lfs(&self.client, worktree).await.map(|_| ())?)
    }

    pub async fn unhide_worktree(&self, worktree: Uuid) -> Result<(), SessionError> {
        Ok(crate::actions::unhide_worktree(&self.client, worktree).await?)
    }

    /// Put the worktrees in this order, first on screen first.
    pub async fn reorder_worktrees(&self, ordered: &[Uuid]) -> Result<(), SessionError> {
        Ok(crate::actions::reorder_worktrees(&self.client, ordered).await?)
    }

    /// Remove a worktree, or find out it needs the task name typed first —
    /// see `actions::remove_worktree` for what `confirm` means.
    pub async fn remove_worktree(
        &self,
        worktree: Uuid,
        confirm: &str,
    ) -> Result<crate::actions::RemoveWorktreeOutcome, SessionError> {
        Ok(crate::actions::remove_worktree(&self.client, worktree, confirm).await?)
    }

    pub async fn add_repository_root(
        &self,
        absolute_path: &str,
    ) -> Result<RepositoryRoot, SessionError> {
        Ok(crate::actions::add_repository_root(&self.client, absolute_path).await?)
    }

    pub async fn register_repository(
        &self,
        relative_path: &str,
    ) -> Result<Repository, SessionError> {
        Ok(crate::actions::register_repository(&self.client, relative_path).await?)
    }

    // ---- runner settings ----
    //
    // Every write answers with what the file now says, read back by the daemon
    // rather than echoed from the request, so a value the writer normalized is
    // what the caller ends up holding.

    pub async fn set_branch_prefix(
        &self,
        prefix: &str,
    ) -> Result<farcooler_protocol::v1::Host, SessionError> {
        let payload =
            request::Payload::HostSettings(farcooler_protocol::v1::HostSettings {
                branch_prefix: prefix.to_string(),
                projector: false,
            });
        match self.value("settings.set_branch_prefix", None, Some(payload)).await? {
            result::Value::Host(h) => Ok(h),
            other => Err(wrong("host", &other)),
        }
    }

    pub async fn upsert_theme(
        &self,
        theme: farcooler_protocol::v1::Theme,
    ) -> Result<Vec<farcooler_protocol::v1::Theme>, SessionError> {
        let payload = request::Payload::Theme(theme);
        match self.value("theme.upsert", None, Some(payload)).await? {
            result::Value::ThemeList(l) => Ok(l.items),
            other => Err(wrong("themes", &other)),
        }
    }

    pub async fn delete_theme(
        &self,
        name: &str,
    ) -> Result<Vec<farcooler_protocol::v1::Theme>, SessionError> {
        // The name travels in `TypedConfirmation`, which carries exactly one
        // string. Not a confirmation of intent: deleting a theme touches no
        // files and is undone by saving it again.
        let payload = request::Payload::TypedConfirmation(
            farcooler_protocol::v1::TypedConfirmation { typed_confirmation: name.to_string() },
        );
        match self.value("theme.delete", None, Some(payload)).await? {
            result::Value::ThemeList(l) => Ok(l.items),
            other => Err(wrong("themes", &other)),
        }
    }

    pub async fn stop_terminal(&self, terminal: Uuid) -> Result<(), SessionError> {
        self.value("terminal.stop", Some(terminal), None).await.map(|_| ())
    }

    pub async fn restart_terminal(&self, terminal: Uuid) -> Result<(), SessionError> {
        self.value("terminal.restart", Some(terminal), None).await.map(|_| ())
    }

    pub async fn dismiss_lost(&self, terminal: Uuid) -> Result<(), SessionError> {
        self.value("terminal.dismiss_lost", Some(terminal), None).await.map(|_| ())
    }

    /// Delete a terminal's record. The daemon refuses a `Running` or
    /// `Starting` one, so closing a live pane is `stop_terminal` first.
    pub async fn remove_terminal(&self, terminal: Uuid) -> Result<(), SessionError> {
        self.value("terminal.remove", Some(terminal), None).await.map(|_| ())
    }

    /// A user has looked at this terminal, which is what ends `Done`.
    ///
    /// `Done` is idle-and-UNSEEN, so a client that can show a terminal has to be
    /// able to say it showed one. Without this the phone could open a finished
    /// agent, read it, and leave it still announcing itself — on the phone, on
    /// the Mac, and in every push notification after.
    pub async fn mark_seen(&self, terminal: Uuid) -> Result<(), SessionError> {
        self.value("terminal.seen", Some(terminal), None).await.map(|_| ())
    }

    /// The terminals this client currently has in front of a person, and the
    /// whole set of them.
    ///
    /// What stops a push arriving about a pane you are sitting there reading.
    /// `mark_seen` above answers the same question one beat too late — a client
    /// reports it on the poll AFTER the agent finished, and the notification
    /// crossed the relay while that poll was in flight — so the runner has to be
    /// told a pane is being watched BEFORE the turn ends, not after. See
    /// `attention` in `crates/daemon/src/watch.rs`.
    ///
    /// Sent repeatedly, on whatever clock the caller already polls on, and the
    /// runner only believes it for a few seconds: a client that crashes, is
    /// suspended, or loses its link stops suppressing on its own rather than
    /// leaving a terminal permanently silent. An empty slice is a real call and
    /// the one that matters most — "I am looking at nothing" — which is what an
    /// app sends as it goes to the background, releasing every terminal it named
    /// in one round trip instead of waiting the claim out.
    ///
    /// Several ids because one window can show a whole tiled layout, and a pane
    /// tiled beside the focused one is as much on screen as the focused one.
    pub async fn report_watching(&self, terminals: &[Uuid]) -> Result<(), SessionError> {
        let payload = request::Payload::TerminalsWatched(
            farcooler_protocol::v1::TerminalsWatched {
                terminal_ids: terminals
                    .iter()
                    .map(|id| bytes::Bytes::copy_from_slice(id.as_bytes()))
                    .collect(),
            },
        );
        self.value("terminal.watching", None, Some(payload)).await.map(|_| ())
    }

    /// A terminal's visible screen, with escapes intact.
    ///
    /// `known_revision` is the last one received; the runner answers `unchanged`
    /// rather than resending a screen already held. Pass 0 to always get one.
    ///
    /// `history_lines` is how much of the pane's scrollback to send above that
    /// screen, and 0 — an ordinary poll — is none. It costs a second capture on
    /// the runner and a second one on the wire, so a client asks once, when it
    /// opens a pane, and polls with 0 from then on. The answer arrives in
    /// `history` even when the screen comes back `unchanged`: a client asking
    /// for scrollback is asking because it has none.
    pub async fn screen(
        &self,
        terminal: Uuid,
        known_revision: u64,
        history_lines: u32,
    ) -> Result<farcooler_protocol::v1::TerminalScreen, SessionError> {
        let payload = request::Payload::TerminalScreenRequest(
            farcooler_protocol::v1::TerminalScreenRequest { known_revision, history_lines },
        );
        match self.value("terminal.screen", Some(terminal), Some(payload)).await? {
            result::Value::TerminalScreen(s) => Ok(s),
            other => Err(wrong("terminal_screen", &other)),
        }
    }

    /// Send exact bytes to a terminal.
    pub async fn write(&self, terminal: Uuid, bytes: Vec<u8>) -> Result<(), SessionError> {
        self.start_write(terminal, bytes)?.value().await.map(|_| ())
    }

    /// `write`, queued for the wire before this returns. Keys sent this way
    /// reach the runner in the order this was called, whichever task waits.
    pub fn start_write(&self, terminal: Uuid, bytes: Vec<u8>) -> Result<Pending, SessionError> {
        let payload = request::Payload::TerminalWrite(farcooler_protocol::v1::TerminalWrite {
            payload: bytes.into(),
        });
        self.start_terminal_input("terminal.write", terminal, payload, None)
    }

    /// Send a file into a terminal, and return the path it landed at.
    ///
    /// The whole transfer lives here rather than in each app for the reason
    /// this crate exists: the chunking, the retry-free failure behavior and the
    /// size ceiling must not differ between a Mac, a phone and the CLI, and
    /// three implementations of a loop is three chances for them to.
    ///
    /// `progress` is called after each chunk with (sent, total). It is how a
    /// client draws a ring; nothing here depends on what it does.
    pub async fn paste_file(
        &self,
        terminal: Uuid,
        name: &str,
        mime: &str,
        file: &[u8],
        progress: impl FnMut(u64, u64),
    ) -> Result<String, SessionError> {
        // The size check is here rather than in `actions` so a phone can say
        // something a person can act on. `actions` refuses too, because the CLI
        // reaches it without passing through this.
        let total = file.len() as u64;
        if total == 0 {
            return Err(SessionError::Protocol("that file is empty".into()));
        }
        if total > farcooler_protocol::MAX_PASTE_FILE_BYTES {
            return Err(SessionError::Protocol(format!(
                "that file is {} MB, and the limit is {} MB",
                total / (1024 * 1024),
                farcooler_protocol::MAX_PASTE_FILE_BYTES / (1024 * 1024)
            )));
        }
        Ok(crate::actions::paste_file(&self.client, terminal, name, mime, file, progress).await?)
    }

    pub async fn resize_terminal(
        &self,
        terminal: Uuid,
        columns: u32,
        rows: u32,
    ) -> Result<(), SessionError> {
        self.start_resize(terminal, columns, rows)?.value().await.map(|_| ())
    }

    /// `resize_terminal`, queued before this returns, in order with
    /// `start_write`: a resize between two keys stays between them.
    pub fn start_resize(&self, terminal: Uuid, columns: u32, rows: u32) -> Result<Pending, SessionError> {
        let payload = request::Payload::TerminalResize(farcooler_protocol::v1::TerminalResize {
            columns,
            rows,
            view_activity_id: 0,
        });
        self.start_terminal_input("terminal.resize", terminal, payload, None)
    }

    // ---- agent channel ----
    //
    // Every payload here names its own `terminal_id` rather than the
    // envelope's `target_resource_id`, matching the daemon side: `agent_subscribe`
    // in particular legitimately targets a terminal that holds no session yet,
    // which is not the versioned-resource shape `target_resource_id` is for.

    /// New agent events for one terminal, from a cursor.
    ///
    /// Never an error for a terminal with no agent session: a client attaches
    /// to a PANE, not a session — the daemon answers an empty batch rather than
    /// refusing, and this call must not turn that into a special case here.
    pub async fn agent_subscribe(
        &self,
        terminal: Uuid,
        from_seq: u64,
        epoch: u64,
    ) -> Result<AgentEventBatch, SessionError> {
        let payload = request::Payload::AgentSubscribe(farcooler_protocol::v1::AgentSubscribe {
            epoch,
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            from_seq,
        });
        match self.value("terminal.agent_subscribe", None, Some(payload)).await? {
            result::Value::AgentEventBatch(b) => Ok(b),
            other => Err(wrong("agent_event_batch", &other)),
        }
    }

    /// Answer a pending permission request; a claude question with option
    /// `answer` and `answers`, each question's text to its answer (ov-370).
    pub async fn agent_answer(
        &self,
        terminal: Uuid,
        request_id: &str,
        option_id: &str,
        answers: std::collections::HashMap<String, String>,
    ) -> Result<Terminal, SessionError> {
        let payload = request::Payload::AgentAnswer(farcooler_protocol::v1::AgentAnswer {
            terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
            request_id: request_id.to_string(),
            option_id: option_id.to_string(),
            answers,
            // This library is the apps', so its caller is a person (ov-493).
            actor: "user".to_string(),
        });
        match self.value("terminal.agent_answer", None, Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// Worktree-relative paths matching `query`, for an `@`-mention.
    ///
    /// Relative, never a runner path: the same redaction the rest of this
    /// protocol applies to everything below `host_admin`.
    pub async fn search_worktree_files(
        &self,
        worktree: Uuid,
        query: &str,
        limit: u32,
    ) -> Result<Vec<String>, SessionError> {
        let payload =
            request::Payload::WorktreeFileSearch(farcooler_protocol::v1::WorktreeFileSearch {
                worktree_id: bytes::Bytes::copy_from_slice(worktree.as_bytes()),
                query: query.to_string(),
                limit,
            });
        match self.value("worktree.file_search", None, Some(payload)).await? {
            result::Value::WorktreeFileList(l) => Ok(l.paths),
            other => Err(wrong("worktree_file_list", &other)),
        }
    }

    /// What needs a person on this runner, in rank order, in the shape
    /// `farcooler needs-you --json` prints (`needs_you_json`).
    ///
    /// Refused here, without a round trip, on a runner that doesn't advertise
    /// `needs_you`: the app derives that runner's blocked items from the fleet
    /// instead, and the refusal's `capability-unsupported` is how it knows to.
    pub async fn needs_you(&self) -> Result<serde_json::Value, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::NEEDS_YOU, "needs_you.list")?;
        match self.value("needs_you.list", None, None).await? {
            result::Value::NeedsYouList(list) => Ok(crate::needs_you_json::needs_you_json(&list)),
            other => Err(wrong("needs_you_list", &other)),
        }
    }

    /// What agents spent on this runner, totaled as `query` asks
    /// (`usage.report`, ov-194). Refused without a round trip on a runner
    /// that records none.
    pub async fn usage_report(
        &self,
        query: farcooler_protocol::v1::UsageQuery,
    ) -> Result<farcooler_protocol::v1::UsageReport, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::AGENT_USAGE, "usage.report")?;
        match self.value("usage.report", None, Some(request::Payload::UsageReport(query))).await? {
            result::Value::UsageReport(report) => Ok(report),
            other => Err(wrong("usage_report", &other)),
        }
    }

    /// One task's spend, for its view (`usage.task`).
    pub async fn task_usage(&self, task: Uuid) -> Result<farcooler_protocol::v1::TaskUsage, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::AGENT_USAGE, "usage.task")?;
        let payload = request::Payload::UsageTask(farcooler_protocol::v1::TaskUsageRequest {
            task_id: bytes::Bytes::copy_from_slice(task.as_bytes()),
        });
        match self.value("usage.task", None, Some(payload)).await? {
            result::Value::TaskUsage(usage) => Ok(usage),
            other => Err(wrong("task_usage", &other)),
        }
    }

    /// The branches in a repository, for resuming onto one that already exists.
    pub async fn branches(&self, repository: Uuid) -> Result<serde_json::Value, SessionError> {
        match self.value("branch.list", Some(repository), None).await? {
            result::Value::BranchList(l) => Ok(json!({
                "branches": l.items.iter().map(|b| json!({
                    "name": b.name,
                    "local": b.local,
                    "remote": b.remote,
                    // git refuses a second checkout of the same branch, so a
                    // client has to be able to show this before someone picks it.
                    "checkedOut": b.checked_out,
                    "subject": b.subject,
                    "updatedAt": b.updated_at.as_ref().map(|t| t.seconds * 1000),
                })).collect::<Vec<_>>()
            })),
            other => Err(wrong("branch_list", &other)),
        }
    }

    /// A branch's parent chain and the PR state along it.
    pub async fn stack(
        &self,
        repository: Uuid,
        branch: &str,
    ) -> Result<serde_json::Value, SessionError> {
        let payload = request::Payload::StackGet(farcooler_protocol::v1::StackGet {
            repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
            branch: branch.to_string(),
        });
        match self.value("stack.get", Some(repository), Some(payload)).await? {
            result::Value::StackLinkList(l) => Ok(stack_json(&l)),
            other => Err(wrong("stack_link_list", &other)),
        }
    }

    /// Ask GitHub again, rather than answering from what was last read.
    pub async fn pr_refresh(&self, repository: Uuid) -> Result<serde_json::Value, SessionError> {
        let payload = request::Payload::PrRefresh(farcooler_protocol::v1::PrRefresh {
            repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
        });
        match self.value("pr.refresh", Some(repository), Some(payload)).await? {
            result::Value::StackLinkList(l) => Ok(stack_json(&l)),
            other => Err(wrong("stack_link_list", &other)),
        }
    }

    /// One repository's board, in the shape `farcooler task list --json`
    /// prints — see `tasks_json`, which both of them call.
    ///
    /// Read-only, and the whole board rather than a status: the phone draws
    /// every column and counts the one waiting on the person.
    ///
    /// Refused here, without a round trip, on a runner that does not
    /// advertise `tasks`. The daemon would refuse it too, with the same code;
    /// asking first only spends a request to be told what the handshake
    /// already said.
    ///
    /// `workspace` narrows the read to that workspace's board. See
    /// `task_list_request` for what it does on a runner without
    /// `workstreams`.
    pub async fn tasks(
        &self,
        repository: Uuid,
        workspace: Option<Uuid>,
    ) -> Result<serde_json::Value, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::TASKS, "task.list")?;
        let (list, required) = task_list_request(self.capabilities(), repository, workspace);
        let whole = self.task_pages(repository, list, required).await?;
        Ok(crate::tasks_json::board_json(&whole, now_millis()))
    }

    /// One task with its whole record, in the shape `farcooler task show
    /// --json` prints. Gated like `tasks`.
    pub async fn task(&self, task: Uuid) -> Result<serde_json::Value, SessionError> {
        Ok(crate::tasks_json::detail_json(&self.task_record(task).await?, now_millis()))
    }

    async fn task_record(&self, task: Uuid) -> Result<farcooler_protocol::v1::TaskDetail, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::TASKS, "task.get")?;
        let payload = request::Payload::TaskGet(farcooler_protocol::v1::TaskGetRequest {
            task_id: bytes::Bytes::copy_from_slice(task.as_bytes()),
            note_kind: 0,
        });
        match self.value("task.get", Some(task), Some(payload)).await? {
            result::Value::TaskDetail(d) => Ok(d),
            other => Err(wrong("task_detail", &other)),
        }
    }

    /// Append an entry to a task's record, as the person holding this
    /// device: a phone answering a decision writes an `answer` note, which is
    /// what takes the decision off Needs You. Gated like `tasks`.
    /// Answers with the entry, in `task show --json`'s note shape.
    pub async fn task_note(
        &self,
        append: farcooler_protocol::v1::TaskNoteAppend,
    ) -> Result<serde_json::Value, SessionError> {
        Ok(crate::tasks_json::note_json(&self.task_note_record(append).await?))
    }

    async fn task_note_record(
        &self,
        append: farcooler_protocol::v1::TaskNoteAppend,
    ) -> Result<farcooler_protocol::v1::TaskNote, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::TASKS, "task.note")?;
        let task = uuid_of(&append.task_id);
        match self.value("task.note", Some(task), Some(request::Payload::TaskNoteAppend(append))).await? {
            result::Value::TaskNote(n) => Ok(n),
            other => Err(wrong("task_note", &other)),
        }
    }

    /// Open a workspace's orchestrator (ruling 8: a workspace with none was a
    /// dead end on the phone). Refused without a round trip on a runner
    /// without `workstreams`, which has no orchestrators.
    pub async fn start_orchestrator(
        &self,
        workspace: Uuid,
        start: farcooler_protocol::v1::WorkspaceStartOrchestrator,
    ) -> Result<Terminal, SessionError> {
        require(
            self.capabilities(),
            farcooler_protocol::capability::WORKSTREAMS,
            "workspace.start_orchestrator",
        )?;
        let payload = request::Payload::WorkspaceStartOrchestrator(start);
        match self.value("workspace.start_orchestrator", Some(workspace), Some(payload)).await? {
            result::Value::Terminal(t) => Ok(t),
            other => Err(wrong("terminal", &other)),
        }
    }

    /// What the daemon is, and what it can do.
    ///
    /// Named apart from the `daemon_version` accessor above, which answers from
    /// the hello this session already exchanged and costs nothing. This one is a
    /// round trip and is worth it only for `capabilities`, which the hello does
    /// not carry.
    pub async fn daemon_capabilities(&self) -> Result<serde_json::Value, SessionError> {
        match self.value("daemon.version", None, None).await? {
            result::Value::DaemonVersion(v) => Ok(json!({
                "daemonVersion": v.daemon_version,
                "protocolVersions": v.protocol_versions,
                "capabilities": v.capabilities,
            })),
            other => Err(wrong("daemon_version", &other)),
        }
    }

    /// Whether the link answers, within `deadlines::PROBE` (ov-613).
    ///
    /// `daemon.version`, because the daemon answers it from memory: no git, no
    /// tmux, no inventory refresh, so the only thing its time measures is the
    /// link. Sent ahead of anything queued, so a diff in flight cannot make a
    /// healthy link look dead.
    pub async fn probe(&self) -> Result<(), SessionError> {
        self.probe_within(crate::deadlines::PROBE).await
    }

    /// `probe`, with the caller's own allowance (the page's, `WEB_PROBE`).
    pub async fn probe_within(&self, deadline: std::time::Duration) -> Result<(), SessionError> {
        let request = farcooler_transport::request("daemon.version");
        let how = farcooler_transport::CallOptions { deadline: Some(deadline), ..crate::deadlines::for_probe() };
        let answer = self.client.send(request, how)?;
        answer.answer().await?;
        Ok(())
    }

    /// Tell the runner why this client is about to end the link (`link.ending`,
    /// ov-1513), for its log. Queued and not waited on: the transport answers
    /// nothing, and over a dead link it never arrives. Sent urgent, which is
    /// what keeps a request nobody is waiting for in the queue.
    pub fn say_ending(&self, cause: &str, idle_ms: u64) {
        let mut request = farcooler_transport::request(farcooler_protocol::link_ending::METHOD);
        request.payload = Some(request::Payload::LinkEnding(farcooler_protocol::v1::LinkEnding {
            cause: cause.to_string(),
            idle_ms,
        }));
        let _ = self.client.send(request, farcooler_transport::CallOptions { deadline: None, urgent: true });
    }

    // ---- device enrollment ----
    //
    // Which devices may log in to this runner. The daemon owns every rule about
    // what may be written into `authorized_keys` and this owns none of them; the
    // one judgement here is the scope WORD, because the wire carries an enum and
    // neither Swift nor Kotlin has it.
    //
    // Answers are JSON rather than the wire messages, the same way `stack` and
    // `changes_inbox` are: these come straight back out through the FFI, and a
    // shape assembled in two places is two shapes the day one of them changes.
    //
    // `changes_inbox` was the proof of that rather than an example of it: it
    // WAS assembled in two places, here and in `farcooler changes inbox
    // --json`, and by the time anybody compared them they were two shapes — a
    // bare array with a short id against an object with a UUID. It builds
    // through `changes_json::inbox_json` now, which is where the rule this
    // comment states actually lives.

    /// Every line in this runner's fence, ours and otherwise.
    ///
    /// Foreign lines are included and marked, because a person looking at who
    /// may log in to their runner needs to see the key somebody added by hand as
    /// much as the ones Far Cooler wrote.
    pub async fn enrolled_clients(&self) -> Result<serde_json::Value, SessionError> {
        match self.value("client.list", None, None).await? {
            result::Value::ClientList(l) => Ok(enrolled_json(&l.items)),
            other => Err(wrong("client_list", &other)),
        }
    }

    /// Add a device's key to this runner's fence.
    ///
    /// `scope` is a word — `read`, `control` or `host_admin` — and an unknown one
    /// is refused here rather than sent. Refused rather than defaulted for the
    /// reason the daemon refuses an unspecified scope: a key with no scope at all
    /// already means host_admin to sshd, so rounding a misspelling up would turn
    /// a typo into the whole runner.
    ///
    /// `shell_access` chooses the SHAPE of the line: false is the restricted Key
    /// A line with its forced command, true is the plain Key B line that Zed, git
    /// and Terminal need a shell behind. A Mac calls this twice, once each way,
    /// with the same `client_id` — which is what lets one `revoke_client` remove
    /// both. The pairing rule (`shell_access` demands `host_admin`) is NOT
    /// re-checked here: the daemon owns every rule about what may be written into
    /// `authorized_keys`, and a copy of that rule in three client languages is
    /// three places for it to drift from the file's authority.
    /// `node_key` is the device's tailcat node public key, exactly as its
    /// ceremony offer carried it, and empty for a device that has none. Sending
    /// one is what asks this runner to join the tunnel network, and the answer's
    /// `connBlob` is the token the granting app puts in the reply's
    /// `Reach::Tailcat`. It goes on the Key A call only — the daemon refuses it
    /// beside `shell_access`, because a plain line has no forced command to hold
    /// it — and, like the pairing rule above, that refusal is the daemon's.
    pub async fn enroll_client(
        &self,
        public_key: &str,
        label: &str,
        client_id: &str,
        scope: &str,
        shell_access: bool,
        node_key: &str,
    ) -> Result<serde_json::Value, SessionError> {
        self.enroll_client_for(public_key, label, client_id, scope, shell_access, node_key, false).await
    }

    /// `enroll_client`, saying whose the new device is (ov-854).
    ///
    /// `for_teammate` is the owner's Add Teammate: the runner keeps the new
    /// line nobody's, and released, until the `member.set` that follows binds
    /// it (ov-828). It is the one flag this client never defaults on: a device
    /// written without it is the owner's until somebody binds it, which is what
    /// the owner's own devices want and a teammate's must never get.
    #[allow(clippy::too_many_arguments)]
    pub async fn enroll_client_for(
        &self,
        public_key: &str,
        label: &str,
        client_id: &str,
        scope: &str,
        shell_access: bool,
        node_key: &str,
        for_teammate: bool,
    ) -> Result<serde_json::Value, SessionError> {
        let payload = enroll_payload(public_key, label, client_id, scope, shell_access, node_key, for_teammate, None, "")?;
        match self.value("client.enroll", None, Some(payload)).await? {
            result::Value::ClientEnroll(r) => Ok(enroll_json(&r)),
            other => Err(wrong("client_enroll", &other)),
        }
    }

    /// Remove a device's line and close the sessions it was holding.
    ///
    /// Answers with what is left, which the daemon reads back out of the file
    /// after writing it: what `authorized_keys` now says is the only claim worth
    /// making about who may log in.
    ///
    /// `with_added` also revokes every device this one added, and those they
    /// added in turn, in the same write (ov-838).
    pub async fn revoke_client(
        &self,
        client_id: &str,
        with_added: bool,
    ) -> Result<serde_json::Value, SessionError> {
        self.revoke_client_with(client_id, with_added).await
    }

    /// `revoke_client`, taking with it every device the revoked one added that
    /// still has a line (`with_added`, ov-838): what a teammate's key that
    /// failed to bind is taken off with, since a key from before ov-828 read as
    /// the owner's and could have added others.
    pub async fn revoke_client_with(
        &self,
        client_id: &str,
        with_added: bool,
    ) -> Result<serde_json::Value, SessionError> {
        let payload =
            request::Payload::ClientRevoke(farcooler_protocol::v1::ClientRevoke {
                client_id: client_id.to_string(),
                with_added,
            });
        match self.value("client.revoke", None, Some(payload)).await? {
            result::Value::ClientList(l) => Ok(enrolled_json(&l.items)),
            other => Err(wrong("client_list", &other)),
        }
    }

    /// Remove a repository root. The paired `add` already exists above.
    ///
    /// `confirm` is the root directory's own name, typed by the person doing it.
    /// Unlike `remove_worktree`, there is no phase that asks first and finds
    /// out: the daemon demands the name every time, so a caller with nothing
    /// typed has nothing to send and `NameDidNotMatch` is the only thing an
    /// empty string can come back as.
    ///
    /// It used to take no `confirm` at all and send `None`, so every request
    /// built here was refused before the runner looked at the root — see
    /// `actions::remove_repository_root` for what that cost.
    pub async fn remove_repository_root(
        &self,
        root: Uuid,
        confirm: &str,
    ) -> Result<crate::actions::RemoveRootOutcome, SessionError> {
        Ok(crate::actions::remove_repository_root(&self.client, root, confirm).await?)
    }

    async fn value(
        &self,
        method: &str,
        target: Option<Uuid>,
        payload: Option<request::Payload>,
    ) -> Result<result::Value, SessionError> {
        self.value_requiring(method, target, payload, Vec::new()).await
    }

    /// `value`, naming the capabilities the request depends on.
    ///
    /// For a field an older daemon would silently drop — `TaskListRequest.
    /// workspace_id` is one — so that daemon refuses the request instead of
    /// answering a different question. See `Request.required_capabilities`.
    async fn value_requiring(
        &self,
        method: &str,
        target: Option<Uuid>,
        payload: Option<request::Payload>,
        required: Vec<String>,
    ) -> Result<result::Value, SessionError> {
        self.start(method, target, payload, required)?.value().await
    }

    /// Queue a call for the wire, with the deadline and priority `deadlines`
    /// gives its method, and return what waits for its answer.
    fn start(
        &self,
        method: &str,
        target: Option<Uuid>,
        payload: Option<request::Payload>,
        required: Vec<String>,
    ) -> Result<Pending, SessionError> {
        let mut request = farcooler_transport::request(method);
        request.required_capabilities = required;
        // This build places an FYI (ov-756): the list is the one thing it adds
        // to, so the one method that says so.
        if method == "needs_you.list" {
            request.accepts = vec![farcooler_protocol::accepts::NEEDS_YOU_FYI.to_string()];
        }
        if let Some(id) = target {
            request.target_resource_id = Some(bytes::Bytes::copy_from_slice(id.as_bytes()));
        }
        if let Some(p) = payload {
            request.payload = Some(p);
        }
        self.start_request(request)
    }

    fn start_request(&self, request: farcooler_protocol::v1::Request) -> Result<Pending, SessionError> {
        let method = request.method.clone();
        let answer = self.client.send(request, crate::deadlines::for_method(&method))?;
        Ok(Pending { answer, method })
    }
}

/// A call already queued for the wire, waiting for its answer. Dropping it
/// gives the call up. See `farcooler_transport::Answer`.
pub struct Pending {
    answer: farcooler_transport::Answer,
    method: String,
}

impl Pending {
    pub async fn value(self) -> Result<result::Value, SessionError> {
        let Pending { answer, method } = self;
        let outcome = answer.answer().await?;
        outcome.value.ok_or_else(|| SessionError::Protocol(format!("{method} returned nothing")))
    }
}

/// Whether a runner that advertised `advertised` can do `capability`.
///
/// Empty means a daemon too old to answer the question at all — every one of
/// those predates capabilities, so it has exactly the feature set that existed
/// then. Reporting `true` for the two floor capabilities and `false` for the
/// rest is the honest reading, and it is what lets a new app talk to an old
/// runner without a special case at every call site.
fn advertises(advertised: &[String], capability: &str) -> bool {
    if advertised.is_empty() {
        return capability == farcooler_protocol::capability::WORKTREES
            || capability == farcooler_protocol::capability::TERMINALS;
    }
    advertised.iter().any(|c| c == capability)
}

/// A board read, and the capabilities it depends on.
///
/// With a workspace, on a runner that advertises `workstreams`, the read is
/// that workspace's board, and names `workstreams` as required: an older
/// daemon drops `workspace_id` without a word and would answer with the whole
/// repository.
///
/// With a workspace, on a runner WITHOUT `workstreams`, the workspace is left
/// off and the read is the repository's board. That runner has one board per
/// repository, and the only workspace an app can hold for it is the implicit
/// one AgentKit's `WorkspaceSummary.implicit(repository:)` stands in for, so
/// the repository's board IS that workspace's board. Refusing would leave an
/// old runner's board unreadable from a client keyed by workspace.
fn task_list_request(
    advertised: &[String],
    repository: Uuid,
    workspace: Option<Uuid>,
) -> (farcooler_protocol::v1::TaskListRequest, Vec<String>) {
    let workstreams = farcooler_protocol::capability::WORKSTREAMS;
    let workspace = workspace.filter(|_| advertises(advertised, workstreams));
    let list = farcooler_protocol::v1::TaskListRequest {
        repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
        status: 0,
        stale_after_millis: None,
        workspace_id: workspace.map(|w| bytes::Bytes::copy_from_slice(w.as_bytes())),
        page_after: None,
    };
    let required = workspace.map(|_| vec![workstreams.to_string()]).unwrap_or_default();
    (list, required)
}

/// The workspace a worktree the phones make is claimed for, and the
/// capabilities that claim depends on.
///
/// `chosen`, when the phone named one on a runner with `workstreams`.
/// Otherwise `repository`'s Main, from `workspaces` as that runner listed
/// them. `None` from a runner without them, which has no workspace to claim
/// for and would refuse the requirement, and where the list names no Main for
/// the repository.
pub(crate) fn claim(
    repository: Uuid,
    chosen: Option<Uuid>,
    workspaces: Option<&[farcooler_protocol::v1::Workspace]>,
) -> (Option<bytes::Bytes>, Vec<String>) {
    if let Some(chosen) = chosen {
        return (
            Some(bytes::Bytes::copy_from_slice(chosen.as_bytes())),
            vec![farcooler_protocol::capability::WORKSTREAMS.to_string()],
        );
    }
    let main = workspaces
        .unwrap_or_default()
        .iter()
        .find(|w| w.is_main && w.repository_id.as_ref() == repository.as_bytes());
    match main {
        Some(w) => (Some(w.id.clone()), vec![farcooler_protocol::capability::WORKSTREAMS.to_string()]),
        None => (None, Vec::new()),
    }
}

/// A note a phone writes: always as `user`, because the person holding the
/// device is the only actor a phone can be.
pub fn task_note_append(
    task: Uuid,
    kind: farcooler_protocol::v1::TaskNoteKind,
    body: &str,
) -> farcooler_protocol::v1::TaskNoteAppend {
    farcooler_protocol::v1::TaskNoteAppend {
        task_id: bytes::Bytes::copy_from_slice(task.as_bytes()),
        kind: kind as i32,
        body: body.to_string(),
        extra_json: String::new(),
        supersedes: None,
        actor: "user".to_string(),
    }
}

/// Refuse `method` on a runner that cannot serve it, as the runner itself
/// would: `CAPABILITY_UNSUPPORTED`, the code an app words as "this runner is
/// too old for that" rather than as a failure it should retry.
fn require(advertised: &[String], capability: &str, method: &str) -> Result<(), SessionError> {
    if advertises(advertised, capability) {
        return Ok(());
    }
    Err(SessionError::Refused {
        code: farcooler_protocol::v1::ErrorCode::CapabilityUnsupported as i32,
        retryable: false,
        message: format!("this runner does not serve {method}"),
        what: String::new(),
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// The page's clock: `SystemTime::now` panics on wasm32-unknown-unknown, which
/// has no clock of its own (ov-768).
#[cfg(target_arch = "wasm32")]
pub(crate) fn now_millis() -> i64 {
    js_sys::Date::now() as i64
}

pub fn uuid_of(bytes: &[u8]) -> Uuid {
    Uuid::from_slice(bytes).unwrap_or(Uuid::nil())
}

/// An id the runner may not have sent, as `None` rather than the nil uuid.
///
/// For the workspace fields, which are absent from a runner without
/// `workstreams` and absent on an unclaimed worktree. `uuid_of` would turn
/// both into the nil uuid, and a client would key a board by it — one board
/// every old runner's events and every unclaimed worktree shared. `task_of`
/// follows the same rule for the same reason.
pub fn some_uuid(bytes: Option<&[u8]>) -> Option<Uuid> {
    bytes.and_then(|b| Uuid::from_slice(b).ok()).filter(|u| !u.is_nil())
}

/// UUIDv7 puts a timestamp in its LEADING bytes, so anything created in the
/// same millisecond shares a prefix. Short ids use the trailing random bytes.
pub fn short(bytes: &[u8]) -> String {
    let s = uuid_of(bytes).simple().to_string();
    s[s.len() - 8..].to_string()
}

/// The agent's activity, as the daemon derived it.
///
/// Distinct from `state`, which is about the process. A Claude Code sitting at
/// a permission prompt and one halfway through a file edit are both `running`;
/// the difference between them is the reason to look at a fleet at all.
fn activity_label(a: i32) -> &'static str {
    use farcooler_protocol::v1::AgentActivity;
    match AgentActivity::try_from(a).unwrap_or(AgentActivity::Unspecified) {
        AgentActivity::None => "none",
        AgentActivity::Idle => "idle",
        AgentActivity::Working => "working",
        AgentActivity::Blocked => "blocked",
        AgentActivity::Done => "done",
        AgentActivity::Unknown => "unknown",
        AgentActivity::Unspecified => "none",
    }
}

/// When the activity last changed, as Unix milliseconds.
///
/// A client shows "working for 4m" from this rather than timing it locally,
/// which would restart at every reconnect and lie after a laptop sleeps.
fn activity_since(t: &farcooler_protocol::v1::Terminal) -> Option<i64> {
    t.activity_changed_at.as_ref().map(|ts| ts.seconds * 1000 + (ts.nanos as i64) / 1_000_000)
}

/// When the current turn started, as Unix milliseconds.
///
/// Distinct from `activity_since`: this is held across a permission prompt and
/// cleared when the turn ends, so it answers "how long has this been running"
/// rather than "how long has it been in this particular state".
fn turn_started_at(t: &farcooler_protocol::v1::Terminal) -> Option<i64> {
    t.turn_started_at.as_ref().map(|ts| ts.seconds * 1000 + (ts.nanos as i64) / 1_000_000)
}

/// The pane mode, as a word rather than a number.
///
/// Same reason `activity_label` exists: a client that switched on an integer
/// would hold a second copy of the enum and drift from it silently.
pub(crate) fn pane_mode_label(mode: i32) -> &'static str {
    match farcooler_protocol::v1::PaneMode::try_from(mode) {
        Ok(farcooler_protocol::v1::PaneMode::Changes) => "changes",
        Ok(farcooler_protocol::v1::PaneMode::Web) => "web",
        // Unspecified from an older daemon is terminal, the mode that always
        // works, and so is AGENT from a runner from before chat mode was
        // retired (ov-377): the pane's TUI is what's left to draw.
        _ => "terminal",
    }
}

fn worktree_label(s: WorktreeState) -> &'static str {
    match s {
        WorktreeState::Unspecified => "?",
        WorktreeState::Creating => "creating",
        WorktreeState::Ready => "ready",
        WorktreeState::Active => "active",
        WorktreeState::Error => "ERROR",
        WorktreeState::Hidden => "hidden",
        WorktreeState::WorktreeMissing => "worktree_missing",
    }
}

/// Enrolled devices, in the shape three apps decode.
fn enrolled_json(items: &[farcooler_protocol::v1::EnrolledClient]) -> serde_json::Value {
    json!({ "clients": items.iter().map(enrolled_client_json).collect::<Vec<_>>() })
}

/// One device that may log in to a runner.
///
/// Every field of the wire message is carried, and `scope` is a word rather than
/// the enum's number: neither Swift nor Kotlin has the generated enum, so a
/// number here would be two mapping tables in two languages and the second one
/// would be wrong. Same reasoning as `origin` on an adapter.
fn enrolled_client_json(
    client: &farcooler_protocol::v1::EnrolledClient,
) -> serde_json::Value {
    json!({
        "clientId": client.client_id,
        "fingerprint": client.fingerprint,
        "label": client.label,
        "scope": scope_label(client.scope),
        // Which local account's `authorized_keys` this line was read from.
        // Nothing in the line names it — the file's location does — so this is
        // the daemon's answer and cannot be derived here.
        "account": client.account,
        // 0 for unknown, which is the ordinary answer: `authorized_keys` records
        // no time, so only the reply to an enrollment that just happened has one.
        "enrolledAt": client.enrolled_at,
        // Far Cooler did not write this line. It is reported so a person can see
        // it is there, and it is never touched.
        "foreign": client.foreign,
        // This is the device's PLAIN line — Key B, the one Zed, git and Terminal
        // use — and it is the ONLY thing that tells the two rows of a Mac apart.
        // Both lines carry the same client id and both are ours; without this a
        // Key B row and a Key A row are the same JSON object, Settings > Devices
        // draws one of them twice, and the removal copy's promise ("removing this
        // takes that Mac's ssh, git and Zed access away, not only its Far Cooler
        // access") is attached to whichever row happened to sort first.
        //
        // Never true for a foreign line: a plain line of OURS is one this program
        // rendered and manages, and a stranger's key is left as it was found.
        "shellAccess": client.shell_access,
        // The device that added this one (ov-838): what lets Settings name,
        // beside a lost phone, the devices it added.
        "addedBy": client.added_by,
    })
}

/// A scope as the word `authorized_keys` spells it.
///
/// `unspecified` is reported as itself rather than rounded to anything, and that
/// is the honest answer twice over: it is what a foreign line grants, and what a
/// line of ours whose scope word this build does not have grants — which is
/// nothing, because the daemon serving it refuses the word too. The daemon's own
/// writer rounds an absent scope UP to host_admin when it renders a line, and
/// that asymmetry is deliberate: writing without a restriction is what an
/// unscoped line already means to sshd, whereas READING one as host_admin would
/// tell a person a device has access it does not have.
fn scope_label(scope: i32) -> &'static str {
    use farcooler_protocol::v1::Scope;
    match Scope::try_from(scope) {
        Ok(Scope::Read) => "read",
        Ok(Scope::Control) => "control",
        Ok(Scope::HostAdmin) => "host_admin",
        _ => "unspecified",
    }
}

/// The same words going the other way, for an enrollment request.
///
/// `None` is a word this build does not have, and the caller refuses rather than
/// resolving it — the same rule `fence::scope_from_word` applies on the daemon
/// side, and for the same reason: rounding a typo up is privilege escalation by
/// misspelling. `unspecified` is deliberately not a word a caller may pass; it
/// is a state a line can be in, not a grant anybody can ask for.
fn scope_from_word(word: &str) -> Option<farcooler_protocol::v1::Scope> {
    use farcooler_protocol::v1::Scope;
    match word {
        "read" => Some(Scope::Read),
        "control" => Some(Scope::Control),
        "host_admin" => Some(Scope::HostAdmin),
        _ => None,
    }
}



fn terminal_label(s: TerminalState) -> &'static str {
    match s {
        TerminalState::Unspecified => "?",
        TerminalState::Starting => "starting",
        TerminalState::Running => "running",
        TerminalState::Exited => "exited",
        TerminalState::Error => "error",
        TerminalState::Lost => "LOST",
        // The same word the CLI and both apps use. A phone showing a state the
        // Mac calls something else is two products.
        TerminalState::Unknown => "unknown",
    }
}

/// A fleet row with its workspace fields, and its terminals' too.
///
/// Added to the row after it is built rather than written into it, because
/// the row's `json!` is at the macro's recursion limit. The terminals in the
/// row are the worktree's in `terminals`' order, which is how the row was
/// built.
///
/// On the worktree: `workspace` owns it, or null while it is unclaimed and
/// from a runner without `workstreams`; `claim_source` is the machine word
/// for what made the claim; `foreign_writers` names the other workspaces with
/// a live terminal in it. On each terminal: `workspace`, whose work it is
/// doing (not always the worktree's owner), `role`, `splitOf`, the pane a
/// split made it beside (`split_of`), and `splitOfOrchestrator`, whether
/// that pane was the orchestrator then. The CLI's `worktree list --json`
/// spells all six the same way; see `workspaces_json`. The Mac
/// draws an orchestrator once, as its workspace's own row, and leaves it out
/// of the worktree it runs in.
fn with_workspaces(
    mut row: serde_json::Value,
    w: &Worktree,
    terminals: &[Terminal],
    known: &[farcooler_protocol::v1::Workspace],
) -> serde_json::Value {
    use crate::workspaces_json::{foreign_writers, role_word, workspace_of};
    row["workspace"] = json!(workspace_of(w.workspace_id.as_deref()));
    row["claim_source"] = json!(w.claim_source);
    row["foreign_writers"] = json!(foreign_writers(w, known));
    // The tasks working here that aren't finished, as `{id, key, title,
    // status}`: what lets a worktree's row and a pane name their task without
    // reading a board. `[]` from a runner too old to fill it. Here rather than
    // in `fleet` so the test below can see it; `worktree list --json` carries
    // the same key from the same function.
    row["open_tasks"] = crate::needs_you_json::open_tasks_json(&w.open_tasks);
    // How many large files weren't downloaded (ov-199); 0 from an older runner.
    row["lfs_pointers"] = json!(w.lfs_pointers);
    let mine = terminals.iter().filter(|t| t.worktree_id == w.id);
    if let Some(rows) = row["terminals"].as_array_mut() {
        for (out, t) in rows.iter_mut().zip(mine) {
            out["workspace"] = json!(workspace_of(t.workspace_id.as_deref()));
            out["role"] = json!(role_word(t.role));
            // Here rather than in the row's `json!`, which is at the macro's
            // recursion limit. The CLI's two projections carry the same key.
            out["splitOf"] = json!(split_of(t));
            out["splitOfOrchestrator"] = json!(split_of(t).and(t.split_of_orchestrator));
            // The runner's answer, not a rule the phones keep (ov-112).
            out["noticeTaskId"] = json!(notice_task::notice_task_of(t));
        }
    }
    row
}

/// The object `Session::fleet` returns, given what it read.
///
/// Its own function so the keys the apps decode by name can be checked by a
/// test: the round trips that fill it need a live daemon, and the spelling of
/// `fleetTraceAnchor` is a contract with `CoreModel.Fleet` that nothing on the
/// Swift side can see break.
fn fleet_json(
    healthy: bool,
    live_panes: serde_json::Value,
    list: &farcooler_protocol::v1::TerminalList,
    worktrees: Vec<serde_json::Value>,
    workspaces: Option<Vec<serde_json::Value>>,
) -> serde_json::Value {
    let mut fleet = json!({
        "runtime_healthy": healthy,
        "live_panes": live_panes,
        // Every pane's trace added together at one width, summed by the
        // daemon. Same encoding and the same absent-not-empty rule as a
        // terminal's own. See the comment on `activityTrace` above.
        "fleetTrace": fleet_trace(list),
        // Where that trace's newest bucket sits in time: its absolute index, a
        // NUMBER, beside the base64 rather than in it. What lets a phone
        // polling several runners sum their fleet traces onto one axis
        // exactly. Null with no trace, and from an older daemon.
        "fleetTraceAnchor": fleet_trace_anchor(list),
        "worktrees": worktrees,
    });
    // The workstreams, as `workspace list --json` prints them. Absent, not
    // empty, from a runner without `workstreams`; see `Session::fleet`.
    //
    // NOT AgentKit's `RunnerDirectory` key `"workspaces"`, which is older,
    // persisted, and holds worktrees. Same word, different thing; see
    // `workspaces_json`.
    if let Some(workspaces) = workspaces {
        fleet["workspaces"] = json!(workspaces);
    }
    fleet
}

/// The fleet's trace as `fleet` spells it: base64, or no key for none.
fn fleet_trace(list: &farcooler_protocol::v1::TerminalList) -> Option<String> {
    (!list.fleet_trace.is_empty()).then(|| farcooler_core::base64::encode(&list.fleet_trace))
}

/// The fleet trace's anchor as `fleet` spells it: only beside the bytes it
/// anchors, whatever the daemon sent. See `TerminalList.fleet_trace_anchor`.
fn fleet_trace_anchor(list: &farcooler_protocol::v1::TerminalList) -> Option<i64> {
    list.fleet_trace_anchor.filter(|_| !list.fleet_trace.is_empty())
}

#[cfg(test)]
mod tests;
