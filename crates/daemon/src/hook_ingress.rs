//! Where a live agent session reports what it is doing.
//!
//! **One socket per daemon, not one per terminal**, which is the departure
//! from `agent_supervisor::socket_path` worth explaining. That socket is per
//! terminal because the shim is launched by the daemon and told which terminal
//! it is. A hook is launched by the AGENT and knows only its own `session_id`
//! and its worktree, so the routing has to happen on this side.
//!
//! The path is short for `agent_supervisor`'s reason, unchanged: `sun_path` is
//! 104 bytes on macOS and the default runtime directory already spends 66 of
//! them. A bind that fails here is silent in exactly the way that costs a
//! week, so it is logged loudly.
//!
//! Silence is the shape of every failure on this side, and that is worth
//! saying plainly. `farcooler hook` bounds its whole conversation at 400ms,
//! exits 0 and prints nothing whatever happens — which is the property that
//! keeps Far Cooler out of the way of somebody's agent, and also means a
//! listener that is slow, wrong or absent looks from the agent's end exactly
//! like a listener that is working. Nothing over there will ever complain
//! about anything done here.
//!
//! So the diagnostics have to carry the whole weight, and they are pitched at
//! the level the daemon actually runs at — `main.rs` defaults the filter to
//! `farcooler=info,warn`, which a module whose every line was `debug!` would
//! be entirely invisible under. Two things are said out loud: the first time a
//! session binds to a terminal, because which conversation landed on which
//! pane has no other record and a wrong binding renders as an ordinary
//! transcript; and a read that FAILED, because folding that into the same
//! `None` an unclaimed session produces makes a broken runner and an idle one
//! look identical. Everything else — an unbound session, an ambiguity, a frame
//! we could not read, one that never ended, a connection that said nothing at
//! all — is written at `debug!` rather than not written: ordinary enough that
//! a warning would cry wolf, and a warning that cries wolf is how the one that
//! matters gets ignored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use farcooler_agent::event::AgentEvent;
use farcooler_agent_hooks::Agent;
use farcooler_agent_hooks::assemble::MessageAssembler;
use farcooler_agent_hooks::facts::{Facts, facts};
use farcooler_agent_hooks::wire::{HookLine, decode_line};
use farcooler_core::derive;
use farcooler_core::inventory::{RuntimeInventory, RuntimeSnapshot};
use farcooler_protocol::v1::TerminalState;
use farcooler_store::{Store, Terminal};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use uuid::Uuid;

/// How long a connection may say nothing at all before it is dropped.
///
/// NOT a deadline on a decision. `farcooler hook` bounds its whole
/// conversation at 400ms today, and the design says a permission somebody is
/// looking at "may take as long as a person takes" — so this is deliberately
/// far longer than any answer a person would give, and a gating hook added
/// later must still fit inside it.
///
/// It exists because nothing else reclaims a connection. A hook process that
/// leaked, or a peer that connected and died, holds a descriptor and a task
/// for the life of the daemon; enough of those is the descriptor shortage that
/// `listen` now has to survive.
const IDLE: std::time::Duration = std::time::Duration::from_secs(600);

/// How long to wait before accepting again after a refusal.
///
/// `LiveInventory::RETRY_PAUSE`'s reasoning, for the same kind of condition:
/// not a backoff, just long enough that the retry is asking about a different
/// moment. Immediately retrying `EMFILE` is a spin at full CPU.
const ACCEPT_RETRY_PAUSE: std::time::Duration = std::time::Duration::from_millis(150);

/// How often a refusal that will not go away is worth repeating.
///
/// The condition this exists for — a descriptor shortage — lasts as long as
/// whatever caused it, and at `ACCEPT_RETRY_PAUSE` the loop meets it about
/// seven times a second. A line each would put thousands of them an hour into
/// the log this module just went to some trouble to make worth reading.
const REFUSAL_REPORT_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// Says the first refusal, then says it again rarely, then says how many there
/// were.
///
/// Separated from the loop so the rule can be tested against a clock the test
/// owns; provoking a real `EMFILE` would mean exhausting the descriptor table
/// of the whole test binary.
#[derive(Default)]
struct Refusals {
    since_report: u64,
    total: u64,
    last_report: Option<std::time::Instant>,
}

impl Refusals {
    /// One more refusal. `Some(n)` when it is worth a line, `n` being how many
    /// have happened since the last one.
    ///
    /// The FIRST is always worth a line: the whole point is that a runner
    /// going deaf says so at the moment it happens, not thirty seconds later.
    fn refused(&mut self, now: std::time::Instant) -> Option<u64> {
        self.since_report += 1;
        self.total += 1;
        let due = self.last_report.is_none_or(|t| now.duration_since(t) >= REFUSAL_REPORT_EVERY);
        if !due {
            return None;
        }
        self.last_report = Some(now);
        Some(std::mem::take(&mut self.since_report))
    }

    /// The socket is taking connections again. `Some(n)` when it had stopped,
    /// so the recovery names what the quiet was hiding.
    fn recovered(&mut self) -> Option<u64> {
        let total = std::mem::take(&mut self.total);
        self.since_report = 0;
        self.last_report = None;
        (total > 0).then_some(total)
    }
}

/// Whether an `accept` failure is about this connection rather than the socket.
///
/// A descriptor shortage is the one that matters and it is the one Rust does
/// not name: `EMFILE` and `ENFILE` both arrive as `ErrorKind::Uncategorized`,
/// so they are matched by errno or not at all.
fn transient(e: &std::io::Error) -> bool {
    if matches!(
        e.kind(),
        std::io::ErrorKind::Interrupted | std::io::ErrorKind::ConnectionAborted
    ) {
        return true;
    }
    matches!(
        e.raw_os_error(),
        Some(libc::EMFILE) | Some(libc::ENFILE) | Some(libc::ENOBUFS) | Some(libc::ENOMEM)
    )
}

#[derive(Clone)]
pub struct HookIngress {
    store: Arc<Store>,
    /// tmux's view, for the one question the store cannot answer.
    ///
    /// A terminal's row says what somebody INTENDED; whether the pane is still
    /// alive is derived from this and never stored, which is the whole premise
    /// of `farcooler-store`. The announce path needs it because an exited pane
    /// keeps its row, and a row nobody reaps would otherwise sit in its
    /// worktree forever as a second candidate.
    inventory: Arc<dyn RuntimeInventory>,
    /// One assembler per terminal. Claude's prose arrives in pieces and the
    /// pieces of two panes must never be added to each other.
    ///
    /// Entries are added when a hook first routes to a terminal and removed by
    /// `forget`, which `Service` calls when a terminal's record is deleted.
    /// Without that call this map would hold an entry for every terminal this
    /// daemon has ever seen a hook from, each holding a marker for every
    /// message it ever displayed — a leak that grows with uptime and that
    /// nothing else in the process is positioned to notice.
    assemblers: Arc<Mutex<HashMap<Uuid, MessageAssembler>>>,
}

impl HookIngress {
    pub fn new(store: Arc<Store>, inventory: Arc<dyn RuntimeInventory>) -> Self {
        Self { store, inventory, assemblers: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Short, for `agent_supervisor::socket_path`'s reason.
    pub fn socket_path(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join("h.sock")
    }

    /// The terminal a session belongs to, or `None`.
    ///
    /// `None` is not a failure. A person running an agent in a pane Far Cooler
    /// does not know about is an ordinary thing, and the only wrong answer
    /// here is a confident one — attaching a conversation to the wrong pane is
    /// invisible by construction, because a transcript from another session
    /// still looks like a transcript.
    ///
    /// Two answers are therefore refused as firmly as no answer: a session id
    /// two terminals both claim, and a worktree holding two panes of the same
    /// agent with nothing to tell them apart. This is `set_pane_mode`'s
    /// adoption rule ("an ambiguous lookup refuses rather than attaching a
    /// chat to the wrong conversation") arriving by the new route.
    pub fn terminal_for(&self, f: &Facts, agent: Agent) -> Option<Uuid> {
        let session = f.session_id.as_deref()?;
        // A store error is not an answer. Swallowed into `None` it would be
        // indistinguishable from an ordinary unbound session, and every hook
        // on the runner would go quietly nowhere with nothing anywhere saying
        // why -- the hook itself exits 0 and prints nothing.
        let claimants = match self.store.terminals_with_agent_session(session) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    session,
                    "could not read which terminal claims this session; the hook is dropped"
                );
                return None;
            }
        };
        match claimants.as_slice() {
            [only] => return Some(only.id),
            [] => {}
            many => {
                tracing::debug!(
                    session,
                    claimants = many.len(),
                    "a session two terminals both claim reaches neither"
                );
                return None;
            }
        }
        self.announced_terminal(f, agent)
    }

    /// The terminal an agent that announces itself is sitting in.
    ///
    /// Claude declares its session at launch — `preset_command` passes
    /// `--session-id` and `create_terminal` mints it — so a claude session no
    /// row claims is a claude somebody started by hand, and guessing a
    /// worktree-mate for it would be the mistake `session_discovery`'s
    /// ambiguity refusal exists to avoid. Codex and cursor have no such flag
    /// and can only announce, so for those two a worktree match is the only
    /// binding there is.
    ///
    /// Keyed on the agent rather than on a `SessionStart` event name, because
    /// only codex sends one: cursor has no session-start hook at all, and
    /// gating on the name would leave every cursor session permanently
    /// unattached. The match is therefore standing rather than one-shot: it
    /// re-resolves on every hook and reaches the same answer, because nothing
    /// here writes.
    ///
    /// That is not free, and the cost is worth stating rather than assuming.
    /// `terminals` has no DECLARED index, and none on `workspace_id` — the
    /// `sqlite_autoindex` behind its `BLOB PRIMARY KEY` serves `get_terminal`
    /// and nothing on this path — so a hook costs a scan of `workspaces`, one
    /// `canonicalize` syscall per workspace to compare it, and a scan of
    /// `terminals` per matching workspace. At a flush every couple of seconds
    /// per session, against a fleet of panes, that is small; it is not a
    /// lookup by identity, and nothing here should be written as though it
    /// were.
    ///
    /// Two kinds of pane are never candidates. One that already names a
    /// session is speaking for a conversation, and handing it a second would
    /// draw two sessions into one transcript. And one whose pane is gone:
    /// nothing reaps a terminal's row, so a worktree accumulates the rows of
    /// every pane it has held, and counting those would find two candidates
    /// where there is one live pane and bind NEITHER — permanently, and
    /// silently, since a refusal looks exactly like an unmanaged pane. That
    /// judgement needs tmux, which is why this holds an inventory: the store
    /// records intent and nothing writes an exit onto a row when a process
    /// ends by itself, so a codex that quit looks, to the store alone, exactly
    /// like the live one beside it.
    fn announced_terminal(&self, f: &Facts, agent: Agent) -> Option<Uuid> {
        if agent == Agent::Claude {
            return None;
        }
        let cwd = canonical(f.cwd.as_deref()?);
        // Hidden rows included, deliberately. `hide_workspace` sets a flag and
        // never touches git or tmux, so an agent in a hidden worktree keeps
        // running and keeps firing hooks; `list_all_workspaces` filters
        // `hidden = 0` and says in its own doc that it is for summaries.
        // Reading through that one would leave every codex and cursor session
        // in a hidden worktree unattached while claude, which never consults a
        // workspace, kept working — an asymmetry nobody could guess from the
        // symptom.
        let workspaces = match self.store.list_workspaces_in_order() {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "could not read the worktrees; the announcement is dropped");
                return None;
            }
        };
        // Once, so every candidate is judged against one view of the runner.
        let snapshot = self.inventory.snapshot();

        let mut candidates: Vec<Uuid> = Vec::new();
        for ws in workspaces.iter().filter(|w| canonical(Path::new(&w.worktree_path)) == cwd) {
            let terminals = match self.store.list_terminals_for_workspace(ws.id) {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        workspace = %ws.id,
                        "could not read this worktree's panes; the announcement is dropped"
                    );
                    return None;
                }
            };
            candidates.extend(
                terminals
                    .into_iter()
                    .filter(|t| t.agent_session_id.is_none())
                    .filter(|t| preset_agent(&t.command_preset) == Some(agent))
                    .filter(|t| still_a_pane(t, &snapshot))
                    .map(|t| t.id),
            );
        }

        match candidates.as_slice() {
            [only] => Some(*only),
            [] => None,
            many => {
                tracing::debug!(
                    agent = agent.as_str(),
                    worktree = %cwd.display(),
                    candidates = many.len(),
                    "two panes of one agent in one worktree; the announcement binds to neither"
                );
                None
            }
        }
    }

    /// Fold one hook firing into events for a terminal it has already been
    /// routed to.
    ///
    /// Separate from `serve` so the state it accumulates can be reached
    /// without a socket, which is what lets a test put a terminal into this
    /// map and then watch `Service` delete that terminal — the only way to
    /// prove `forget` is reached by anything other than itself.
    pub fn accept(
        &self,
        terminal: Uuid,
        agent: Agent,
        event: &str,
        payload: &serde_json::Value,
        session: Option<&str>,
    ) -> Vec<AgentEvent> {
        let mut assemblers = self.assemblers.lock().unwrap_or_else(|e| e.into_inner());
        // The transition, not the flush. Which conversation ended up on which
        // pane is the fact with no other record: the hook exits 0 and prints
        // nothing, and a wrong binding renders as a working transcript.
        // `set_pane_mode`'s adoption path already learned this and logs at the
        // same level for the same reason — "the success path was the silent
        // one ... an ADOPTION recorded nothing at all".
        //
        // Once per terminal, because the map's own emptiness is what says
        // this is the first, and `forget` clears it along with the row.
        let first = !assemblers.contains_key(&terminal);
        let events = assemblers.entry(terminal).or_default().accept(agent, event, payload);
        drop(assemblers);
        if first {
            tracing::info!(
                terminal = %terminal,
                agent = agent.as_str(),
                session = ?session,
                "a live agent session is bound to this terminal"
            );
        }
        events
    }

    /// Drop the assembly state held for a terminal whose record is gone.
    ///
    /// Called from `Service`'s one delete path. A terminal that has been
    /// deleted can never be routed to again — `terminal_for` reads the same
    /// rows — so nothing here is reachable afterwards, and everything here is
    /// a partly-assembled message that will never be finished.
    pub fn forget(&self, terminal: Uuid) {
        self.assemblers.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
    }

    /// Whether any assembly state is held for a terminal. For tests and for
    /// logs.
    pub fn is_tracking(&self, terminal: Uuid) -> bool {
        self.assemblers.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&terminal)
    }

    /// Bind this daemon's one hook socket and serve it until something goes
    /// wrong with the listener itself.
    ///
    /// Returns only on an error, so a caller spawns it. A failure to bind is
    /// the one worth being loud about: every hook then connects to nothing,
    /// exits 0 and prints nothing, and every live session on the runner is
    /// silently invisible with no symptom anywhere but here.
    pub async fn listen<F>(&self, runtime_dir: &Path, on_events: F) -> std::io::Result<()>
    where
        F: Fn(Uuid, Vec<AgentEvent>) + Send + Sync + 'static,
    {
        let path = Self::socket_path(runtime_dir);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).inspect_err(|e| {
            tracing::error!(
                error = %e,
                path = %path.display(),
                bytes = path.as_os_str().len(),
                limit = crate::agent_supervisor::MAX_SOCKET_PATH,
                "could not bind the hook socket; no live session will report anything"
            );
        })?;
        let on_events = Arc::new(on_events);
        let mut refusals = Refusals::default();

        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => {
                    if let Some(refused) = refusals.recovered() {
                        tracing::warn!(
                            refused,
                            "the hook socket is taking connections again; \
                             this many hooks were turned away in the meantime"
                        );
                    }
                    stream
                }
                // A refused connection is not a broken listener. `?` here
                // ended the loop for good on a descriptor shortage somebody
                // else caused, and nothing calls this twice —
                // `agent_supervisor` at least re-arms through
                // `ensure_listening`. Every session on the runner then went
                // silent until the daemon was restarted, with the hook side
                // exiting 0 and printing nothing.
                Err(e) if transient(&e) => {
                    if let Some(refused) = refusals.refused(std::time::Instant::now()) {
                        tracing::warn!(
                            error = %e,
                            refused,
                            "the hook socket could not take a connection; still listening"
                        );
                    }
                    // A pause, because the common cause is a descriptor
                    // shortage and retrying it immediately is a spin at full
                    // CPU against a condition only time fixes.
                    tokio::time::sleep(ACCEPT_RETRY_PAUSE).await;
                    continue;
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        path = %path.display(),
                        "the hook listener has stopped; no live agent session will report \
                         anything on this runner until the daemon is restarted"
                    );
                    return Err(e);
                }
            };
            let this = self.clone();
            let on_events = on_events.clone();
            // One task per connection: a hook that is waiting on a decision
            // must not hold up every other hook on the runner.
            tokio::spawn(async move {
                if let Err(e) = this.serve(stream, on_events.as_ref()).await {
                    tracing::debug!(error = %e, "a hook connection ended");
                }
            });
        }
    }

    async fn serve<F>(&self, stream: UnixStream, on_events: &F) -> std::io::Result<()>
    where
        F: Fn(Uuid, Vec<AgentEvent>),
    {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            let Ok(read) = tokio::time::timeout(IDLE, reader.read_line(&mut line)).await else {
                tracing::debug!(
                    seconds = IDLE.as_secs(),
                    "a hook connection said nothing at all for this long; dropping it"
                );
                return Ok(());
            };
            if read? == 0 {
                return Ok(());
            }
            // The half of the framing contract this side owns. `hook.rs`'s
            // `converse` may be cut off mid-write by its own deadline and says
            // so: "the daemon reads whole lines, and half a frame with no
            // newline is discarded at EOF rather than acted on".
            //
            // `read_line` cannot tell those apart on its own — it returns the
            // bytes it found before EOF exactly as it returns a finished line —
            // and a truncated frame can still be valid JSON, so nothing further
            // down would notice either. The terminator is the whole of the
            // check.
            if !line.ends_with('\n') {
                tracing::debug!(
                    bytes = line.len(),
                    "a frame that never ended; discarded rather than acted on"
                );
                return Ok(());
            }
            let Ok(hook) = decode_line::<HookLine>(line.trim()) else {
                // Said, rather than dropped in silence. This frame was written
                // by our own `farcooler hook`, so a shape we cannot read means
                // the two halves of one design disagree — which is the
                // `ShimMessage::Failed` class of bug, declared and handled and
                // constructed nowhere, that this module's tests exist against.
                //
                // At `debug!` because it costs nothing under the daemon's own
                // filter and because a payload shape that really has changed
                // would arrive on every flush; the point is that somebody
                // looking has something to find, not that anybody is paged.
                tracing::debug!("a frame this daemon could not read");
                continue;
            };
            let f = facts(hook.agent, &hook.payload);
            let Some(terminal) = self.terminal_for(&f, hook.agent) else {
                tracing::debug!(session = ?f.session_id, "a hook from a session no terminal claims");
                continue;
            };

            let events =
                self.accept(terminal, hook.agent, &hook.event, &hook.payload, f.session_id.as_deref());
            if !events.is_empty() {
                on_events(terminal, events);
            }
        }
    }
}

/// Whether a terminal's row still has a pane a session could be running in.
///
/// `Exited`, `Lost` and `Error` are findings that the pane is gone. `Unknown`
/// is not: `derive_terminal` says outright that an unusable inventory "is not
/// proof of life — and it is not proof of death either", and tmux answers one
/// request at a time per server, so a single wedged pane makes every read
/// unhealthy for a few seconds. Retiring a candidate on that would make every
/// announcement fail exactly when the runner is busiest. `Starting` stays a
/// candidate too — a hook can fire before the daemon has confirmed the pane.
fn still_a_pane(terminal: &Terminal, snapshot: &RuntimeSnapshot) -> bool {
    !matches!(
        derive::derive_terminal(&crate::service::to_record(terminal), snapshot).state,
        TerminalState::Exited | TerminalState::Lost | TerminalState::Error
    )
}

/// Which agent a command preset runs, when it is one of the three that hook.
///
/// A preset is `claude`, or `claude:opus` — `preset_command` splits the model
/// off the same way, and a preset naming a model is still the same agent.
fn preset_agent(preset: &str) -> Option<Agent> {
    preset.split(':').next()?.parse().ok()
}

/// A worktree path in one spelling.
///
/// `/tmp` is a symlink to `/private/tmp` on macOS and `$TMPDIR` sits under
/// another one, so the `cwd` an agent reports and the path git reported for
/// the worktree routinely differ by a symlink neither side chose. Comparing
/// the strings would leave every session in such a worktree unattached, and it
/// would do it silently, which is the failure this whole module is prone to.
///
/// A path that cannot be resolved — the worktree is gone, or it never existed,
/// as in a store-level test — falls back to what it was given, which compares
/// exactly as it would have before.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_names_its_agent_with_or_without_a_model() {
        assert_eq!(preset_agent("codex"), Some(Agent::Codex));
        assert_eq!(preset_agent("codex:gpt-5.6-terra"), Some(Agent::Codex));
        assert_eq!(preset_agent("cursor"), Some(Agent::Cursor));
        assert_eq!(preset_agent("shell"), None, "a shell pane hosts no agent");
        assert_eq!(preset_agent(""), None);
    }

    /// The descriptor shortage is the case that matters, and it is the one
    /// `ErrorKind` does not name.
    ///
    /// A reader written against `ErrorKind` alone looks complete — it handles
    /// `Interrupted` and `ConnectionAborted` — and still ends the listener for
    /// good on `EMFILE`, which is both the likeliest cause and the one that
    /// mends itself. `EMFILE` and `ENFILE` arrive as `Uncategorized`, which no
    /// pattern may match, so they are matched by errno or not at all.
    #[test]
    fn a_descriptor_shortage_is_a_refused_connection_and_not_a_broken_socket() {
        for errno in [libc::EMFILE, libc::ENFILE, libc::ENOBUFS, libc::ENOMEM] {
            let e = std::io::Error::from_raw_os_error(errno);
            assert!(
                transient(&e),
                "errno {errno} ({e}) arrives as {:?} and must not end the listener",
                e.kind()
            );
        }
        for kind in [std::io::ErrorKind::Interrupted, std::io::ErrorKind::ConnectionAborted] {
            assert!(transient(&std::io::Error::from(kind)), "{kind:?} is about one connection");
        }
    }

    /// And something that really is the socket still stops it.
    ///
    /// Without this the rule could be "everything is transient", which never
    /// returns and never reports — the accept loop would spin forever on a
    /// listener that is genuinely gone.
    #[test]
    fn a_socket_that_is_actually_broken_is_not_treated_as_transient() {
        for errno in [libc::EBADF, libc::EINVAL, libc::ENOTSOCK] {
            let e = std::io::Error::from_raw_os_error(errno);
            assert!(!transient(&e), "errno {errno} ({e}) is the listener itself");
        }
    }

    /// A refusal that will not go away must not become the log.
    ///
    /// The first is said at once — a runner going deaf has to say so when it
    /// happens — and then the rule has to hold its tongue, because the
    /// condition it was written for persists and the loop meets it about
    /// seven times a second. The nearest wrong implementation is the one this
    /// replaced: a line every time, which is thousands an hour under exactly
    /// the circumstance the warning was added for.
    #[test]
    fn a_refusal_that_persists_is_reported_once_and_then_rarely() {
        let mut refusals = Refusals::default();
        let start = std::time::Instant::now();

        assert_eq!(refusals.refused(start), Some(1), "the first is always worth a line");

        // A second of it, at the rate the loop actually runs.
        let mut at = start;
        for _ in 0..7 {
            at += ACCEPT_RETRY_PAUSE;
            assert_eq!(refusals.refused(at), None, "and then it stops talking");
        }

        // Past the reporting interval, one line, carrying what the silence held.
        let later = start + REFUSAL_REPORT_EVERY;
        assert_eq!(
            refusals.refused(later),
            Some(8),
            "the repeat says how many refusals it is standing for"
        );
    }

    /// And the count is what makes the quiet safe to have.
    #[test]
    fn recovery_reports_every_refusal_the_quiet_covered() {
        let mut refusals = Refusals::default();
        let start = std::time::Instant::now();
        for i in 0..100 {
            refusals.refused(start + ACCEPT_RETRY_PAUSE * i);
        }

        assert_eq!(
            refusals.recovered(),
            Some(100),
            "every refusal is counted, including the ones no line was written for"
        );
        assert_eq!(refusals.recovered(), None, "and a socket that never stopped says nothing");
    }

    /// The socket has to fit, and the reason it might not is the runtime
    /// directory rather than anything chosen here.
    #[test]
    fn the_hook_socket_fits_in_a_unix_socket_path() {
        let real = Path::new("/Users/somebody/Library/Application Support/com.farcooler.Far Cooler");
        let path = HookIngress::socket_path(real);
        assert!(
            path.as_os_str().len() <= crate::agent_supervisor::MAX_SOCKET_PATH,
            "{} is {} bytes, over the {} a bind accepts",
            path.display(),
            path.as_os_str().len(),
            crate::agent_supervisor::MAX_SOCKET_PATH
        );
    }
}
