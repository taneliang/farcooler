//! Method dispatch: the seam where a `Request` becomes domain work.
//!
//! The transport owns framing, the handshake, request correlation and
//! backpressure. This owns exactly three things and nothing else:
//!
//! 1. **Scope.** Every method declares the scope it needs, in one table, and
//!    the check happens before the payload is even read. A method that forgets
//!    to declare one does not exist.
//! 2. **Method to service call.** A thin mapping. Business rules live in
//!    `service`, so a second transport cannot acquire different behavior.
//! 3. **Errors to wire codes.** Through `DomainError::wire()`, which is
//!    exhaustively matched, so an unmapped variant fails the build rather than
//!    reaching a phone as a generic failure at the moment the user needs to
//!    know what actually went wrong.

use std::sync::Arc;

use farcooler_agent::link::DaemonMessage;
use farcooler_core::{DomainError, Result};
use farcooler_protocol::method::Method;
use farcooler_protocol::v1::{
    Empty, Error as WireError, Request, Response, Result as WireResult, Scope, request, response,
    result,
};
use farcooler_store::models;
use farcooler_transport::{Handler, Peer};
use uuid::Uuid;

use crate::service::Service;
use crate::wire;

/// One connection's terminal stream, ended when nothing holds it any more.
///
/// A guard rather than a bare `JoinHandle`, so the abort cannot be forgotten by
/// one of the two things that must cause it: a second `terminal.attach`, which
/// replaces this, and the connection ending, which drops the last `Rpc` and the
/// factory that built them and takes this with it.
///
/// Dropping the push receiver already ends the stream — `Runtime::attach` stops
/// the moment a send fails — but only on the NEXT byte, and a pane sitting at a
/// shell prompt may not write one for hours. Until then the task waits in `read`
/// on the fanout, and a fanout with a watcher that will never leave keeps the
/// pane's `pipe-pane` alive for nobody. Two of those were once found still
/// running from sessions whose app had been relaunched; see `Runtime::stream`.
struct Attachment(tokio::task::JoinHandle<()>);

impl Drop for Attachment {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct Rpc {
    service: Arc<Service>,
    watcher: Arc<crate::watch::Watcher>,
    /// Where `terminal.attach` sends this connection's terminal bytes.
    ///
    /// Per connection, not per runner: an attachment is the answer to one
    /// session's request, and the broadcast every connection subscribes to is
    /// the wrong shape for it twice over — it would hand a pane's output to
    /// clients that never asked, and it drops frames from a slow reader, which
    /// for terminal bytes is an escape sequence cut in half. See
    /// `Handler::pushes`.
    push: farcooler_transport::PushSender,
    /// The attachment this connection is currently serving, so a second
    /// `terminal.attach` can end the first rather than interleave with it.
    attachment: Arc<std::sync::Mutex<Option<Attachment>>>,
    /// Who this request's connection belongs to: its scope, and which enrolled
    /// device it is.
    ///
    /// Both halves of one fact, from one place. The scope used to be copied
    /// from the connection into here and separately into the handshake, which
    /// is how a session could be TOLD it held `read` and then be permitted
    /// everything.
    peer: Peer,
    daemon_version: String,
    /// Fired by `daemon.shutdown`; the process's own stop signal, from inside.
    stop: Arc<tokio::sync::Notify>,
}

impl Rpc {
    /// Private, and it has to be: the attachment guard it takes is this
    /// module's own, and `RpcFactory` is the only thing that should be building
    /// one anyway — a handler constructed anywhere else is a connection
    /// revocation cannot find.
    fn new(
        service: Arc<Service>,
        watcher: Arc<crate::watch::Watcher>,
        peer: Peer,
        stop: Arc<tokio::sync::Notify>,
        push: farcooler_transport::PushSender,
        attachment: Arc<std::sync::Mutex<Option<Attachment>>>,
    ) -> Self {
        Self {
            service,
            watcher,
            peer,
            daemon_version: farcooler_protocol::BUILD.to_string(),
            stop,
            push,
            attachment,
        }
    }

    /// Which device made this call, for a log line that has to name one.
    ///
    /// A device id, never a key or a secret: it is the name this runner's own
    /// `authorized_keys` gave the entry, and it is already written in that file
    /// in plain text. `-` rather than an empty field for a local caller, so an
    /// audit line reads the same shape whoever made the call.
    fn who(&self) -> &str {
        self.peer.client_id.as_deref().unwrap_or("-")
    }
}

/// One `Rpc` per request, over one shared `Service`, for one connection.
///
/// In the library rather than beside the socket loop in `main.rs`, because it
/// is the only thing that knows a connection is a live session: it registers
/// one when it is built and holds it until the connection ends. A second copy
/// of that wiring — in a test harness, or in the stdio path — is a connection
/// that revocation cannot find, which fails silently and only for the device it
/// was supposed to contain.
///
/// The scope belongs to the connection; the service does not, and must not — a
/// second `Service` would mean a second SQLite handle and a second, divergent
/// view of the tmux inventory.
#[derive(Clone)]
pub struct RpcFactory {
    service: Arc<Service>,
    watcher: Arc<crate::watch::Watcher>,
    /// Shared with whatever is waiting to stop this process. See `daemon.shutdown`.
    stop: Arc<tokio::sync::Notify>,
    /// Who this connection is, for its whole life. Copied into every `Rpc` this
    /// builds, so the dispatcher's scope check and the handshake's advertised
    /// scope are the same value and not two that happen to agree.
    peer: Peer,
    /// This connection's registration, which ends when this does.
    ///
    /// An `Arc` because this struct is `Clone`: the session lasts until the last
    /// clone is gone, rather than until whichever clone happened to be dropped
    /// first.
    session: Arc<crate::sessions::Session>,
    /// This connection's own push channel, for `terminal.attach`.
    ///
    /// Built here rather than by the attach handler because `serve_connection`
    /// asks for the receiving half once, before the first request — a handler
    /// that only had one after somebody attached would have nothing to hand it.
    ///
    /// Bounded at `resync::TERMINAL_BACKLOG_BYTES`. See `resync::TerminalSink`
    /// for what happens at the bound.
    push: farcooler_transport::PushSender,
    /// The receiving half, until `Handler::pushes` takes it.
    ///
    /// A `std::sync::Mutex` and not a `tokio` one: it is locked once, held for
    /// the length of an `Option::take`, and `pushes` is not async. `Option`
    /// because a receiver can only be taken once — a second
    /// `serve_connection` on the same handler gets `None` and pushes nothing,
    /// rather than half of every attachment's bytes.
    pushes: Arc<std::sync::Mutex<Option<farcooler_transport::PushReceiver>>>,
    /// The attachment this connection is serving, so a second attach can end
    /// the first. Shared with every `Rpc` this builds, for the reason `session`
    /// is: the clones are one connection, not several.
    attachment: Arc<std::sync::Mutex<Option<Attachment>>>,
}

impl RpcFactory {
    /// The handler for one accepted connection, registered as a live session.
    ///
    /// Registration happens HERE rather than at the call sites so it cannot be
    /// forgotten by one of them. A connection that was never registered is one
    /// `client.revoke` will report closing and quietly leave open.
    pub fn new(
        service: Arc<Service>,
        watcher: Arc<crate::watch::Watcher>,
        stop: Arc<tokio::sync::Notify>,
        peer: Peer,
    ) -> Self {
        let session = service.sessions().open(peer.client_id.clone());
        let (push, pushes) = farcooler_transport::push_queue(crate::resync::TERMINAL_BACKLOG_BYTES);
        Self {
            service,
            watcher,
            stop,
            peer,
            session,
            push,
            pushes: Arc::new(std::sync::Mutex::new(Some(pushes))),
            attachment: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

impl Handler for RpcFactory {
    fn peer(&self) -> Peer {
        self.peer.clone()
    }

    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send {
        let rpc = Rpc::new(
            self.service.clone(),
            self.watcher.clone(),
            self.peer.clone(),
            self.stop.clone(),
            self.push.clone(),
            self.attachment.clone(),
        );
        let session = self.session.clone();
        async move {
            // The one request a closed connection could still serve.
            //
            // `serve_connection` returns the moment a session is closed and
            // dispatches nothing after that, so this only ever catches a
            // request that was ALREADY in flight when the close landed —
            // microseconds, and exactly the microseconds in which somebody is
            // revoking a device they no longer trust. Answering it would be a
            // call served by a device whose access had already been withdrawn.
            if session.is_closed() {
                return error_response(req.request_id, DomainError::AuthRequired);
            }
            rpc.handle(req).await
        }
    }

    /// Every connection is subscribed to the push stream.
    ///
    /// No opt-in request: a client that connected wants to know when something
    /// changes, and making it ask would just be a round trip before the first
    /// event. Cost is zero on a quiet runner, because only changes are sent.
    fn events(&self) -> Option<tokio::sync::broadcast::Receiver<farcooler_protocol::v1::Event>> {
        Some(self.watcher.subscribe())
    }

    /// This connection's own channel, taken once. See the field.
    fn pushes(&self) -> Option<farcooler_transport::PushReceiver> {
        self.pushes.lock().ok().and_then(|mut held| held.take())
    }

    fn closed(&self) -> impl std::future::Future<Output = ()> + Send {
        let session = self.session.clone();
        async move { session.closed().await }
    }
}

/// `Host.agents_found` for this runner, now.
///
/// On a blocking thread: `programs::find` asks the login shell about a program
/// it hasn't found, which can take up to its five-second deadline, and a
/// missing agent is asked about again once a minute. Never on a runtime worker.
/// A lookup that panicked reports no agents rather than failing `host.get`,
/// whose other facts a client needs more.
async fn agents_found_now() -> Vec<String> {
    let stand_in = crate::service::stand_in_agent().is_some() || crate::service::test_stub_agents();
    tokio::task::spawn_blocking(move || wire::agents_found(stand_in, farcooler_core::programs::find))
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not look for the agents on this runner");
            Vec::new()
        })
}

/// A refusal in the shape every answer takes.
///
/// Shared with `Rpc::handle` so a response built outside the dispatcher cannot
/// be a differently shaped one — same code mapping, same redaction.
fn error_response(request_id: bytes::Bytes, err: DomainError) -> Response {
    let (code, retryable) = err.wire();
    Response {
        request_id,
        outcome: Some(response::Outcome::Error(WireError {
            code: code as i32,
            retryable,
            // Redacted by construction: never a path, terminal byte, command,
            // or session id.
            message: err.redacted_message(),
            // WHICH argument, when the code alone does not say. A client
            // switches on this and never shows it; see `DomainError::what`.
            what: err.what().to_string(),
        })),
    }
}

/// Hand a message to a terminal's shim, or say that nothing got it.
///
/// Every one of the nine agent methods below is a person acting on a chat, and
/// every one of them used to do nothing at all when no shim held that pane's
/// socket -- the supervisor found no writer, dropped the message, and the RPC
/// replied with the terminal read back, which is the same reply a delivery
/// gets. The prompt was the worst of them, because what it dropped was words
/// somebody had typed, but a mode a client believes it set and an answer to a
/// permission request that never arrives are the same lie.
///
/// `AgentNotConnected` and not `OperationFailed`, so a client can say which
/// thing happened; retryable, because the usual cause is a chat whose shim has
/// not finished dialing. `AgentStopped` for a pane whose agent is gone, which
/// no retry fixes (ov-174); see `AgentSupervisor::deliver`. The runner sends
/// the word and the app owns the sentence, as with every other code here.
fn to_the_shim(svc: &Service, terminal: Uuid, message: DaemonMessage) -> Result<()> {
    svc.agents().deliver(terminal, message)
}

/// The name a deny carries for the device that sent it: "Denied from iPhone".
///
/// `local` is a caller on this runner's own socket, the Mac app or the CLI,
/// and it has no enrollment to name it, so it is named for the computer the
/// runner is (`local_name`). A remote device is named by its
/// enrollment label, which is whatever a device sent when it paired, so it is
/// cut to one short line: a newline or a control character in a message
/// claude shows the model is not something a device gets to put there.
fn decider_name(local: bool, label: Option<&str>) -> String {
    /// Long enough for any device name a person chose; short enough to stay
    /// one line in the model's context.
    const LONGEST_NAME: usize = 40;
    if local {
        return local_name(cfg!(target_os = "macos")).to_string();
    }
    let clean: String = label.unwrap_or_default().chars().filter(|c| !c.is_control()).collect();
    let name: String = clean.trim().chars().take(LONGEST_NAME).collect();
    let name = name.trim_end();
    if name.is_empty() { "a paired device".to_string() } else { name.to_string() }
}

/// What the computer the runner runs on is called in a deny from its own
/// socket. "Mac" is only true on a Mac; a Linux runner's local caller is the
/// CLI on that machine.
fn local_name(macos: bool) -> &'static str {
    if macos { "Mac" } else { "this computer" }
}

/// The scope a method requires, by its wire name, or `None` for a name this
/// build does not know, which `handle` refuses as a capability it lacks.
pub(crate) fn required_scope(method: &str) -> Option<Scope> {
    Method::parse(method).map(scope_of)
}

/// The scope each method requires.
///
/// Exhaustive by construction: a match on `Method` with no wildcard, so a
/// method added to the protocol's table does not compile here until it has a
/// scope. An unknown name never reaches this; `required_scope` refuses it
/// rather than defaulting it.
fn scope_of(method: Method) -> Scope {
    match method {
        // Themes are what a client paints with. Read, not Control: naming a
        // color changes nothing on the runner, and a phone connected in a
        // read-only capacity should still be able to render itself properly.
        Method::HostGet | Method::HostHealth | Method::DaemonVersion | Method::ThemeList => Scope::Read,
        // Stopping the daemon stops nothing a user is watching — terminals are
        // tmux's — but it is the one method that ends the process, so it sits
        // at the highest scope. A local caller already holds it; a remote one
        // gets it only where ssh has proved who they are.
        Method::DaemonShutdown => Scope::HostAdmin,
        Method::RepositoryList | Method::WorktreeList | Method::TerminalList | Method::BranchList => Scope::Read,
        Method::LayoutList => Scope::Read,
        // Discovery reveals paths, which live behind the same gate as every
        // other path in this protocol.
        Method::WorktreeDiscover => Scope::HostAdmin,
        Method::RepositoryRegister
        | Method::WorktreeCreate
        | Method::WorktreeHide
        | Method::WorktreeUnhide
        // Dragging a card is a preference about a list, the same weight as
        // hiding one. It writes no git data and reveals no path, so it sits
        // where hide and unhide sit rather than behind `host_admin`.
        | Method::WorktreeReorder
        // Writes the worktree's own large files and reveals no path.
        | Method::WorktreeHydrateLfs
        | Method::TerminalCreate
        | Method::TerminalResize
        | Method::TerminalStop
        | Method::TerminalDismissLost
        | Method::TerminalRestart
        | Method::TerminalSeen
        // Saying what is on your screen sits at the same scope as saying you
        // have read it, and for the same reason: both change what this runner
        // tells the owner. `read` is the scope handed to something that should
        // only see the SHAPE of the fleet, and a read-scoped client that could
        // assert attention could hold a terminal silent — which is a way of
        // withholding a notification, not a way of looking at one.
        | Method::TerminalWatching
        | Method::TerminalRemove
        // Reading a screen is `control`, not `read`.
        //
        // A screen is the most sensitive thing this protocol carries — it is
        // whatever the agent has on it, which routinely includes source, paths
        // and tokens — and `read` is the scope handed to something that should
        // only see the shape of the fleet.
        | Method::TerminalScreen
        // And a stream is that same screen, continuously, plus every byte
        // between one screen and the next. Anything looser than `terminal.screen`
        // would hand a read-scoped client strictly more than one call of it
        // already gives — so this sits here rather than reasoning from "watching
        // is not writing", which is not the axis this table splits on.
        | Method::TerminalAttach
        // Pasting a file writes bytes to a pane and a file to the runner.
        //
        // The bytes are the same privilege as `terminal.write`, and so is the
        // file: anything that can type into a shell can already create a file
        // of its choosing. That is why this accepts any type rather than only
        // images — the restriction protected nothing and cost the case people
        // actually want, which is dropping a PDF or a log on a pane.
        | Method::TerminalPasteFile
        | Method::TerminalWrite => Scope::Control,
        // A pane's agent channel is exactly as sensitive as its screen — it is
        // the same conversation, just structured — so it sits at the same
        // scope rather than behind `host_admin`. Search returns
        // worktree-relative paths only, never a runner path, so it belongs here
        // too rather than beside `worktree.discover`.
        Method::TerminalSetPaneMode
        | Method::TerminalAgentSubscribe
        | Method::TerminalAgentPrompt
        // Types into a TUI pane like `terminal.write`, but never Enter, and
        // only past the answer wake's gate (`Watcher::draft_into`).
        | Method::TerminalDraftPrompt
        // Types into the orchestrator's TUI and presses Enter, past the same
        // gate (`Watcher::tell_into`, ov-214).
        | Method::TerminalTell
        | Method::TerminalAgentAnswer
        | Method::TerminalAgentSetMode | Method::TerminalAgentSetModel | Method::TerminalAgentSetConfig
        | Method::TerminalAgentCancel
        // The queue is `control` for `terminal.agent_prompt`'s reason, because
        // each of these is a prompt. Steering sends a queued message into the
        // running turn; editing rewrites words the agent will read; cancelling
        // withdraws them. A read-scoped client cannot send a prompt, so it
        // cannot rewrite or launch one either. Not `host_admin`: they touch
        // nothing outside the conversation the pane's own chat already shows.
        | Method::TerminalAgentEditQueued
        | Method::TerminalAgentCancelQueued
        | Method::TerminalAgentSteerQueued
        | Method::WorktreeFileSearch => Scope::Control,
        // Review is `control`, and for exactly the reason the screen above is.
        //
        // A diff IS source. `read` is the scope handed to something that should
        // only see the shape of the fleet, and serving file content there would
        // quietly redefine what every already-enrolled read-only client is
        // allowed to see — a change of security posture made as a side effect of
        // adding a feature. `Scope` is runner-wide (there is no per-repository
        // authorization to reach for), so the honest answer is the scope that
        // can already read a terminal screen, which already shows source.
        Method::ChangesChangeSet
        | Method::ChangesCommitFiles
        | Method::ChangesFileDiff
        | Method::ChangesSetBase
        | Method::ChangesMarkRead
        | Method::StackSetParent
        | Method::PrRefresh
        // A file is source, the same as a diff (ov-189).
        | Method::WorktreeListDir
        | Method::WorktreeReadFile => Scope::Control,
        // Metadata about work, not the work. Counts, +/-, PR state and the
        // needs-you badge let a read-scoped phone triage the fleet without being
        // able to read a line of the code.
        Method::ChangesInbox | Method::StackGet => Scope::Read,
        // The shape of the work, like `changes.inbox`: what is waiting on a
        // person and where. Below `control` the converter drops what an ask's
        // option names carry, which is the raw command or path
        // (`needs_you::redact_below_control`).
        Method::NeedsYouList => Scope::Read,
        // Counts, durations, and the keys and titles of tasks: what the board
        // reads below already show a read-scoped phone.
        Method::ReportGet => Scope::Read,
        // Counts, times and dollars about work, not the work (ov-194).
        Method::UsageReport | Method::UsageTask => Scope::Read,
        // The board reads. A task's title, intent and record are metadata about
        // work, not the work: no path, no diff, no terminal byte — the same
        // ground `changes.inbox` stands on, and a read-scoped phone has to be
        // able to see what the fleet is doing to be worth carrying.
        Method::TaskList | Method::TaskGet | Method::TaskGetByKey | Method::TaskSearch => Scope::Read,
        // The board writes, at the scope `worktree.create` and
        // `terminal.create` already sit at: a write that touches no git data
        // and reveals no path.
        //
        // Not `host_admin`, because an agent has to be able to move its own
        // card and file its own findings for any of this to be automatable —
        // and `host_admin` is the scope that decides who may log in. Not
        // `read`, because `read` is what a client gets when it should only see
        // the SHAPE of the fleet, and a client that could append notes could
        // write a decision the record then claims a person made.
        Method::TaskCreate
        | Method::TaskUpdate
        | Method::TaskSetStatus
        | Method::TaskNote
        | Method::TaskBlock
        | Method::TaskSetWait
        | Method::TaskSetLine
        | Method::TaskWorker => Scope::Control,
        // The plan layer (ov-268). Reading it is metadata about work, as the
        // board reads are; a lane's worktree path is withheld below
        // `host_admin` (`rpc_plan::pb_lane`). Its writes sit with the board
        // writes, for the same reason: an orchestrator has to be able to write
        // its own plan, and they touch no git data.
        Method::PlanGet | Method::PlanEvents => Scope::Read,
        Method::PlanSet
        | Method::BoardThemeCreate
        | Method::BoardThemeUpdate
        | Method::BoardThemeCards
        | Method::LaneCreate
        | Method::LaneUpdate
        | Method::LaneCards
        | Method::LaneAgent
        | Method::RulingAdd
        | Method::RulingSet
        | Method::TrainStart
        | Method::TrainSet => Scope::Control,
        // Orchestrator pages (ov-269): free text the runner can't redact, so
        // everyone who can read the board reads them, as with a task note. The
        // orchestrator writes its own pages, and they touch no git data.
        Method::PageList | Method::PageGet | Method::PageStats => Scope::Read,
        Method::PageSet | Method::PageRemove => Scope::Control,
        // Workspaces: the list is the shape of the fleet, like
        // `worktree.list`; paths in it are redacted below `host_admin` by the
        // converter, as everywhere.
        Method::WorkspaceList => Scope::Read,
        // The writes sit with the board writes and `worktree.create`: they
        // touch no git data and reveal no path, and an orchestrator has to
        // be able to split its own workstream for any of this to be
        // automatable. `terminal.set_role` decides which terminal a wake-up
        // reaches, the same weight as saying you have read one. Starting an
        // orchestrator opens an agent pane, which is `terminal.create`'s
        // weight.
        Method::WorkspaceCreate
        | Method::WorkspaceRename
        | Method::WorkspaceSetPrefix
        | Method::WorkspaceSetSettings
        | Method::WorkspaceDelete
        | Method::WorkspaceStartOrchestrator
        | Method::TaskMove
        | Method::WorktreeAssign
        // Saying you have read a board sits with `terminal.seen` and for its
        // reason: it changes what this runner tells the owner, and the state
        // is shared by every device, so a read-scoped client that could write
        // it could silence Unread on the owner's others.
        | Method::WorkspaceMarkRead
        | Method::TerminalSetRole
        // A label, kept in the runner's store: the weight of `terminal.set_role`.
        | Method::TerminalRename => Scope::Control,
        // Tiling is `control`, not `host_admin`. It touches no files and stops
        // no process — the worst a wrong one does is show you the wrong pane —
        // and it has to be reachable by an agent for any of this to be
        // automatable.
        Method::LayoutSplit
        | Method::LayoutMove
        | Method::LayoutResize
        | Method::LayoutBreak
        | Method::LayoutRename
        | Method::LayoutViewport
        | Method::LayoutPreset
        | Method::LayoutCycle
        | Method::LayoutFocus
        | Method::LayoutZoom
        | Method::LayoutSwap
        | Method::LayoutGroupSelect => Scope::Control,
        Method::RepositoryRootList
        | Method::RepositoryRootAdd
        | Method::RepositoryRootRemove
        | Method::WorktreeRemove => Scope::HostAdmin,
        // Runner settings, reads included.
        //
        // These write a file in the user's home directory on a runner that may
        // not be the one asking, which is `host_admin` by the same rule paths
        // are. `adapter.list` is a READ and still belongs here: it reports
        // `program`, `args` and `env`, which is local paths and, for an agent
        // that needs one, an API key. `Scope::Read` is for the shape of the
        // fleet, not for its secrets.
        // Which devices may log in here.
        //
        // Reading is `read`: a list of enrolled devices is the shape of the
        // fleet, carries no path and no secret — a public key's fingerprint is
        // published by design — and a phone that can see the fleet should be
        // able to see who else can.
        //
        // Enrolling and revoking are `host_admin` for a stronger reason than
        // the settings writes below. They do not merely write a file in the
        // user's home directory; they decide who may log in to this runner at
        // all. A client that could enroll could widen its own access, which
        // would make every scope beneath this one advisory.
        Method::ClientList => Scope::Read,
        Method::ClientEnroll | Method::ClientRevoke => Scope::HostAdmin,
        // Registering a node key is `read`, and that is not an oversight.
        //
        // It writes into `authorized_keys`, which everything else in this
        // paragraph is `host_admin` for — but what it writes is a route onto
        // the CALLER'S OWN line, chosen by this runner's file rather than by
        // the request, and a route to access the caller is already using to
        // make the call. It widens nobody's access, least of all its own: a
        // device that could not log in cannot make this call, and a device
        // that can gains nothing it did not have. Requiring `host_admin` here
        // would mean a read-scoped phone could never migrate onto the tunnel,
        // which is the entire purpose of the method.
        Method::ClientSetNodeKey => Scope::Read,
        Method::SettingsSetBranchPrefix
        | Method::ThemeUpsert
        | Method::ThemeDelete
        | Method::AdapterList
        | Method::AdapterUpsert
        | Method::AdapterDelete
        | Method::AdapterTest => Scope::HostAdmin,
    }
}

/// Where an adapter in force came from.
///
/// A pure function of two name sets rather than a branch inside the list
/// builder, so it can be tested without a config file — which matters because
/// `config_path()` reads process-global environment and the test harness runs
/// in parallel, so a test that pointed it at a scratch file would move it out
/// from under every other test in the binary.
fn adapter_origin(
    preset: &str,
    configured: &std::collections::BTreeSet<String>,
    built_in: &std::collections::BTreeSet<String>,
) -> farcooler_protocol::v1::AdapterOrigin {
    use farcooler_protocol::v1::AdapterOrigin;
    match (configured.contains(preset), built_in.contains(preset)) {
        // A table shadowing something Far Cooler ships. Deleting it restores
        // the shipped one, which is what "revert to default" means.
        (true, true) => AdapterOrigin::Override,
        // A table for an agent Far Cooler does not ship.
        (true, false) => AdapterOrigin::User,
        // No table, so whatever is in force is what shipped.
        (false, _) => AdapterOrigin::BuiltIn,
    }
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
    /// One `Rpc` is built per request from its connection's peer, so this is
    /// that connection's answer. Nothing serves a connection with an `Rpc`
    /// directly — `RpcFactory` does — so this exists for the tests that dispatch
    /// against one without a socket in front of it.
    fn peer(&self) -> Peer {
        self.peer.clone()
    }

    async fn handle(&self, req: Request) -> Response {
        let request_id = req.request_id.clone();
        let outcome = match required_scope(&req.method) {
            // A method this daemon does not implement.
            //
            // `CapabilityUnsupported`, not `NotFound`: to a newer client asking
            // for a feature this build predates, "no such method" and "no such
            // worktree" were the same code, so it could neither dim the
            // control nor say anything a person could act on.
            None => Err(DomainError::CapabilityUnsupported {
                needed: farcooler_protocol::capability::for_method(&req.method).unwrap_or("a newer Far Cooler"),
            }),
            Some(required) if !satisfies(self.peer.scope, required) => Err(DomainError::ScopeDenied { needed: scope_name(required) }),
            // The envelope's capability precondition, checked here beside scope
            // and before any domain logic — the same rule the target id and
            // expected version follow, and for the same reason.
            //
            // This is what catches a NEW FIELD on an existing payload. An older
            // daemon drops one as an unknown proto3 field and does the old
            // thing, so the client believes it asked for something it did not
            // get. Naming the capability turns that silence into a refusal.
            Some(_) => match self.unsupported(&req.required_capabilities) {
                Some(needed) => Err(DomainError::CapabilityUnsupported { needed }),
                None => self.dispatch(req).await,
            },
        };

        match outcome {
            Ok(value) => Response {
                request_id,
                outcome: Some(response::Outcome::Result(WireResult { value: Some(value) })),
            },
            Err(err) => error_response(request_id, err),
        }
    }
}

impl Rpc {
    /// The first capability this daemon does not have, if the request names one.
    ///
    /// Returns a `&'static str` from this build's own table rather than the
    /// caller's string, so nothing a client sends is ever echoed back into an
    /// error message.
    fn unsupported(&self, required: &[String]) -> Option<&'static str> {
        required.iter().find_map(|name| {
            farcooler_protocol::capability::ALL
                .iter()
                .find(|known| *known == name)
                .is_none()
                .then_some("a newer Far Cooler")
        })
    }

    /// The envelope's target, which every single-resource mutation needs.
    fn target(req: &Request) -> Result<Uuid> {
        req.target_resource_id
            .as_deref()
            .and_then(wire::parse_id)
            .ok_or(DomainError::NotFound)
    }

    // MARK: - Runner settings helpers

    /// This runner as it is right now, for a settings write to answer with.
    ///
    /// Named for the wire type it builds, not for the word a person reads —
    /// see `wire::host` for why `Host` stays `Host` on the wire.
    async fn host_now(&self, svc: &Service) -> Result<farcooler_protocol::v1::Host> {
        svc.inventory.refresh().await;
        Ok(wire::host(
            &self.daemon_version,
            svc.host_id,
            &svc.inventory_snapshot(),
            0,
            crate::service::stand_in_agent(),
            crate::push::Pairing::load_in(svc.root_dir()).is_some(),
            svc.store.schema().ok(),
            agents_found_now().await,
            svc.read_only_folders(), self.peer.scope,
        ))
    }

    /// A config write that failed, as something a form can show.
    ///
    /// Never the raw `io::Error`. A settings screen showing "Permission denied
    /// (os error 13)" has told the user nothing about which file or what to do,
    /// and the one thing they need to know is that nothing was changed.
    fn config_write_failed(what: &str, error: std::io::Error) -> DomainError {
        tracing::warn!(%what, error = %error, "could not write config.toml");
        DomainError::OperationFailed
    }

    /// The runner's themes, in the shape every settings write answers with.
    ///
    /// Read back from the file rather than assembled from what was sent, so a
    /// client's list is what the file now says — including a color the writer
    /// normalized on the way in.
    fn runner_themes() -> farcooler_protocol::v1::ThemeList {
        let items = farcooler_core::config::load_themes()
            .into_iter()
            .map(|t| farcooler_protocol::v1::Theme {
                name: t.name,
                dark: t.dark,
                background: t.background,
                foreground: t.foreground,
                cursor: t.cursor,
                ansi: t.ansi.to_vec(),
            })
            .collect();
        farcooler_protocol::v1::ThemeList { items }
    }

    /// A wire theme, validated.
    ///
    /// Exactly sixteen ANSI colors, and a name. The reader refuses a short list
    /// rather than padding it — "a color on screen that nobody chose and nobody
    /// can find in the file" — so the writer refuses one too, before it can
    /// produce a table the reader will then silently drop.
    fn theme_from_wire(
        wire_theme: &farcooler_protocol::v1::Theme,
    ) -> Result<farcooler_core::theme::Theme> {
        let name = wire_theme.name.trim();
        farcooler_core::validate::display_name(name)?;
        if wire_theme.ansi.len() != 16 {
            return Err(DomainError::InvalidArgument { what: "ansi" });
        }
        let mut ansi = [0u32; 16];
        ansi.copy_from_slice(&wire_theme.ansi);
        Ok(farcooler_core::theme::Theme {
            name: name.to_string(),
            dark: wire_theme.dark,
            background: wire_theme.background,
            foreground: wire_theme.foreground,
            cursor: wire_theme.cursor,
            ansi,
        })
    }

    /// Every adapter the daemon would use, marked with where it came from.
    ///
    /// Built by asking the live registry what it holds and the config file what
    /// it says, then comparing — rather than by reading the file alone, which
    /// could not report a built-in, or the registry alone, which has already
    /// merged the two and forgotten which was which.
    fn adapters(svc: &Service) -> farcooler_protocol::v1::AdapterList {
        let configured: std::collections::BTreeSet<String> =
            farcooler_core::config::load_adapter_names().into_iter().collect();
        let built_in: std::collections::BTreeSet<String> = farcooler_core::activity::Registry::built_in()
            .all()
            .iter()
            .map(|r| r.preset.clone())
            .collect();

        let registry = svc.registry();
        let items = registry
            .all()
            .iter()
            .map(|rules| {
                let origin = adapter_origin(&rules.preset, &configured, &built_in);
                let spec = rules.adapter.clone().unwrap_or_default();
                farcooler_protocol::v1::Adapter {
                    preset: rules.preset.clone(),
                    program: spec.program,
                    args: spec.args,
                    env: spec.env.into_iter().collect(),
                    commands: rules.commands.clone(),
                    identity: rules.identity.clone(),
                    blocked: rules.blocked.clone(),
                    working: rules.working.clone(),
                    origin: origin as i32,
                    backend: spec.backend.to_proto() as i32,
                }
            })
            .collect();
        farcooler_protocol::v1::AdapterList { items }
    }

    /// A wire adapter as the config writer wants it.
    fn adapter_table(
        wire_adapter: &farcooler_protocol::v1::Adapter,
    ) -> farcooler_core::config::AdapterTable {
        farcooler_core::config::AdapterTable {
            backend: farcooler_core::activity::AdapterBackend::from_proto(wire_adapter.backend),
            program: wire_adapter.program.trim().to_string(),
            args: wire_adapter.args.clone(),
            env: wire_adapter.env.clone().into_iter().collect(),
            commands: wire_adapter.commands.clone(),
            identity: wire_adapter.identity.clone(),
            blocked: wire_adapter.blocked.clone(),
            working: wire_adapter.working.clone(),
        }
    }

    async fn dispatch(&self, req: Request) -> Result<result::Value> {
        let svc = &self.service;
        let scope = self.peer.scope;

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
                    crate::service::stand_in_agent(),
                    crate::push::Pairing::load_in(svc.root_dir()).is_some(),
                    svc.store.schema().ok(),
                    agents_found_now().await,
                    svc.read_only_folders(), scope,
                )))
            }

            // Stop, so a differently-built daemon can take over.
            //
            // The Mac app owns the local daemon's lifecycle: at launch it makes
            // sure the one answering this socket was built from the same source
            // as the app, and replaces it when it was not. Two components built
            // from different source behave like two different programs, and the
            // symptom is a bug you already fixed still happening.
            //
            // Scheduled rather than performed here, because this handler's
            // return value IS the reply: exiting before it is written would
            // leave every caller unable to tell "stopped" from "died". The
            // delay is the write, and nothing else depends on its length.
            //
            // Nothing is lost by stopping. Terminals are tmux windows, agent
            // shims reconnect on the next start (`resume_agent_listeners`), and
            // durable state is in SQLite, committed per call.
            "daemon.shutdown" => {
                let stop = self.stop.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    stop.notify_one();
                });
                Ok(result::Value::Empty(Empty {}))
            }

            "daemon.version" => Ok(result::Value::DaemonVersion(
                farcooler_protocol::v1::DaemonVersion {
                    daemon_version: self.daemon_version.clone(),
                    protocol_versions: vec![farcooler_protocol::PROTOCOL_VERSION],
                    // From the one table, exactly as `ServerHello` is, so the
                    // two cannot disagree about what this daemon can do.
                    capabilities: farcooler_protocol::capability::ALL
                        .iter()
                        .map(|c| (*c).to_string())
                        .collect(),
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
                    farcooler_protocol::v1::RepositoryRootList { items },
                ))
            }

            // What this runner defines in `[themes.<name>]`, read fresh.
            //
            // Read on each call rather than cached at startup, so editing the
            // file and reconnecting is enough to see the change — which is the
            // whole reason themes live in a hand-edited file. It is a few
            // hundred bytes of TOML parsed a handful of times per session, and
            // a file watcher on every runner would be a lot of machinery for
            // something that changes twice a year.
            "theme.list" => {
                let items = farcooler_core::config::load_themes()
                    .into_iter()
                    .map(|t| farcooler_protocol::v1::Theme {
                        name: t.name,
                        dark: t.dark,
                        background: t.background,
                        foreground: t.foreground,
                        cursor: t.cursor,
                        ansi: t.ansi.to_vec(),
                    })
                    .collect();
                Ok(result::Value::ThemeList(farcooler_protocol::v1::ThemeList { items }))
            }

            // MARK: - Runner settings
            //
            // Editing what `config.toml` holds, from a settings screen instead
            // of an ssh session and a text editor.
            //
            // Every write goes through `farcooler_core::config`, which is
            // format-preserving and atomic and refuses a malformed file — see
            // the module's own comment on why that matters for a file a
            // dotfiles repository tracks. Nothing here rewrites the whole
            // document, so a hand edit to another table survives a write here.
            //
            // `config_path()` rather than a path from the request: a client
            // naming the file it wanted written would be a client that could
            // name any file.
            "settings.set_branch_prefix" => {
                let Some(request::Payload::HostSettings(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let path =
                    farcooler_core::config::config_path().ok_or(DomainError::OperationFailed)?;
                farcooler_core::config::write_branch_prefix(&path, &p.branch_prefix)
                    .map_err(|e| Self::config_write_failed("the branch prefix", e))?;
                // Read back rather than echoing what was sent: the writer trims,
                // so what the file now says is not always what arrived.
                Ok(result::Value::Host(self.host_now(svc).await?))
            }

            "theme.upsert" => {
                let Some(request::Payload::Theme(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let theme = Self::theme_from_wire(&p)?;
                let path =
                    farcooler_core::config::config_path().ok_or(DomainError::OperationFailed)?;
                farcooler_core::config::write_theme(&path, &theme)
                    .map_err(|e| Self::config_write_failed("the theme", e))?;
                Ok(result::Value::ThemeList(Self::runner_themes()))
            }

            "theme.delete" => {
                // `TypedConfirmation` carries the NAME, not a confirmation of
                // intent: deleting a theme touches no files and is undone by
                // saving it again, so it needs no typed gate.
                let Some(request::Payload::TypedConfirmation(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let path =
                    farcooler_core::config::config_path().ok_or(DomainError::OperationFailed)?;
                farcooler_core::config::delete_theme(&path, p.typed_confirmation.trim())
                    .map_err(|e| Self::config_write_failed("the theme", e))?;
                Ok(result::Value::ThemeList(Self::runner_themes()))
            }

            "adapter.list" => Ok(result::Value::AdapterList(Self::adapters(svc))),

            "adapter.upsert" => {
                let Some(request::Payload::Adapter(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let preset = p.preset.trim().to_string();
                farcooler_core::validate::command_preset(&preset)?;
                // The same guard `Registry::merge` applies when reading, applied
                // before writing: an adapter with no program cannot start, and a
                // table for one would offer a chat toggle that silently fails.
                if p.program.trim().is_empty() {
                    return Err(DomainError::InvalidArgument { what: "program" });
                }
                let path =
                    farcooler_core::config::config_path().ok_or(DomainError::OperationFailed)?;
                farcooler_core::config::write_adapter(&path, &preset, &Self::adapter_table(&p))
                    .map_err(|e| Self::config_write_failed("the adapter", e))?;
                // The one place the registry is not read per call, so the one
                // place it has to be told.
                svc.reload_registry();
                Ok(result::Value::AdapterList(Self::adapters(svc)))
            }

            "adapter.delete" => {
                let Some(request::Payload::TypedConfirmation(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let path =
                    farcooler_core::config::config_path().ok_or(DomainError::OperationFailed)?;
                farcooler_core::config::delete_adapter(&path, p.typed_confirmation.trim())
                    .map_err(|e| Self::config_write_failed("the adapter", e))?;
                svc.reload_registry();
                Ok(result::Value::AdapterList(Self::adapters(svc)))
            }

            "adapter.test" => {
                let Some(request::Payload::Adapter(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                // Tests what the CLIENT is holding, not what is saved. The point
                // is to answer "will this work" before committing it to the
                // file, so an unsaved form is exactly the input this wants.
                //
                // Blocking, on a blocking pool: a cold `npx` fetches a package
                // on first use and the bound is 90 seconds, which is far too
                // long to hold a runtime worker.
                let spec = farcooler_core::activity::AdapterSpec {
                    // From the client, so Test exercises the protocol the form
                    // is actually configured for. This used to be hardcoded to
                    // ACP because the wire had no field for it, which meant a
                    // native adapter was reported working by a button that had
                    // only ever spoken ACP to it.
                    backend: farcooler_core::activity::AdapterBackend::from_proto(p.backend),
                    program: p.program.trim().to_string(),
                    args: p.args.clone(),
                    env: p.env.clone().into_iter().collect(),
                };
                // The preset chooses WHICH native protocol, when the backend is
                // native — codex speaks app-server, claude speaks stream-json,
                // and they share nothing. It was already on the wire.
                let preset = p.preset.trim().to_string();
                let outcome = tokio::task::spawn_blocking(move || {
                    farcooler_agent::dispatch::handshake(
                        &preset,
                        &spec,
                        farcooler_agent::dispatch::HANDSHAKE_TIMEOUT,
                    )
                })
                .await
                .map_err(|_| DomainError::OperationFailed)?;

                Ok(result::Value::AdapterTestResult(match outcome {
                    Ok(shake) => farcooler_protocol::v1::AdapterTestResult {
                        ok: true,
                        reported: shake,
                        failure: String::new(),
                    },
                    // The adapter's own words, not "the test failed": the
                    // message is the only clue about which field is wrong, and
                    // it is going straight into a form.
                    Err(failure) => farcooler_protocol::v1::AdapterTestResult {
                        ok: false,
                        reported: String::new(),
                        failure,
                    },
                }))
            }

            // MARK: - Device enrollment
            //
            // Every one of these reads or writes this runner's own
            // `~/.ssh/authorized_keys`, through `fence` and nothing else. The
            // rules live in `enrollment`, so a second transport cannot acquire
            // a different idea of what may be written into that file.
            "client.list" => Ok(result::Value::ClientList(crate::enrollment::list(svc).await?)),

            "client.enroll" => {
                let Some(request::Payload::ClientEnroll(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                // Who granted access to whom, in this runner's log, and WHICH OF
                // THE TWO KEYS.
                //
                // The audit entry the spec asks for is on the wire in
                // `EnrolledClient`; this is the other half of it, and it is the
                // half that says which device performed the ceremony rather
                // than merely which one was enrolled. Both are device ids, both
                // are already written in `authorized_keys` in plain text.
                //
                // The shape is here because a plain line is a shell on this
                // account, and a `host_admin` client could always have got one
                // by driving a terminal — see `fence::Grant`. What this call
                // adds over that route is that the runner records it and
                // manages the line, so the record has to say what was granted.
                tracing::info!(
                    by = self.who(),
                    client = %p.client_id,
                    shell = p.shell_access,
                    "enrolling a device"
                );
                Ok(result::Value::ClientEnroll(crate::enrollment::enroll(svc, &p).await?))
            }

            // Removes the line AND closes what the line let in — see
            // `enrollment::revoke` for exactly what that does and does not
            // contain.
            "client.revoke" => {
                let Some(request::Payload::ClientRevoke(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                tracing::info!(by = self.who(), client = %p.client_id, "revoking a device");
                Ok(result::Value::ClientList(crate::enrollment::revoke(svc, &p).await?))
            }

            // A device registers its own node key over the access it already
            // holds.
            //
            // `&self.peer` and NEVER `p.client_id`. The peer is this
            // connection's identity as sshd proved it and this runner's own
            // `authorized_keys` names it; `p.client_id` is a string the caller
            // chose. Passing the latter — or building a `Peer` out of it here —
            // would let any enrolled device add a tunnel route to any other
            // device's line, which is the one thing this method must never do.
            // `enrollment::set_node_key` reads the peer and ignores the
            // request's field, and it can only do that if this arm hands it the
            // connection's own.
            "client.set_node_key" => {
                let Some(request::Payload::ClientSetNodeKey(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                // Both ids, deliberately: `by` is the line that will be
                // written and `client` is the line the caller believed it was
                // writing. When they differ, the log is the only place that
                // shows a client got this wrong.
                tracing::info!(
                    by = self.who(),
                    client = %p.client_id,
                    "registering a device's node key"
                );
                Ok(result::Value::ClientSetNodeKey(
                    crate::enrollment::set_node_key(svc, &self.peer, &p).await?,
                ))
            }

            "repository.list" => {
                let items =
                    svc.list_repositories()?.iter().map(|r| wire::repository(r, scope)).collect();
                Ok(result::Value::RepositoryList(farcooler_protocol::v1::RepositoryList { items }))
            }

            "worktree.list" => {
                let items =
                    svc.fleet().await?.iter().map(|view| wire::worktree(view, scope)).collect();
                Ok(result::Value::WorktreeList(farcooler_protocol::v1::WorktreeList { items }))
            }

            "terminal.list" => {
                // An absent target lists every terminal; a present one filters
                // to that worktree.
                let filter = req.target_resource_id.as_deref().and_then(wire::parse_id);
                let mut items = Vec::new();
                for view in svc.fleet().await? {
                    if filter.is_some_and(|id| id != view.worktree.id) {
                        continue;
                    }
                    for terminal in &view.terminals {
                        items.push(self.with_activity(terminal).await);
                    }
                }
                // The fleet trace: every ring added together at one width.
                //
                // Added here rather than left for the client because rows do
                // not share a width — each snaps to the shortest window holding
                // its own activity — so adding one row's bucket 4 to another's
                // would add two different spans of time. Only the daemon holds
                // every ring.
                //
                // Sent on the whole-fleet listing only. A filtered list is one
                // worktree, and a fleet figure beside it would be a number
                // about terminals the reply does not contain.
                let (fleet_trace, fleet_trace_anchor) =
                    if filter.is_none() { self.watcher.fleet_trace() } else { (Vec::new(), None) };
                Ok(result::Value::TerminalList(farcooler_protocol::v1::TerminalList {
                    items,
                    fleet_trace: fleet_trace.into(),
                    fleet_trace_anchor,
                }))
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
                // Registering adopts every worktree the repository already
                // has, which changes the fleet without touching git — so the
                // reconciler's mtime gate never fires for this. Without an
                // explicit announce, other connected clients would not learn
                // about the new worktrees until this repository's next
                // RECONCILE_BACKSTOP_MS pass (5 min).
                self.watcher.announce_fleet_changed();
                Ok(result::Value::Repository(wire::repository(&repo, scope)))
            }

            "branch.list" => {
                let repository = Self::target(&req)?;
                let items = svc
                    .list_branches(repository)
                    .await?
                    .into_iter()
                    .map(|b| farcooler_protocol::v1::Branch {
                        name: b.name,
                        local: b.local,
                        remote: b.remote,
                        checked_out: b.checked_out,
                        updated_at: Some(wire::timestamp(b.updated_at * 1000)),
                        subject: b.subject,
                    })
                    .collect();
                Ok(result::Value::BranchList(farcooler_protocol::v1::BranchList { items }))
            }

            "worktree.discover" => {
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
                        farcooler_protocol::v1::DiscoveredWorktree {
                            path: w.path.clone(),
                            branch: w.branch,
                            head: w.head,
                            suggested_name: name,
                            locked: w.locked,
                        }
                    })
                    .collect();
                Ok(result::Value::DiscoveredWorktreeList(farcooler_protocol::v1::DiscoveredWorktreeList {
                    items,
                }))
            }

            "worktree.create" => {
                let repository = Self::target(&req)?;
                let Some(request::Payload::WorktreeCreate(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                // `task_name` keeps its name on the wire and changes meaning:
                // it is the worktree's name now, not a description of the work.
                // Renaming the field would have made every shipped client fail
                // to create a worktree against a new daemon, to say the same
                // thing in different words.
                //
                // Adoption ignores it outright. A worktree taken over for a
                // branch that already exists is named after that branch, so
                // there is nothing for a caller to choose.
                // The workspace to claim it for, explicitly, when the caller
                // names one. Unreadable is refused rather than dropped:
                // quietly making an unclaimed worktree would leave the caller
                // believing it had claimed one.
                let workspace = match p.workspace_id.as_deref() {
                    None => None,
                    Some(raw) => Some(wire::parse_id(raw).ok_or(DomainError::NotFound)?),
                };
                let ws = if p.adopt_existing {
                    svc.adopt_branch(repository, &p.branch, workspace).await?
                } else {
                    svc.create_worktree_with(
                        repository,
                        &p.task_name,
                        &p.branch,
                        &p.base_revision,
                        p.fork_only,
                        workspace,
                    )
                    .await?
                };
                // A worktree with nothing running in it is a directory.
                //
                // Done here rather than as a second call from each client
                // because it is a product rule, not a client preference — and
                // a rule implemented three times is a rule three clients can
                // disagree about. The apps ask for `shell`; a caller about to
                // start its own agent terminal asks for nothing.
                //
                // A terminal that fails to start is LOGGED, not fatal. The
                // worktree exists and is useful, and failing the call would
                // report an error for a worktree that was in fact created —
                // which sends someone looking for a worktree that is already
                // there. The view returned below shows no terminals, so the
                // failure is visible without being reported as the wrong one.
                //
                // The title is the preset, which is the convention the CLI's
                // own `terminal create` already follows. An empty title would
                // be refused by `validate::display_name`.
                let preset = p.terminal_preset.trim();
                if !preset.is_empty() {
                    if let Err(e) = svc.create_terminal(ws.id, preset, preset).await {
                        tracing::warn!(
                            worktree_id = %ws.id,
                            preset = %preset,
                            error = ?e,
                            "the worktree was created but its terminal was not"
                        );
                    }
                }
                // The mutation writes the worktree row itself, so the
                // reconcile pass that follows finds nothing to adopt — the
                // fleet already matches git by the time it runs, which makes
                // `Outcome::is_quiet()` true and skips its own broadcast.
                // Hoisted above `worktree_view` so a transient store error
                // there does not cost the announce too: nothing else would
                // ever raise it.
                self.watcher.announce_fleet_changed();
                let view = svc.worktree_view(&ws).await?;
                Ok(result::Value::Worktree(wire::worktree(&view, scope)))
            }

            "worktree.hide" => {
                let ws = svc.hide_worktree(Self::target(&req)?).await?;
                // Hoisted above `worktree_view`: hiding changes the fleet
                // without touching git, so the reconciler's mtime gate never
                // sees it, and a transient store error from `worktree_view`
                // must not cost the announce too — nothing else will ever
                // raise it.
                self.watcher.announce_fleet_changed();
                let view = svc.worktree_view(&ws).await?;
                Ok(result::Value::Worktree(wire::worktree(&view, scope)))
            }

            // Try again to download the large files a worktree still holds as
            // pointers (ov-199). `control`, like the reorder below it: it
            // writes files only into the worktree's own, and reveals no path.
            "worktree.hydrate_lfs" => {
                let ws = svc.hydrate_lfs(Self::target(&req)?).await?;
                // The count moved without git's help, so the reconciler's gate
                // never sees it; announce so every device's notice follows.
                self.watcher.announce_fleet_changed();
                let view = svc.worktree_view(&ws).await?;
                Ok(result::Value::Worktree(wire::worktree(&view, scope)))
            }

            "worktree.reorder" => {
                let Some(request::Payload::WorktreeReorder(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let mut ids = Vec::with_capacity(p.worktree_ids.len());
                for raw in &p.worktree_ids {
                    // A client that sent something that is not a uuid is
                    // refused whole. Skipping the bad one would silently
                    // reorder around it and hand back a layout nobody asked
                    // for, which is worse than an error a client can retry.
                    ids.push(wire::parse_id(raw).ok_or(DomainError::InvalidArgument {
                        what: "worktree_ids",
                    })?);
                }
                svc.reorder_worktrees(&ids).await?;
                // Reordering never touches git, so the reconciler's mtime gate
                // never sees it — the same reasoning `worktree.hide` gives.
                // Without this announce, every OTHER connected client keeps
                // drawing the old order until the next backstop pass, which is
                // five minutes of a phone and a Mac disagreeing about where a
                // card is.
                self.watcher.announce_fleet_changed();
                Ok(result::Value::Empty(farcooler_protocol::v1::Empty {}))
            }

            "worktree.unhide" => {
                let ws = svc.unhide_worktree(Self::target(&req)?).await?;
                // Same reasoning as `worktree.hide`: unhiding never touches
                // git either, so this is the only signal other clients get
                // before the next backstop tick, and it must not depend on
                // `worktree_view` succeeding.
                self.watcher.announce_fleet_changed();
                let view = svc.worktree_view(&ws).await?;
                Ok(result::Value::Worktree(wire::worktree(&view, scope)))
            }

            "repository_root.remove" => {
                let id = Self::target(&req)?;
                // Removing a root revokes Far Cooler's permission to operate
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

            "worktree.remove" => {
                let id = Self::target(&req)?;
                let ws = svc
                    .list_worktrees()?
                    .into_iter()
                    .find(|w| w.id == id)
                    .ok_or(DomainError::NotFound)?;
                // Checked HERE rather than in the client, because a client that
                // skips the dialog must still be refused. Demanded only for a
                // dirty worktree: everything committed lives in the branch,
                // which this never touches.
                if svc.removal_needs_confirmation(id).await? {
                    let Some(request::Payload::TypedConfirmation(p)) = req.payload else {
                        return Err(DomainError::ConfirmationRequired);
                    };
                    if p.typed_confirmation.trim() != ws.name() {
                        return Err(DomainError::ConfirmationRequired);
                    }
                }
                svc.remove_worktree(id).await?;
                // Same reasoning as `worktree.create`: this mutation writes
                // the worktree row itself, so the reconcile pass that
                // follows finds nothing gone and stays quiet. Without this,
                // other connected clients would not learn the worktree is
                // gone until this repository's next RECONCILE_BACKSTOP_MS
                // pass (5 min), if ever.
                self.watcher.announce_fleet_changed();
                let view = svc.worktree_view(&ws).await?;
                Ok(result::Value::Worktree(wire::worktree(&view, scope)))
            }

            "terminal.create" => {
                let worktree = Self::target(&req)?;
                let Some(request::Payload::TerminalCreate(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                // Before anything is made, so a key that isn't on this
                // worktree's board opens no pane and writes no record.
                let task = match p.task_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
                    Some(key) => Some(svc.task_on_worktree_board(worktree, key)?),
                    None => None,
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
                    // The active layout among the worktree's own, so a new
                    // pane never joins an orchestrator's window.
                    let anchor = svc.active_layout(worktree).await.ok().flatten().and_then(|view| {
                        let pane =
                            view.panes.iter().find(|pane| pane.pane_active).or(view.panes.first())?;
                        Some(pane.terminal_id)
                    });
                    if let Some(anchor) = anchor {
                        let term = svc
                            .split_terminal_with_prompt(
                                worktree,
                                anchor,
                                farcooler_protocol::v1::SplitSide::Right,
                                &p.title,
                                &p.command_preset,
                                p.prompt.as_deref(),
                                task,
                            )
                            .await?;
                        return self.terminal_result(term.id).await;
                    }
                }
                let term = svc
                    .create_terminal_with_prompt(
                        worktree,
                        &p.title,
                        &p.command_preset,
                        p.prompt.as_deref(),
                        task,
                    )
                    .await?;
                // A new terminal is a new tmux window, which IS a new layout —
                // so the worktree's set of layouts just changed and every
                // watcher has to be told.
                //
                // Clients read layouts once at startup and rely on events for
                // everything after, so without this the tab simply does not
                // appear. It shows up minutes later when some unrelated action
                // happens to refresh, which reads as the pane arriving nowhere
                // and then teleporting into a tab.
                if let Ok(groups) = svc.layout(worktree).await {
                    self.watcher.publish_layout(worktree, &groups);
                }
                self.terminal_result(term.id).await
            }

            // A screen, for clients that cannot read tmux themselves.
            //
            // The CLI and the Mac app go straight to tmux because they are on the
            // runner; a phone over ssh cannot, so without this it can list
            // terminals and act on them but never show one.
            "terminal.screen" => {
                let id = Self::target(&req)?;
                let (known, history_lines) = match req.payload {
                    Some(request::Payload::TerminalScreenRequest(p)) => {
                        (p.known_revision, p.history_lines)
                    }
                    _ => (0, 0),
                };

                // Only when it was asked for. The scrollback is a second
                // `capture-pane` on the runner and a second capture on the wire,
                // and a client showing a pane polls this several times a second
                // — so a poll that wants only the screen has to cost exactly
                // what it cost before scrollback existed. `Runtime::history`
                // spawns nothing at zero.
                let history = bytes::Bytes::from(svc.history(id, history_lines).await?);

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
                        farcooler_protocol::v1::TerminalScreen {
                            contents: bytes::Bytes::new(),
                            columns,
                            rows,
                            cursor_column,
                            cursor_row,
                            revision,
                            unchanged: true,
                            modes: modes.clone(),
                            // Still sent, unlike the contents. A client asks
                            // for scrollback because it has none, and "your
                            // screen has not moved" is not an answer to "give
                            // me your history" — withheld here, a client whose
                            // screen happened to be current would have to dirty
                            // the pane to be allowed to scroll.
                            history,
                        },
                    ));
                }

                Ok(result::Value::TerminalScreen(farcooler_protocol::v1::TerminalScreen {
                    contents: bytes::Bytes::from(contents.into_bytes()),
                    columns,
                    rows,
                    cursor_column,
                    cursor_row,
                    revision,
                    unchanged: false,
                    modes,
                    history,
                }))
            }

            // Attach this connection to a pane's live bytes.
            //
            // Answers, then streams: the result goes back through
            // `serve_connection`'s request arm, and the frames this spawns are
            // only drained on a later turn of that loop, so the client is told
            // the attachment exists before the first byte of it arrives.
            //
            // The pane is resolved HERE and the four captures the replay costs
            // are left to the task, which is what keeps a terminal that is not
            // running an error a client can act on rather than an attachment
            // that opens and immediately ends — while still answering in the
            // time an in-memory lookup takes.
            "terminal.attach" => {
                let id = Self::target(&req)?;
                let pane = svc.runtime().live_pane(id)?;
                let (columns, rows) = (pane.columns, pane.rows);
                // Zero for a pane the daemon has no record of, which a runtime
                // handle can legitimately see: the epoch says which RUN this is,
                // and "the one I have" is a better answer than refusing to
                // stream a pane that is plainly alive.
                let epoch = svc.terminal_epoch(id).unwrap_or(0);

                let service = self.service.clone();
                let sink = crate::resync::TerminalSink::new(self.push.clone(), id, epoch);
                let task = tokio::spawn(async move {
                    let _ = service.runtime().attach(pane, sink).await;
                });

                // One attachment per connection, so this ends whatever the last
                // one was streaming. Two panes' bytes interleaved on one
                // connection would be two streams a client has to demultiplex
                // for no gain — and the client that attaches gives every
                // attachment a session of its own anyway.
                // The previous one is dropped here, and `Attachment`'s own
                // `Drop` is what ends it — the same line that ends this one when
                // the connection goes away, rather than a second place that has
                // to remember to.
                if let Ok(mut held) = self.attachment.lock() {
                    held.replace(Attachment(task));
                }

                Ok(result::Value::TerminalAttach(
                    farcooler_protocol::v1::TerminalAttachResult {
                        epoch,
                        // Equal, which is this runner saying it retained nothing
                        // for you: the replay that follows is the whole of what
                        // there is. See `TerminalAttach.last_acked_sequence` for
                        // what it would take to answer otherwise, and why no
                        // runner does yet.
                        next_sequence: 0,
                        oldest_sequence: 0,
                        columns,
                        rows,
                        // Empty for the same reason. A client stores it and
                        // hands it back; an empty one means start over.
                        resume_token: String::new(),
                    },
                ))
            }

            "terminal.write" => {
                let id = Self::target(&req)?;
                let Some(request::Payload::TerminalWrite(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                svc.send_bytes(id, &p.payload).await?;
                self.terminal_result(id).await
            }

            // One chunk of a file on its way into a pane. The last one names
            // the file and types its path; every other one only says how much
            // has landed, which is what the sender's next `offset` must be.
            "terminal.paste_file" => {
                let id = Self::target(&req)?;
                let Some(request::Payload::TerminalFilePut(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let stored = crate::pastes::put_chunk(
                    svc.root_dir(),
                    &p.transfer_id,
                    &p.name,
                    p.total_size,
                    p.offset,
                    &p.chunk,
                )
                .await?;
                let out = match stored {
                    crate::pastes::Stored::Partial { stored } => {
                        farcooler_protocol::v1::TerminalFilePutResult { stored, path: None }
                    }
                    crate::pastes::Stored::Complete { path, stored } => {
                        let shown = path.to_string_lossy().to_string();
                        svc.paste_path(id, &shown).await?;
                        farcooler_protocol::v1::TerminalFilePutResult {
                            stored,
                            path: Some(shown),
                        }
                    }
                };
                Ok(result::Value::TerminalFilePut(out))
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
                let worktree = svc.store.get_terminal(id)?.worktree_id;
                svc.stop_terminal(id).await?;
                // Stopping kills the pane, so its window's grid just changed.
                self.announce_layout(worktree).await;
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

            // What this client currently has in front of a person, replacing
            // whatever it last said.
            //
            // The counterpart to `terminal.seen` and deliberately NOT a variant
            // of it: `seen` is a fact about the past that ends `Done`, where
            // this is a claim about the present that decides whether a
            // transition is allowed to interrupt anybody. They also fire at
            // different moments, which is the entire point — `seen` arrives on
            // the poll AFTER an agent finished, and by then the push has already
            // buzzed a wrist. See `crate::watch::attention`.
            //
            // Nothing on the runner changes, so nothing is echoed back: an
            // `Empty` rather than the terminal rows, since the rows this
            // describes are the rows the caller was already looking at when it
            // made the call. It also takes no `target_resource_id` — there are
            // several ids, a Mac window shows a whole tiled layout at once —
            // and ids it cannot parse are dropped rather than refused, because
            // a heartbeat is not a place to fail a client over one bad entry.
            //
            // Answering an unpaired or unenrolled client costs nothing and is
            // still correct: a runner nobody has paired for push sends no push
            // to suppress, and the claim expires on its own either way.
            "terminal.watching" => {
                let Some(request::Payload::TerminalsWatched(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let terminals: Vec<Uuid> =
                    p.terminal_ids.iter().filter_map(|id| wire::parse_id(id)).collect();
                self.watcher.report_watching(self.who(), terminals);
                Ok(result::Value::Empty(farcooler_protocol::v1::Empty {}))
            }

            "terminal.remove" => {
                let id = Self::target(&req)?;
                let worktree = svc.store.get_terminal(id)?.worktree_id;
                svc.remove_terminal(id).await?;
                // Removing takes an exited terminal's retained dead pane with
                // it, which is a cell leaving the window's grid.
                self.announce_layout(worktree).await;
                // No terminal to return: it is gone. An empty worktree list is
                // the honest shape for "this succeeded and there is nothing to
                // show", rather than echoing back a record that no longer
                // exists.
                Ok(result::Value::TerminalList(farcooler_protocol::v1::TerminalList {
                    items: Vec::new(),
                    fleet_trace: Default::default(),
                    fleet_trace_anchor: None,
                }))
            }

            "terminal.dismiss_lost" => {
                let id = Self::target(&req)?;
                svc.dismiss_lost(id).await?;
                // Gone, so there is no record to echo — the same shape
                // `terminal.remove` answers with, for the same reason.
                Ok(result::Value::TerminalList(farcooler_protocol::v1::TerminalList {
                    items: Vec::new(),
                    fleet_trace: Default::default(),
                    fleet_trace_anchor: None,
                }))
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
                let mode = match farcooler_protocol::v1::PaneMode::try_from(p.pane_mode) {
                    Ok(farcooler_protocol::v1::PaneMode::Agent) => models::PaneMode::Agent,
                    Ok(farcooler_protocol::v1::PaneMode::Terminal) => models::PaneMode::Terminal,
                    // A client that sends UNSPECIFIED is asking for a mode
                    // that does not exist, not for a default — guessing one
                    // would silently switch a pane nobody asked to switch.
                    _ => return Err(DomainError::InvalidArgument { what: "pane_mode" }),
                };
                svc.set_pane_mode(id, mode, p.force).await?;
                // Same reasoning as `worktree.hide`: this changes a pane
                // WITHOUT changing anything the watcher observes. Activity,
                // current command, and liveness all stay exactly as they were,
                // so the runtime poll has nothing to notice and never
                // announces — and the reply below reaches only the client that
                // asked.
                //
                // Every other client therefore kept rendering the pane in its
                // old mode until something unrelated happened to it, or until
                // a human reached for "Reload Fleet". A pane switched to agent
                // mode from the CLI stayed a terminal on screen while its
                // agent talked into a view nobody was showing.
                self.watcher.announce_fleet_changed();
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
                to_the_shim(
                    svc,
                    id,
                    DaemonMessage::Prompt {
                        text: wire::prompt_text(&p.blocks),
                        images: wire::prompt_images(&p.blocks),
                    },
                )?;
                // Typing into a pane is reading it. Waiting for the shim's first
                // event to move the row off `Done` would leave a terminal you
                // are actively using still asking for your attention.
                self.watcher.mark_seen(id).await;
                self.terminal_result(id).await
            }

            // The same payload as a prompt, typed into a TUI pane's box past
            // the answer wake's gate, else refused with nothing typed. Ask the
            // Orchestrator (ov-184) never sends it (`Watcher::draft_into`); the
            // Mac title bar's field (ov-214) submits it (`Watcher::tell_into`).
            "terminal.draft_prompt" | "terminal.tell" => {
                let Some(request::Payload::AgentPrompt(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                let text = wire::prompt_text(&p.blocks);
                if req.method == "terminal.tell" {
                    self.watcher.tell_into(id, &text).await?;
                } else {
                    self.watcher.draft_into(id, &text).await?;
                }
                self.terminal_result(id).await
            }

            // ---- review ----
            //
            // Thin on purpose: every one of these is a call into `review_ops`,
            // so the dispatch table stays a table and the logic stays testable
            // without a wire frame around it.
            "changes.change_set" => {
                let Some(request::Payload::ChangeSetRequest(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let set = crate::review_ops::change_set(svc, &p).await?;
                // The notice for large files that weren't downloaded lives on
                // Changes, so its count is made fresh here (ov-199).
                if let Some(id) = wire::parse_id(&p.worktree_id) {
                    if svc.recheck_lfs_pointers(id).await {
                        self.watcher.announce_fleet_changed();
                    }
                }
                Ok(result::Value::ChangeSet(set))
            }

            "changes.commit_files" => {
                let Some(request::Payload::CommitFilesRequest(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::FileChangeList(
                    crate::review_ops::commit_files(svc, &p).await?,
                ))
            }

            "changes.file_diff" => {
                let Some(request::Payload::FileDiffRequest(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::FileDiff(crate::review_ops::file_diff(svc, &p).await?))
            }

            "changes.set_base" => {
                let Some(request::Payload::ChangesSetBase(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::ChangeSet(crate::review_ops::set_base(svc, &p).await?))
            }

            "changes.mark_read" => {
                let Some(request::Payload::ChangesMarkRead(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                crate::review_ops::mark_read(svc, &p).await?;
                // A reviewed worktree leaves the runner's review count, which
                // the lock screen shows and no needs-you item moved for.
                self.watcher.announce_needs_you();
                Ok(result::Value::Empty(farcooler_protocol::v1::Empty {}))
            }

            "changes.inbox" => {
                Ok(result::Value::ChangesInbox(crate::review_ops::inbox(svc).await?))
            }

            "needs_you.list" => {
                let inputs = crate::needs_you::gather(svc, &self.watcher).await?;
                let mut items = crate::needs_you::assemble(&inputs, std::time::SystemTime::now());
                if !satisfies(self.peer.scope, Scope::Control) {
                    items = items.into_iter().map(crate::needs_you::redact_below_control).collect();
                }
                Ok(result::Value::NeedsYouList(farcooler_protocol::v1::NeedsYouList { items }))
            }

            "report.get" => {
                let Some(request::Payload::ReportRequest(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::Report(crate::report::serve(&svc.store, &p, crate::review::now_millis())?))
            }

            "usage.report" => {
                let Some(request::Payload::UsageReport(q)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::UsageReport(crate::usage::report(svc, &q)?))
            }

            "usage.task" => {
                let Some(request::Payload::UsageTask(q)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::TaskUsage(crate::usage::task(svc, &q)?))
            }

            "stack.get" => {
                let Some(request::Payload::StackGet(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let list = crate::review_ops::stack_get(svc, &p).await?;
                // Fill on first read, and never block the read.
                //
                // PR state is in memory and nowhere else, written until now only
                // by an explicit `pr.refresh` — which is `Scope::Control`. So a
                // read-scoped phone could DISPLAY PR state and never populate
                // it, and every client saw an empty row after a daemon restart
                // until somebody found a sheet and pressed Refresh. A status row
                // that is blank until tapped is not glanceable.
                //
                // This stays `Scope::Read`. The split in `required_scope` is
                // about who may FORCE a refresh; one bounded fill of an empty
                // cache is not that, and it is self-limiting by construction —
                // no client looking at a repository means no `stack.get`, means
                // no fetch. That is what makes it affordable where the
                // daemon-wide poll it could have been would not be.
                //
                // Spawned, never awaited. See `fill_prs_in_background`.
                if let Ok(repository_id) = Uuid::from_slice(&p.repository_id) {
                    crate::review_ops::fill_prs_in_background(
                        self.service.clone(),
                        self.watcher.clone(),
                        repository_id,
                        p.branch.clone(),
                    );
                }
                Ok(result::Value::StackLinkList(list))
            }

            "stack.set_parent" => {
                let Some(request::Payload::StackSetParent(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::StackLinkList(
                    crate::review_ops::stack_set_parent(svc, &p).await?,
                ))
            }

            "pr.refresh" => {
                let Some(request::Payload::PrRefresh(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::StackLinkList(crate::review_ops::pr_refresh(svc, &p).await?))
            }

            // ---- the board ----
            //
            // Thin for the reason the review arms above are, and one more:
            // every write here has to announce, and the announce lives at the
            // foot of the `task_ops` function rather than in this table, so a
            // route cannot be added with the call site left empty. See that
            // module's doc.
            //
            // There is no arm that edits a note, and there must not be one.
            // Current understanding is mutable and lives on the row —
            // `task.update` — while the record of how you got there is
            // append-only and lives in notes that can be superseded and never
            // edited. `task_notes` carries a `BEFORE UPDATE` trigger that
            // refuses unconditionally, so such an arm would fail on a caller's
            // data rather than at review.
            //
            // The arms live in `rpc_board`; every route is named here.
            "task.list" | "task.get" | "task.get_by_key" | "task.search" | "task.create" | "task.update"
            | "task.set_status" | "task.note" | "task.block" | "task.set_wait" | "task.set_line"
            | "task.worker" | "workspace.mark_read" => crate::rpc_board::dispatch(svc, &self.watcher, req).await,

            // The plan layer (ov-268); the arms live in `rpc_plan`.
            "plan.get" | "plan.set" | "plan.events" | "board_theme.create" | "board_theme.update"
            | "board_theme.cards" | "lane.create" | "lane.update" | "lane.cards" | "lane.agent"
            | "ruling.add" | "ruling.set" | "train.start" | "train.set" => {
                crate::rpc_plan::dispatch(svc, &self.watcher, self.peer.scope, req).await
            }

            // Orchestrator pages (ov-269); the arms live in `rpc_pages`.
            "page.list" | "page.get" | "page.set" | "page.remove" | "page.stats" => {
                crate::rpc_pages::dispatch(svc, &self.watcher, req).await
            }

            // ---- workspaces ----
            //
            // Each arm reads its payload and hands it to `workspace_ops`,
            // which announces. A create targets its parent, the repository;
            // everything else targets the resource it changes.
            "workspace.list" => {
                let repository = req.target_resource_id.as_deref().map(|raw| {
                    wire::parse_id(raw).ok_or(DomainError::NotFound)
                });
                Ok(result::Value::WorkspaceList(crate::workspace_ops::list(
                    svc,
                    repository.transpose()?,
                    scope,
                )?))
            }

            "workspace.create" => {
                let repository = Self::target(&req)?;
                let p = match req.payload {
                    Some(request::Payload::WorkspaceCreate(p)) => p,
                    // An app from before the worktree rename, asking for a
                    // WORKTREE by this method's old name. Refused as the other
                    // retired names are, rather than as a malformed payload:
                    // what it needs is `worktree.create`.
                    Some(request::Payload::WorktreeCreate(_)) => {
                        return Err(DomainError::CapabilityUnsupported { needed: "a newer Far Cooler" });
                    }
                    _ => return Err(DomainError::InvalidArgument { what: "payload" }),
                };
                Ok(result::Value::Workspace(crate::workspace_ops::create(
                    svc,
                    &self.watcher,
                    repository,
                    &p,
                    scope,
                )?))
            }

            "workspace.rename" => {
                let id = Self::target(&req)?;
                let Some(request::Payload::WorkspaceRename(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::Workspace(crate::workspace_ops::rename(svc, &self.watcher, id, &p, scope)?))
            }

            "workspace.set_prefix" => {
                let id = Self::target(&req)?;
                let Some(request::Payload::WorkspaceSetPrefix(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::Workspace(crate::workspace_ops::set_prefix(
                    svc,
                    &self.watcher,
                    id,
                    &p,
                    scope,
                )?))
            }

            "workspace.set_settings" => {
                let id = Self::target(&req)?;
                let Some(request::Payload::WorkspaceSetSettings(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::Workspace(crate::workspace_ops::set_settings(
                    svc,
                    &self.watcher,
                    id,
                    &p,
                    scope,
                )?))
            }

            "workspace.delete" => {
                let id = Self::target(&req)?;
                Ok(result::Value::Empty(crate::workspace_ops::delete(svc, &self.watcher, id)?))
            }

            "workspace.start_orchestrator" => {
                let id = Self::target(&req)?;
                let Some(request::Payload::WorkspaceStartOrchestrator(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let terminal = crate::workspace_ops::start_orchestrator(svc, &self.watcher, id, &p).await?;
                self.terminal_result(terminal).await
            }

            "task.move" => {
                let Some(request::Payload::TaskMove(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::TaskList(crate::workspace_ops::move_tasks(svc, &self.watcher, &p)?))
            }

            "worktree.assign" => {
                let worktree = Self::target(&req)?;
                let Some(request::Payload::WorktreeAssign(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::Worktree(
                    crate::workspace_ops::assign_worktree(svc, &self.watcher, worktree, &p, scope).await?,
                ))
            }

            "terminal.rename" => {
                let terminal = Self::target(&req)?;
                let Some(request::Payload::TerminalRename(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                svc.rename_terminal(terminal, &p.name)?;
                // A rename changes nothing the watcher observes, so no tick
                // would announce it: without this every other client kept the
                // old name until something unrelated moved the pane.
                self.watcher.announce_fleet_changed();
                self.terminal_result(terminal).await
            }

            "terminal.set_role" => {
                let terminal = Self::target(&req)?;
                let Some(request::Payload::TerminalSetRole(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                crate::workspace_ops::set_role(svc, &self.watcher, terminal, &p).await?;
                self.terminal_result(terminal).await
            }

            "terminal.agent_answer" => {
                let Some(request::Payload::AgentAnswer(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                if p.request_id.starts_with(crate::hook_asks::HOOK_ASK_PREFIX) {
                    // A claude TUI's hook is holding this ask, not a shim. The
                    // id says so, and a hook-shaped id nothing holds is a
                    // conflict (answered, withdrawn, or from before a
                    // restart), never a question for a shim that never saw it.
                    let label = match self.peer.client_id.as_deref() {
                        Some(client) => crate::enrollment::label_for(svc, client).await,
                        None => None,
                    };
                    let decider = decider_name(self.peer.client_id.is_none(), label.as_deref());
                    svc.hooks().answer(id, &p.request_id, &p.option_id, &decider).await.map_err(|refused| {
                        use crate::hook_asks::AnswerRefused;
                        match refused {
                            AnswerRefused::UnknownOption => DomainError::InvalidArgument { what: "option_id" },
                            // Named, so a client can say which: someone else
                            // answered, or this answer never reached the hook.
                            AnswerRefused::NotHeld => DomainError::Conflict { what: "not_held" },
                            AnswerRefused::NotDelivered => DomainError::Conflict { what: "not_delivered" },
                        }
                    })?;
                } else {
                    to_the_shim(
                        svc,
                        id,
                        DaemonMessage::Answer { request_id: p.request_id.clone(), option_id: p.option_id.clone() },
                    )?;
                    // No shim ever reports a `Resolved`: the hook path's
                    // `settle` is the only producer. Recorded here, once the
                    // answer reached the shim, so the ask leaves the
                    // transcript's open asks and the needs-you list, and every
                    // surface stops offering its buttons, as a hook ask's do.
                    svc.agents().record(
                        id,
                        vec![farcooler_agent::event::AgentEvent::Resolved { id: p.request_id, chosen: p.option_id }],
                        &|_, _| {},
                    );
                }
                // The same call `terminal.seen` makes: answering is only
                // reachable by having looked, so it ends `Done` the same way.
                // Against the WATCHER, which is the one place `Done` lives —
                // clearing it on the supervisor cleared a copy nothing reads.
                self.watcher.mark_seen(id).await;
                self.terminal_result(id).await
            }

            "terminal.agent_set_mode" => {
                let Some(request::Payload::AgentSetMode(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                to_the_shim(svc, id, DaemonMessage::SetMode { agent_mode: p.agent_mode })?;
                self.terminal_result(id).await
            }

            "terminal.agent_set_model" => {
                let Some(request::Payload::AgentSetModel(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                to_the_shim(svc, id, DaemonMessage::SetModel { model: p.model })?;
                self.terminal_result(id).await
            }

            "terminal.agent_set_config" => {
                let Some(request::Payload::AgentSetConfig(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                to_the_shim(svc, id, DaemonMessage::SetConfig { id: p.config_id, value: p.value })?;
                self.terminal_result(id).await
            }

            "terminal.agent_edit_queued" => {
                let Some(request::Payload::AgentEditQueued(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                to_the_shim(svc, id, DaemonMessage::EditQueued { id: p.queued_id, text: p.text })?;
                self.terminal_result(id).await
            }

            "terminal.agent_cancel_queued" => {
                let Some(request::Payload::AgentCancelQueued(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                to_the_shim(svc, id, DaemonMessage::CancelQueued { id: p.queued_id })?;
                self.terminal_result(id).await
            }

            "terminal.agent_steer_queued" => {
                let Some(request::Payload::AgentSteerQueued(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                to_the_shim(svc, id, DaemonMessage::SteerQueued { id: p.queued_id })?;
                self.terminal_result(id).await
            }

            "terminal.agent_cancel" => {
                let Some(request::Payload::AgentCancel(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.terminal_id).ok_or(DomainError::NotFound)?;
                to_the_shim(svc, id, DaemonMessage::Cancel)?;
                self.terminal_result(id).await
            }

            "worktree.file_search" => {
                let Some(request::Payload::WorktreeFileSearch(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                let id = wire::parse_id(&p.worktree_id).ok_or(DomainError::NotFound)?;
                let paths = svc.search_worktree_files(id, &p.query, p.limit).await?;
                Ok(result::Value::WorktreeFileList(farcooler_protocol::v1::WorktreeFileList {
                    paths,
                }))
            }

            "worktree.list_dir" => {
                let Some(request::Payload::WorktreeDir(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::WorktreeDir(crate::worktree_files::list_dir(svc, &p).await?))
            }

            "worktree.read_file" => {
                let Some(request::Payload::WorktreeFile(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::WorktreeFile(crate::worktree_files::read_file(svc, &p).await?))
            }

            // ---- tiling ----
            //
            // The worktree is always the envelope target and the group is
            // always in the payload, so every one of these reads the same two
            // things and differs only in what it does with them.
            "layout.list" => {
                let worktree = Self::target(&req)?;
                Ok(result::Value::PaneGroupList(wire::pane_group_list(
                    worktree,
                    &svc.layout(worktree).await?,
                )))
            }

            method if method.starts_with("layout.") => {
                let worktree = Self::target(&req)?;
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
                            None => svc.focused_pane(worktree, group).await?,
                        };
                        let title = if p.name.is_empty() { preset } else { p.name.as_str() };
                        svc.split_terminal(worktree, anchor, side, title, preset).await?;
                        // A split is the one layout verb that CREATES a
                        // terminal, so it is the one that changes the fleet.
                        //
                        // Only the layout was announced, and a layout carries
                        // rectangles keyed by terminal id, not the terminals
                        // themselves. So a client drew a rectangle for a pane
                        // it had no record of and left it blank until some
                        // unrelated read happened to fetch the fleet again —
                        // seconds of empty space after every `⌃B %`, every
                        // drop on an edge, and every diff opened from the
                        // toolbar. `terminal.create` has always announced its
                        // own arrival; this is the same announcement for the
                        // other way a terminal can be born.
                        self.watcher.announce_fleet_changed();
                        svc.layout(worktree).await?
                    }
                    // An existing pane moved against another, on an edge. The
                    // drag half of drag and drop, and it works across layouts.
                    "layout.move" => {
                        let [dragged] = terminals.as_slice() else {
                            return Err(DomainError::InvalidArgument { what: "one terminal" });
                        };
                        let onto = target.ok_or(DomainError::InvalidArgument { what: "target" })?;
                        svc.layout_move(worktree, *dragged, onto, side).await?
                    }
                    "layout.preset" => {
                        let preset = p
                            .preset
                            .and_then(|raw| farcooler_protocol::v1::LayoutPreset::try_from(raw).ok())
                            .unwrap_or(farcooler_protocol::v1::LayoutPreset::Tiled);
                        svc.layout_preset(worktree, group, preset).await?
                    }
                    "layout.cycle" => svc.layout_cycle(worktree, group).await?,
                    "layout.focus" => match (p.focus.as_deref().and_then(wire::parse_id), p.pane) {
                        (Some(terminal), _) => svc.layout_focus(worktree, terminal).await?,
                        (None, Some(index)) => {
                            svc.layout_focus_index(worktree, group, index as usize).await?
                        }
                        (None, None) => svc.layout_focus_step(worktree, group, step).await?,
                    },
                    "layout.zoom" => {
                        let terminal = p.zoom.as_deref().and_then(wire::parse_id);
                        svc.layout_zoom(worktree, group, terminal, p.unzoom).await?
                    }
                    "layout.swap" => {
                        let [a, b] = terminals.as_slice() else {
                            return Err(DomainError::InvalidArgument { what: "two terminals" });
                        };
                        svc.layout_swap(worktree, *a, *b).await?
                    }
                    "layout.resize" => {
                        let terminal = target.ok_or(DomainError::InvalidArgument {
                            what: "target",
                        })?;
                        svc.layout_resize(worktree, terminal, side, p.resize.unwrap_or(2)).await?
                    }
                    // Out into a layout of its own, tmux's break-pane.
                    "layout.break" => {
                        let terminal = match target.or(terminals.first().copied()) {
                            Some(id) => id,
                            None => svc.focused_pane(worktree, group).await?,
                        };
                        svc.layout_break(worktree, terminal).await?
                    }
                    "layout.rename" => svc.layout_rename(worktree, group, &p.name).await?,
                    "layout.group.select" => match group {
                        Some(id) => svc.layout_group_select(worktree, id).await?,
                        None => svc.layout_group_step(worktree, step).await?,
                    },
                    // The viewport, so tmux lays out for the size actually on
                    // screen rather than for whatever the window last had.
                    "layout.viewport" => {
                        svc.layout_resize_window(
                            worktree,
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

                self.watcher.publish_layout(worktree, &groups);
                Ok(result::Value::PaneGroupList(wire::pane_group_list(worktree, &groups)))
            }

            // `required_scope` already rejected anything not listed there, so
            // reaching here means the two lists disagree.
            other => {
                tracing::error!(method = %other, "method passed the scope table but has no handler");
                Err(DomainError::NotFound)
            }
        }
    }

    /// Tell every client a worktree's layouts as they now stand.
    ///
    /// For a mutation that changes tmux's arrangement as a side effect rather
    /// than as its purpose, like a pane being killed. When the pane was the
    /// last in its window the window is gone, and the list simply no longer
    /// holds it, which is exactly what a client needs to drop the tab.
    async fn announce_layout(&self, worktree: Uuid) {
        if let Ok(groups) = self.service.layout(worktree).await {
            self.watcher.publish_layout(worktree, &groups);
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
    ) -> farcooler_protocol::v1::Terminal {
        let mut message = wire::terminal_with_agent_state(view, self.service.agents());
        // The runner's answer to which task this agent's notices fold into,
        // from the same call `announce` makes, so a list and an event agree.
        crate::task_link::stamp_notice_task(&self.service.store, &view.terminal, &mut message);
        let (activity, state_since, turn_started_at) = self.watcher.activity(view.terminal.id).await;
        message.activity = activity as i32;
        message.activity_changed_at = state_since.map(wire::timestamp);
        message.turn_started_at = turn_started_at.map(wire::timestamp);
        message.blocked_question = self.watcher.blocked_question(view.terminal.id).await;
        if let Some(command) = self.watcher.command(view.terminal.id).await {
            message.current_command = command;
        }
        message.ports = self.watcher.ports(view.terminal.id).await;
        message.chat_capable = self.watcher.chat_capable(view.terminal.id).await;
        // The same lines the broadcast path sends, off the same `Observed`. A
        // client that reads a list and then watches events must not see the
        // feed appear, vanish, and come back.
        message.feed = self.watcher.feed(view.terminal.id).await;
        // The same message those lines were cut from, cut from its opening
        // instead, off the same `Observed` for the same reason: this is what
        // both apps' notifications quote, and a client that read it from a
        // list and then watched events must not see the sentence a banner is
        // about appear only on one of the two paths.
        message.said = self.watcher.said(view.terminal.id).await;
        // Off the same `Observed` for the same reason the feed is: a client
        // that lists terminals and then watches events must not be told a turn
        // failed by one path and that it finished cleanly by the other.
        message.turn_failed = self.watcher.turn_failed(view.terminal.id).await;
        // The agents this one spawned and has not finished with, off the same
        // `Observed` for the same reason again: a client that lists terminals
        // and then watches events must not see two subagents in the list and
        // none in the push.
        message.subagents = self.watcher.subagents(view.terminal.id).await;
        // The thirteen buckets, off the watcher's ring for the same reason as
        // everything above it: the ring is the only copy, and a client that
        // lists terminals and then watches events must not be handed two
        // different histories of one pane. See `Watcher::trace`.
        self.watcher.stamp_trace(&mut message, view.terminal.id);
        // The compact ladder, computed from everything just set above — see
        // `wire::apply_rungs` for why it has to run last, and why the signal
        // line is handed to it rather than read off the message.
        // The line and the counts behind its task-list rung go in together, so
        // a list reply cannot state a position in prose and a different one in
        // numbers. Two reads of the watcher's state, a moment apart, is the
        // most this path can do — the alternative is a lock held across the
        // whole conversion — and handing both to one function is what keeps
        // the pair from also being set in two places. See `wire::apply_rungs`.
        wire::apply_rungs(
            &mut message,
            self.watcher.signal(view.terminal.id).await.as_deref(),
            self.watcher.plan(view.terminal.id).await,
        );
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

    /// Every method on the wire has a scope, so none of them is refused as a
    /// method this runner does not know.
    ///
    /// Enumerates `Method::ALL` rather than a list of names typed here, which
    /// is what let the agent queue's three slip: they were in the protocol's
    /// table and the hand-typed list below, which checked it, never named them.
    #[test]
    fn every_method_has_a_scope() {
        let unscoped: Vec<_> = farcooler_protocol::method::Method::ALL
            .iter()
            .filter(|m| required_scope(m.name()).is_none())
            .collect();
        assert!(unscoped.is_empty(), "refused as unknown by every runner: {unscoped:?}");
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
        assert_eq!(required_scope("worktree.remove"), Some(Scope::HostAdmin));
        assert_eq!(required_scope("repository_root.remove"), Some(Scope::HostAdmin));
        // Paths live behind the same gate, so listing roots is admin too.
        assert_eq!(required_scope("repository_root.list"), Some(Scope::HostAdmin));
    }

    #[test]
    fn every_runner_setting_method_requires_host_admin() {
        // Writes, because they touch a file in the user's home directory on a
        // runner that may not be the one asking.
        for method in [
            "settings.set_branch_prefix",
            "theme.upsert",
            "theme.delete",
            "adapter.upsert",
            "adapter.delete",
            "adapter.test",
        ] {
            assert_eq!(required_scope(method), Some(Scope::HostAdmin), "{method}");
        }
        // And the READ, which is the one worth stating on its own: it reports
        // `program`, `args` and `env` — local paths, and an API key for any
        // agent that needs one. `theme.list` next to it is `read` because a
        // color is not a secret; an adapter's environment is.
        assert_eq!(required_scope("adapter.list"), Some(Scope::HostAdmin));
        assert_eq!(required_scope("theme.list"), Some(Scope::Read));
    }

    #[test]
    fn enrolling_a_device_requires_host_admin_and_looking_does_not() {
        // Stronger than the settings writes above: these decide who may log in
        // to this runner. A client that could enroll could widen its own
        // access, which would make every scope beneath this one advisory.
        assert_eq!(required_scope("client.enroll"), Some(Scope::HostAdmin));
        assert_eq!(required_scope("client.revoke"), Some(Scope::HostAdmin));
        // Reading is the shape of the fleet: no path, and a public key's
        // fingerprint is published by design.
        assert_eq!(required_scope("client.list"), Some(Scope::Read));
    }

    #[test]
    fn an_adapter_reports_where_it_came_from() {
        use farcooler_protocol::v1::AdapterOrigin;
        let set = |names: &[&str]| -> std::collections::BTreeSet<String> {
            names.iter().map(|s| s.to_string()).collect()
        };
        let built_in = set(&["claude", "codex"]);

        // No table for it: whatever is in force is what shipped.
        assert_eq!(
            adapter_origin("claude", &set(&[]), &built_in),
            AdapterOrigin::BuiltIn
        );
        // A table shadowing a shipped name. The editor offers "Revert to
        // Default" on exactly this, and reverting deletes the table.
        assert_eq!(
            adapter_origin("claude", &set(&["claude"]), &built_in),
            AdapterOrigin::Override
        );
        // A table for an agent Far Cooler does not ship — nothing to revert to.
        assert_eq!(
            adapter_origin("my-agent", &set(&["my-agent"]), &built_in),
            AdapterOrigin::User
        );
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

    // `every_method_is_dispatched_and_every_dispatched_route_is_a_method` is
    // `tests/every_method_is_dispatched.rs` (ov-171): it parses this file, so
    // it reads the arms rather than the text.

    /// The board and workspace routes are split the way the scope table's
    /// comments say they are, which no set comparison can see.
    #[test]
    fn the_board_reads_are_read_and_its_writes_are_control() {
        for method in ["task.list", "task.get", "task.get_by_key", "task.search"] {
            assert_eq!(required_scope(method), Some(Scope::Read), "{method}");
        }
        for method in
            ["task.create", "task.update", "task.set_status", "task.note", "task.block"]
        {
            assert_eq!(required_scope(method), Some(Scope::Control), "{method}");
        }
        assert_eq!(required_scope("workspace.list"), Some(Scope::Read));
        assert_eq!(required_scope("report.get"), Some(Scope::Read), "a report only reads");
        for method in [
            "workspace.create",
            "workspace.rename",
            "workspace.set_prefix",
            "workspace.set_settings",
            "workspace.delete",
            "task.move",
            "workspace.mark_read",
            "worktree.assign",
            "terminal.set_role",
            "workspace.start_orchestrator",
        ] {
            assert_eq!(required_scope(method), Some(Scope::Control), "{method}");
        }
        // The one route that must never exist. Notes are append-only: the
        // store has a `BEFORE UPDATE` trigger that refuses unconditionally, so
        // a route here would fail on a caller's data rather than at review.
        assert_eq!(required_scope("task.note_update"), None);
        assert_eq!(required_scope("task.note_edit"), None);
    }

    #[test]
    fn reads_never_require_more_than_read() {
        for method in ["host.get", "daemon.version", "worktree.list", "terminal.list"] {
            assert_eq!(required_scope(method), Some(Scope::Read), "{method}");
        }
    }
}


/// A cheap identity for a screen.
///
/// FNV-1a over the capture and the cursor. Not a checksum anyone relies on for
/// correctness — a collision means one stale frame until the next change, which
/// is a redraw, not corruption — and it is compared only against a value this
/// same runner produced moments earlier.
///
/// The scrollback is deliberately not in it. This number answers "is the screen
/// I hold still the screen?", and history is not diffed against anything: it is
/// asked for explicitly, by a client that has none, and sent whenever it is
/// asked for. Hashing it in would change the revision every time a line scrolled
/// off — resending screens a client already holds, which is the whole cost this
/// number exists to avoid.
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

/// `terminal.create` for a task, through the daemon's own handler, with the
/// agent stubbed: a pane opened for a task starts on a message telling it to
/// work the task, so these can't run a real one (`service::test_agent`). The
/// socket harness in `tests/rpc_over_socket.rs` can't reach the stub, so the
/// refusals are tested there and the openings here.
#[cfg(test)]
mod terminal_task_tests {
    use farcooler_protocol::v1::{Request, Scope, TerminalCreate, response};
    use farcooler_transport::Handler;

    use super::*;
    use crate::service::test_agent;

    async fn a_handler() -> (crate::test_support::ScratchDir, Arc<Service>, RpcFactory, models::Worktree) {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        crate::reconcile::repository(&svc, repo).await.unwrap();
        let ws = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().next().expect("the main checkout");
        let factory = RpcFactory::new(
            svc.clone(),
            crate::watch::Watcher::new(svc.clone()),
            Arc::new(tokio::sync::Notify::new()),
            Peer { client_id: None, scope: Scope::HostAdmin },
        );
        (dir, svc, factory, ws)
    }

    fn create(ws: Uuid, preset: &str, key: Option<&str>, join: bool) -> Request {
        Request {
            method: "terminal.create".into(),
            target_resource_id: Some(crate::wire::id_bytes(ws)),
            required_capabilities: vec![farcooler_protocol::capability::TERMINAL_TASK.into()],
            payload: Some(request::Payload::TerminalCreate(TerminalCreate {
                title: "worker".into(),
                command_preset: preset.into(),
                join_active_group: join,
                prompt: None,
                task_key: key.map(str::to_string),
            })),
            ..Default::default()
        }
    }

    async fn terminal(factory: &RpcFactory, req: Request) -> farcooler_protocol::v1::Terminal {
        match factory.handle(req).await.outcome {
            Some(response::Outcome::Result(r)) => match r.value {
                Some(result::Value::Terminal(t)) => t,
                other => panic!("not a terminal: {other:?}"),
            },
            other => panic!("refused: {other:?}"),
        }
    }

    /// `#{window_id} #{pane_start_command}` for every pane on the fixture's server.
    async fn panes(svc: &Service) -> Vec<String> {
        let out = svc
            .tmux
            .run(&["list-panes", "-a", "-F", "#{window_id} #{pane_start_command}"])
            .await
            .expect("tmux answered");
        out.stdout.lines().map(str::to_string).collect()
    }

    #[tokio::test]
    async fn a_terminal_for_a_task_on_its_own_board_exports_the_key() {
        let (_dir, svc, factory, ws) = a_handler().await;
        let main = svc.store.ensure_main_workspace(ws.repository_id).unwrap();
        let task = svc.store.create_task(main.id, "the work", models::Actor::User).unwrap();

        let made = terminal(&factory, create(ws.id, "claude", Some(&task.key), false)).await;
        assert_eq!(made.task_id.as_deref(), Some(task.id.as_bytes().as_slice()), "the terminal says which task");

        let panes = panes(&svc).await;
        assert!(panes.iter().all(|p| p.contains(test_agent::MARKER)), "the stub: {panes:?}");
        assert!(
            panes.iter().any(|c| c.contains(&format!("FARCOOLER_TASK={}", task.key))),
            "the pane names its task: {panes:?}"
        );
    }

    /// `join_active_group` makes the pane by splitting the focused one: the
    /// other of the two first launches, and the one `prefix %` takes.
    #[tokio::test]
    async fn a_terminal_split_into_the_layout_for_a_task_exports_the_key() {
        let (_dir, svc, factory, ws) = a_handler().await;
        let main = svc.store.ensure_main_workspace(ws.repository_id).unwrap();
        let task = svc.store.create_task(main.id, "the work", models::Actor::User).unwrap();
        // A layout to join: one ordinary pane first.
        terminal(&factory, create(ws.id, "shell", None, false)).await;

        terminal(&factory, create(ws.id, "claude", Some(&task.key), true)).await;

        let panes = panes(&svc).await;
        assert_eq!(panes.len(), 2, "{panes:?}");
        let window = |line: &str| line.split_whitespace().next().unwrap_or_default().to_string();
        assert_eq!(window(&panes[0]), window(&panes[1]), "a split, in the same window: {panes:?}");
        assert!(
            panes.iter().any(|c| c.contains(&format!("FARCOOLER_TASK={}", task.key))
                && c.contains(test_agent::MARKER)),
            "the split pane names its task, and runs the stub: {panes:?}"
        );
    }
}

#[cfg(test)]
#[path = "rpc_revision_tests.rs"]
mod revision_tests;

/// `terminal.agent_answer` for an ask a claude TUI's hook is holding
/// (`hook_asks`), rather than one an ACP shim is waiting on.
///
/// Its own `Service`, not `test_support::fixture`'s, because that one enrolls
/// into the real `~/.ssh/authorized_keys`, and these tests enroll a device.
#[cfg(test)]
mod hook_answer_tests {
    use farcooler_agent_hooks::wire::Decision;
    use farcooler_protocol::v1::{AgentAnswer, ClientEnroll, Request, Scope, TerminalIntent, response};
    use farcooler_transport::Handler;

    use super::*;

    /// A public key nobody holds the private half of.
    const PHONE_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER phone";

    struct Runner {
        _dir: tempfile::TempDir,
        svc: Arc<Service>,
        terminal: Uuid,
    }

    /// A runner with one Terminal-mode claude pane, and a phone enrolled as
    /// `phone-1` under `label`.
    async fn a_runner(label: &str) -> Runner {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let ssh = dir.path().join("home").join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
        let keys = ssh.join("authorized_keys");
        let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(keys));
        crate::enrollment::enroll(
            &svc,
            &ClientEnroll {
                public_key: PHONE_KEY.into(),
                label: label.into(),
                client_id: "phone-1".into(),
                scope: Scope::Control as i32,
                shell_access: false,
                node_key: String::new(),
            },
        )
        .await
        .expect("the phone enrolls");
        let host = Uuid::now_v7();
        let root = svc.store.create_repository_root(host, "/repos/asks", 1_000).unwrap();
        let repo = svc.store.create_repository(host, root.id, "repo", "/repos/asks/.git", "").unwrap();
        let wt = svc.store.create_worktree(repo.id, "main", "/repos/asks", true).unwrap();
        let term = svc.store.create_terminal(wt.id, "pane", "claude", TerminalIntent::Running, 80, 24).unwrap();
        Runner { _dir: dir, svc, terminal: term.id }
    }

    fn handler(svc: &Arc<Service>, client_id: Option<&str>) -> RpcFactory {
        RpcFactory::new(
            svc.clone(),
            crate::watch::Watcher::new(svc.clone()),
            Arc::new(tokio::sync::Notify::new()),
            Peer { client_id: client_id.map(str::to_string), scope: Scope::Control },
        )
    }

    fn an_answer(terminal: Uuid, request_id: &str, option_id: &str) -> Request {
        Request {
            method: "terminal.agent_answer".into(),
            payload: Some(request::Payload::AgentAnswer(AgentAnswer {
                terminal_id: crate::wire::id_bytes(terminal),
                request_id: request_id.into(),
                option_id: option_id.into(),
            })),
            ..Default::default()
        }
    }

    /// The refusal's code and `what`, or `None` for a result.
    async fn refusal(factory: &RpcFactory, req: Request) -> Option<(i32, String)> {
        match factory.handle(req).await.outcome {
            Some(response::Outcome::Result(_)) => None,
            Some(response::Outcome::Error(e)) => Some((e.code, e.what)),
            other => panic!("no outcome: {other:?}"),
        }
    }

    fn code(error: DomainError) -> (i32, String) {
        (error.wire().0 as i32, error.what().to_string())
    }

    /// Hold an ask on the runner's pane the way `serve` does, and hand back
    /// its id and what its hook is told.
    fn held(r: &Runner) -> (String, tokio::task::JoinHandle<Option<Decision>>) {
        let (id, rx) = r.svc.hooks().asks().hold(r.terminal);
        let hook = tokio::spawn(async move {
            let settled = rx.await.ok()?;
            if let Some(ack) = settled.ack {
                let _ = ack.send(());
            }
            settled.decision
        });
        (id, hook)
    }

    #[tokio::test]
    async fn an_answer_to_a_held_hook_ask_goes_to_the_hook_not_the_shim() {
        let r = a_runner("iPhone").await;
        let (id, hook) = held(&r);
        let phone = handler(&r.svc, Some("phone-1"));
        assert_eq!(refusal(&phone, an_answer(r.terminal, &id, "allow")).await, None);
        assert_eq!(hook.await.unwrap(), Some(Decision::Allow));
    }

    #[tokio::test]
    async fn a_deny_from_an_enrolled_device_is_named_by_its_label() {
        let r = a_runner("iPhone").await;
        let (id, hook) = held(&r);
        let phone = handler(&r.svc, Some("phone-1"));
        assert_eq!(refusal(&phone, an_answer(r.terminal, &id, "deny")).await, None);
        assert_eq!(hook.await.unwrap(), Some(Decision::Deny { message: "Denied from iPhone".into() }));
    }

    #[tokio::test]
    async fn a_deny_from_the_local_socket_is_named_for_the_mac() {
        let r = a_runner("iPhone").await;
        let (id, hook) = held(&r);
        let mac = handler(&r.svc, None);
        assert_eq!(refusal(&mac, an_answer(r.terminal, &id, "deny")).await, None);
        let expected = format!("Denied from {}", local_name(cfg!(target_os = "macos")));
        assert_eq!(hook.await.unwrap(), Some(Decision::Deny { message: expected }));
    }

    #[tokio::test]
    async fn a_second_answer_is_refused_as_not_held() {
        let r = a_runner("iPhone").await;
        let (id, hook) = held(&r);
        let phone = handler(&r.svc, Some("phone-1"));
        assert_eq!(refusal(&phone, an_answer(r.terminal, &id, "allow")).await, None);
        hook.await.unwrap();
        let refused = refusal(&phone, an_answer(r.terminal, &id, "deny")).await;
        assert_eq!(
            refused,
            Some((farcooler_protocol::v1::ErrorCode::ResourceConflict as i32, "not_held".to_string())),
            "the first answer won; this one is told someone already answered"
        );
    }

    /// The ask was settled, but its hook went away before it could be told:
    /// the answer never reached claude, which is a different sentence.
    #[tokio::test]
    async fn an_undelivered_answer_is_refused_as_not_delivered() {
        let r = a_runner("iPhone").await;
        let (id, rx) = r.svc.hooks().asks().hold(r.terminal);
        // A hook that hears its ending and hangs up without acknowledging it.
        let hook = tokio::spawn(async move { drop(rx.await) });
        let phone = handler(&r.svc, Some("phone-1"));
        let refused = refusal(&phone, an_answer(r.terminal, &id, "allow")).await;
        hook.await.unwrap();
        assert_eq!(
            refused,
            Some((farcooler_protocol::v1::ErrorCode::ResourceConflict as i32, "not_delivered".to_string()))
        );
    }

    #[tokio::test]
    async fn an_option_the_ask_never_offered_is_refused_by_name() {
        let r = a_runner("iPhone").await;
        let (id, _hook) = held(&r);
        let phone = handler(&r.svc, Some("phone-1"));
        assert_eq!(
            refusal(&phone, an_answer(r.terminal, &id, "allow_always")).await,
            Some(code(DomainError::InvalidArgument { what: "option_id" }))
        );
        assert!(r.svc.hooks().asks().is_holding(r.terminal), "the ask stays held");
    }

    /// A prompt to a pane whose agent stopped says so, and is not the
    /// retryable "still connecting" the apps tried again forever (ov-174).
    #[tokio::test]
    async fn a_prompt_to_a_stopped_agent_is_refused_as_stopped() {
        let r = a_runner("iPhone").await;
        r.svc.agents().stopped_for_test(r.terminal);
        let prompt = Request {
            method: "terminal.agent_prompt".into(),
            payload: Some(request::Payload::AgentPrompt(farcooler_protocol::v1::AgentPrompt {
                terminal_id: crate::wire::id_bytes(r.terminal),
                blocks: Vec::new(),
            })),
            ..Default::default()
        };
        let phone = handler(&r.svc, Some("phone-1"));
        assert_eq!(refusal(&phone, prompt).await, Some(code(DomainError::AgentStopped)));
    }

    #[test]
    fn a_device_label_is_cut_to_one_short_line() {
        assert_eq!(decider_name(false, Some("iPhone\nrm -rf /")), "iPhonerm -rf /");
        assert_eq!(decider_name(false, Some(&"x".repeat(200))).chars().count(), 40);
        assert_eq!(decider_name(false, Some("  iPad  ")), "iPad");
        assert_eq!(decider_name(false, Some(" \t ")), "a paired device");
        assert_eq!(decider_name(false, None), "a paired device");
        assert_eq!(decider_name(true, None), local_name(cfg!(target_os = "macos")));
    }

    /// "Denied from Mac" is wrong on a Linux runner.
    #[test]
    fn a_local_answer_is_named_for_the_computer_the_runner_is() {
        assert_eq!(local_name(true), "Mac");
        assert_eq!(local_name(false), "this computer");
    }
}
