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
use farcooler_protocol::v1::{
    Empty, Error as WireError, Request, Response, Result as WireResult, Scope, request, response,
    result,
};
use farcooler_store::models;
use farcooler_transport::{Handler, Peer};
use uuid::Uuid;

use crate::service::Service;
use crate::wire;

pub struct Rpc {
    service: Arc<Service>,
    watcher: Arc<crate::watch::Watcher>,
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
    pub fn new(
        service: Arc<Service>,
        watcher: Arc<crate::watch::Watcher>,
        peer: Peer,
        stop: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            service,
            watcher,
            peer,
            daemon_version: farcooler_protocol::BUILD.to_string(),
            stop,
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
        Self { service, watcher, stop, peer, session }
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

    fn closed(&self) -> impl std::future::Future<Output = ()> + Send {
        let session = self.session.clone();
        async move { session.closed().await }
    }
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
        })),
    }
}

/// The scope each method requires.
///
/// Exhaustive by construction: an unknown method is rejected rather than
/// defaulted, so adding a handler arm without adding a row here makes the
/// method unreachable instead of silently unguarded.
fn required_scope(method: &str) -> Option<Scope> {
    Some(match method {
        // Themes are what a client paints with. Read, not Control: naming a
        // color changes nothing on the runner, and a phone connected in a
        // read-only capacity should still be able to render itself properly.
        "host.get" | "host.health" | "daemon.version" | "theme.list" => Scope::Read,
        // Stopping the daemon stops nothing a user is watching — terminals are
        // tmux's — but it is the one method that ends the process, so it sits
        // at the highest scope. A local caller already holds it; a remote one
        // gets it only where ssh has proved who they are.
        "daemon.shutdown" => Scope::HostAdmin,
        "repository.list" | "workspace.list" | "terminal.list" | "branch.list" => Scope::Read,
        "layout.list" => Scope::Read,
        // Discovery reveals paths, which live behind the same gate as every
        // other path in this protocol.
        "worktree.list" => Scope::HostAdmin,
        "repository.register"
        | "workspace.create"
        | "workspace.hide"
        | "workspace.unhide"
        | "terminal.create"
        | "terminal.resize"
        | "terminal.stop"
        | "terminal.dismiss_lost"
        | "terminal.restart"
        | "terminal.seen"
        // Saying what is on your screen sits at the same scope as saying you
        // have read it, and for the same reason: both change what this runner
        // tells the owner. `read` is the scope handed to something that should
        // only see the SHAPE of the fleet, and a read-scoped client that could
        // assert attention could hold a terminal silent — which is a way of
        // withholding a notification, not a way of looking at one.
        | "terminal.watching"
        | "terminal.remove"
        // Reading a screen is `control`, not `read`.
        //
        // A screen is the most sensitive thing this protocol carries — it is
        // whatever the agent has on it, which routinely includes source, paths
        // and tokens — and `read` is the scope handed to something that should
        // only see the shape of the fleet.
        | "terminal.screen"
        // Pasting a file writes bytes to a pane and a file to the runner.
        //
        // The bytes are the same privilege as `terminal.write`, and so is the
        // file: anything that can type into a shell can already create a file
        // of its choosing. That is why this accepts any type rather than only
        // images — the restriction protected nothing and cost the case people
        // actually want, which is dropping a PDF or a log on a pane.
        | "terminal.paste_file"
        | "terminal.write" => Scope::Control,
        // A pane's agent channel is exactly as sensitive as its screen — it is
        // the same conversation, just structured — so it sits at the same
        // scope rather than behind `host_admin`. Search returns
        // worktree-relative paths only, never a runner path, so it belongs here
        // too rather than beside `worktree.list`.
        "terminal.set_pane_mode"
        | "terminal.agent_subscribe"
        | "terminal.agent_prompt"
        | "terminal.agent_answer"
        | "terminal.agent_set_mode" | "terminal.agent_set_model" | "terminal.agent_set_config"
        | "terminal.agent_cancel"
        | "worktree.file_search" => Scope::Control,
        // Review is `control`, and for exactly the reason the screen above is.
        //
        // A diff IS source. `read` is the scope handed to something that should
        // only see the shape of the fleet, and serving file content there would
        // quietly redefine what every already-enrolled read-only client is
        // allowed to see — a change of security posture made as a side effect of
        // adding a feature. `Scope` is runner-wide (there is no per-repository
        // authorization to reach for), so the honest answer is the scope that
        // can already read a terminal screen, which already shows source.
        "changes.change_set"
        | "changes.commit_files"
        | "changes.file_diff"
        | "changes.set_base"
        | "changes.mark_read"
        | "stack.set_parent"
        | "pr.refresh" => Scope::Control,
        // Metadata about work, not the work. Counts, +/-, PR state and the
        // needs-you badge let a read-scoped phone triage the fleet without being
        // able to read a line of the code.
        "changes.inbox" | "stack.get" => Scope::Read,
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
        | "workspace.remove_worktree" => Scope::HostAdmin,
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
        "client.list" => Scope::Read,
        "client.enroll" | "client.revoke" => Scope::HostAdmin,
        "settings.set_branch_prefix"
        | "theme.upsert"
        | "theme.delete"
        | "adapter.list"
        | "adapter.upsert"
        | "adapter.delete"
        | "adapter.test" => Scope::HostAdmin,
        _ => return None,
    })
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
            // workspace" were the same code, so it could neither dim the
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
        Ok(wire::host(&self.daemon_version, svc.host_id, &svc.inventory_snapshot(), 0))
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
    /// Exactly sixteen ANSI colours, and a name. The reader refuses a short list
    /// rather than padding it — "a colour on screen that nobody chose and nobody
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

            "repository.list" => {
                let items =
                    svc.list_repositories()?.iter().map(|r| wire::repository(r, scope)).collect();
                Ok(result::Value::RepositoryList(farcooler_protocol::v1::RepositoryList { items }))
            }

            "workspace.list" => {
                let items =
                    svc.fleet().await?.iter().map(|view| wire::workspace(view, scope)).collect();
                Ok(result::Value::WorkspaceList(farcooler_protocol::v1::WorkspaceList { items }))
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
                Ok(result::Value::TerminalList(farcooler_protocol::v1::TerminalList { items }))
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
                // about the new workspaces until the next RECONCILE_BACKSTOP
                // tick (30s).
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
                        farcooler_protocol::v1::ExistingWorktree {
                            path: w.path.clone(),
                            branch: w.branch,
                            head: w.head,
                            suggested_name: name,
                            locked: w.locked,
                        }
                    })
                    .collect();
                Ok(result::Value::WorktreeList(farcooler_protocol::v1::WorktreeList { items }))
            }

            "workspace.create" => {
                let repository = Self::target(&req)?;
                let Some(request::Payload::WorkspaceCreate(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                // `task_name` keeps its name on the wire and changes meaning:
                // it is the worktree's name now, not a description of the work.
                // Renaming the field would have made every shipped client fail
                // to create a workspace against a new daemon, to say the same
                // thing in different words.
                //
                // Adoption ignores it outright. A worktree taken over for a
                // branch that already exists is named after that branch, so
                // there is nothing for a caller to choose.
                let ws = if p.adopt_existing {
                    svc.adopt_branch(repository, &p.branch).await?
                } else {
                    svc.create_workspace(repository, &p.task_name, &p.branch, &p.base_revision)
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
                // report an error for a workspace that was in fact created —
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
                            workspace = %ws.id,
                            preset = %preset,
                            error = ?e,
                            "the worktree was created but its terminal was not"
                        );
                    }
                }
                // The mutation writes the workspace row itself, so the
                // reconcile pass that follows finds nothing to adopt — the
                // fleet already matches git by the time it runs, which makes
                // `Outcome::is_quiet()` true and skips its own broadcast.
                // Hoisted above `workspace_view` so a transient store error
                // there does not cost the announce too: nothing else would
                // ever raise it.
                self.watcher.announce_fleet_changed();
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }

            "workspace.hide" => {
                let ws = svc.hide_workspace(Self::target(&req)?).await?;
                // Hoisted above `workspace_view`: hiding changes the fleet
                // without touching git, so the reconciler's mtime gate never
                // sees it, and a transient store error from `workspace_view`
                // must not cost the announce too — nothing else will ever
                // raise it.
                self.watcher.announce_fleet_changed();
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
            }

            "workspace.unhide" => {
                let ws = svc.unhide_workspace(Self::target(&req)?).await?;
                // Same reasoning as `workspace.hide`: unhiding never touches
                // git either, so this is the only signal other clients get
                // before the next backstop tick, and it must not depend on
                // `workspace_view` succeeding.
                self.watcher.announce_fleet_changed();
                let view = svc.workspace_view(&ws).await?;
                Ok(result::Value::Workspace(wire::workspace(&view, scope)))
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

            "workspace.remove_worktree" => {
                let id = Self::target(&req)?;
                let ws = svc
                    .list_workspaces()?
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
                // Same reasoning as `workspace.create`: this mutation writes
                // the workspace row itself, so the reconcile pass that
                // follows finds nothing gone and stays quiet. Without this,
                // other connected clients would not learn the worktree is
                // gone until the next RECONCILE_BACKSTOP tick (30s), if ever.
                self.watcher.announce_fleet_changed();
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
                                farcooler_protocol::v1::SplitSide::Right,
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
            // runner; a phone over ssh cannot, so without this it can list
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
                        farcooler_protocol::v1::TerminalScreen {
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

                Ok(result::Value::TerminalScreen(farcooler_protocol::v1::TerminalScreen {
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
                svc.remove_terminal(id).await?;
                // No terminal to return: it is gone. An empty workspace list is
                // the honest shape for "this succeeded and there is nothing to
                // show", rather than echoing back a record that no longer
                // exists.
                Ok(result::Value::TerminalList(farcooler_protocol::v1::TerminalList {
                    items: Vec::new(),
                }))
            }

            "terminal.dismiss_lost" => {
                let id = Self::target(&req)?;
                svc.dismiss_lost(id).await?;
                // Gone, so there is no record to echo — the same shape
                // `terminal.remove` answers with, for the same reason.
                Ok(result::Value::TerminalList(farcooler_protocol::v1::TerminalList {
                    items: Vec::new(),
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
                // Same reasoning as `workspace.hide`: this changes a pane
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
                svc.agents().send(
                    id,
                    DaemonMessage::Prompt {
                        text: wire::prompt_text(&p.blocks),
                        images: wire::prompt_images(&p.blocks),
                    },
                );
                // Typing into a pane is reading it. Waiting for the shim's first
                // event to move the row off `Done` would leave a terminal you
                // are actively using still asking for your attention.
                self.watcher.mark_seen(id).await;
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
                Ok(result::Value::ChangeSet(crate::review_ops::change_set(svc, &p).await?))
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
                Ok(result::Value::Empty(farcooler_protocol::v1::Empty {}))
            }

            "changes.inbox" => {
                Ok(result::Value::ChangesInbox(crate::review_ops::inbox(svc).await?))
            }

            "stack.get" => {
                let Some(request::Payload::StackGet(p)) = req.payload else {
                    return Err(DomainError::InvalidArgument { what: "payload" });
                };
                Ok(result::Value::StackLinkList(crate::review_ops::stack_get(svc, &p).await?))
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
                Ok(result::Value::WorktreeFileList(farcooler_protocol::v1::WorktreeFileList {
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
                            .and_then(|raw| farcooler_protocol::v1::LayoutPreset::try_from(raw).ok())
                            .unwrap_or(farcooler_protocol::v1::LayoutPreset::Tiled);
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
    ) -> farcooler_protocol::v1::Terminal {
        let mut message = wire::terminal_with_agent_state(view, self.service.agents());
        let (activity, state_since, turn_started_at) = self.watcher.activity(view.terminal.id).await;
        message.activity = activity as i32;
        message.activity_changed_at = state_since.map(wire::timestamp);
        message.turn_started_at = turn_started_at.map(wire::timestamp);
        message.blocked_question = self.watcher.blocked_question(view.terminal.id).await;
        if let Some(command) = self.watcher.command(view.terminal.id).await {
            message.current_command = command;
        }
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
            "repository_root.add",
            "repository.register",
            "workspace.create",
            "workspace.hide",
            "workspace.unhide",
            "terminal.seen",
            "terminal.watching",
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
            "terminal.paste_file",
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
            "changes.change_set",
            "changes.commit_files",
            "changes.file_diff",
            "changes.set_base",
            "changes.mark_read",
            "changes.inbox",
            "stack.get",
            "stack.set_parent",
            "pr.refresh",
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
        // colour is not a secret; an adapter's environment is.
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
/// same runner produced moments earlier.
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
