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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use farcooler_agent::event::{AgentEvent, Role};
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

use crate::transcript_tail::TranscriptTail;

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
    /// One `TranscriptTail` per terminal whose agent is codex or cursor —
    /// started once, the first time a hook payload names a `transcript_path`
    /// for that terminal, and never again. Starting a second on the same path
    /// would double every answer, the same failure `MessageAssembler` exists
    /// to avoid for claude's own streamed deltas.
    ///
    /// The value is not the tail itself — nothing here needs to stop the
    /// underlying watch, only to stop ACTING on what it finds — so this holds
    /// an `AtomicBool` the tail's own sink closure checks before calling
    /// `on_events`. `forget` flips it to `false`; the background thread
    /// `TranscriptTail::follow` owns keeps running past that (nothing in its
    /// own interface offers a way to stop it), but every delivery after
    /// `forget` becomes a no-op rather than an event for a terminal whose row
    /// is gone.
    tails: Arc<Mutex<HashMap<Uuid, Arc<AtomicBool>>>>,
    /// The erased sink `listen` was started with, kept so a transcript tail
    /// can call it long after the hook connection that started the tail has
    /// closed. `serve`'s own `on_events: &F` is borrowed from the ONE
    /// connection it is handling and does not outlive it — `farcooler hook`
    /// bounds a whole conversation at 400ms — while a tail must keep
    /// delivering for as long as the terminal exists. `None` until `listen`
    /// has run once; a hook connection reached any other way (a test calling
    /// `accept` directly, say) starts no tail rather than spawning one with
    /// nowhere to deliver to.
    sink: Arc<Mutex<Option<EventSink>>>,
}

/// The erased shape of a `listen`/`start_transcript_tail` sink. Named so
/// neither call site spells out the `Arc<dyn Fn(...) + Send + Sync>` clippy's
/// `type_complexity` lint (CI's `-D warnings`) refuses inline.
type EventSink = Arc<dyn Fn(Uuid, Vec<AgentEvent>) + Send + Sync>;

impl HookIngress {
    pub fn new(store: Arc<Store>, inventory: Arc<dyn RuntimeInventory>) -> Self {
        Self {
            store,
            inventory,
            assemblers: Arc::new(Mutex::new(HashMap::new())),
            tails: Arc::new(Mutex::new(HashMap::new())),
            sink: Arc::new(Mutex::new(None)),
        }
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
            [only] => {
                if is_a_chat(only) {
                    tracing::debug!(
                        session,
                        terminal = %only.id,
                        "this pane's conversation already arrives over its shim; the hook is dropped"
                    );
                    return None;
                }
                return Some(only.id);
            }
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
                    .filter(|t| !is_a_chat(t))
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

    /// Drop the assembly state held for a terminal whose record is gone, and
    /// silence any transcript tail running for it.
    ///
    /// Called from `Service`'s one delete path. A terminal that has been
    /// deleted can never be routed to again — `terminal_for` reads the same
    /// rows — so nothing here is reachable afterwards, and everything here is
    /// a partly-assembled message that will never be finished.
    ///
    /// The tail's own background thread is not stopped — `TranscriptTail`
    /// offers no way to, and it is not asked to here — only its `AtomicBool`
    /// is flipped, so a delivery that lands after this becomes a no-op
    /// instead of an event for a terminal `agents.record` would otherwise
    /// have to invent a fresh entry for.
    pub fn forget(&self, terminal: Uuid) {
        self.assemblers.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
        if let Some(alive) = self.tails.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal) {
            alive.store(false, Ordering::Relaxed);
        }
    }

    /// Whether any assembly state is held for a terminal. For tests and for
    /// logs.
    pub fn is_tracking(&self, terminal: Uuid) -> bool {
        self.assemblers.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&terminal)
    }

    /// Whether a transcript tail has been started for a terminal. For tests
    /// and for logs — the same reason `is_tracking` exists.
    pub fn is_tailing(&self, terminal: Uuid) -> bool {
        self.tails.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&terminal)
    }

    /// Install the erased sink `listen` was started with, so a transcript
    /// tail — whose deliveries outlive any one hook connection — has
    /// somewhere to call long after `serve`'s own `on_events: &F` has gone
    /// out of scope. Returns the same `Arc` `listen`'s own accept loop clones
    /// per connection, so there is exactly one sink behind both paths.
    ///
    /// Split out of `listen` so a test can populate `self.sink` without
    /// binding a real socket — `listen` never returns except on a broken
    /// listener, which makes it an awkward thing to run inline in a test that
    /// only wants `start_transcript_tail` reachable.
    fn install_sink<F>(&self, on_events: F) -> EventSink
    where
        F: Fn(Uuid, Vec<AgentEvent>) + Send + Sync + 'static,
    {
        let sink: EventSink = Arc::new(on_events);
        *self.sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink.clone());
        sink
    }

    /// Start reading a codex or cursor session's own transcript for the
    /// prose no hook payload carries — see `transcript_tail`'s module doc for
    /// why codex and cursor need this and claude does not.
    ///
    /// A no-op past the first call for a given `terminal`: `self.tails`'
    /// entry for it is what says a tail already exists, and starting a
    /// second on the same path would double every answer this one already
    /// delivers. Also a no-op when `listen` has not run — `self.sink` is
    /// `None` — since a tail with nowhere to deliver would be a background
    /// thread doing work for nobody, forever.
    ///
    /// **The `tails` entry is claimed BEFORE the tail is actually started,
    /// not after.** Two hook payloads for one terminal can be `serve`d by two
    /// different connections at once — nothing serializes them — and
    /// claiming the slot first, under one lock, is what stops both from
    /// starting a tail; whichever loses the race sees the entry already
    /// there and returns. If starting genuinely fails (`TranscriptTail::
    /// follow` returns `false`, or the task that runs it panics), the entry
    /// is removed again so the NEXT payload gets another attempt — a failed
    /// watch registration must not brick this terminal's prose for the rest
    /// of the session, which is what an insert with no way back out would
    /// do.
    ///
    /// **`TranscriptTail::follow` is deliberately not run inline on `serve`'s
    /// own task.** It does its catch-up read and its OS-level watch
    /// registration synchronously, and both are ordinary blocking calls —
    /// `File::open`, `read_to_end`, `notify::recommended_watcher`,
    /// `Watcher::watch` — with no `.await` anywhere between them. Run
    /// straight inside `serve`'s async body, that blocks the tokio worker
    /// thread serving THIS connection for however long registration takes —
    /// measured, in the sandbox this was built against, at up to ~11 seconds
    /// under load (see the task report) against a doc that used to claim
    /// this "keeps well inside 400ms". `tokio::spawn` plus `spawn_blocking`
    /// moves that whole sequence off any worker thread `serve` needs, at the
    /// cost of `start_transcript_tail` itself no longer waiting to see
    /// whether the tail actually started before returning — which is exactly
    /// why the slot has to be claimed synchronously, above, rather than by
    /// the spawned task.
    ///
    /// Two things stay on this thread on purpose and are cheap enough to:
    /// claiming the `tails` slot, and the one `std::fs::metadata` that fixes
    /// `from` — see that call's own comment for why deferring a snapshot of
    /// "what counts as history" loses prose.
    fn start_transcript_tail(&self, terminal: Uuid, agent: Agent, f: &Facts) {
        if !matches!(agent, Agent::Codex | Agent::Cursor) {
            return;
        }
        let Some(path) = f.transcript_path.clone() else { return };

        let alive = Arc::new(AtomicBool::new(true));
        {
            let mut tails = self.tails.lock().unwrap_or_else(|e| e.into_inner());
            if tails.contains_key(&terminal) {
                return;
            }
            tails.insert(terminal, alive.clone());
        }
        let Some(sink) = self.sink.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            self.tails.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
            return;
        };

        // The length of the file at THIS moment, not 0: a session's own
        // history is not this feature's to replay. `terminal_for` may bind a
        // session that already has several turns behind it — a daemon
        // restart mid-conversation, or an `announced_terminal` match on a
        // pane that was already running — and starting at 0 would draw every
        // one of those turns into the live transcript as if they had all
        // just happened, in one burst, the moment the tail catches up.
        //
        // Taken HERE, synchronously, and not down inside the spawned task
        // with everything else this defers. `from` is the line between
        // "history, already told" and "live, still to tell", and every byte
        // written between this moment and the moment the snapshot is
        // actually taken lands on the wrong side of it — read as history and
        // dropped, silently, forever. Deferring it moves that line from
        // "when the hook was processed" to "whenever a blocking-pool thread
        // got round to it", which is unbounded. One `stat` is not what I1
        // was about: the ~11s measured under load was `Watcher::watch`, and
        // that is still deferred below.
        let from = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        tracing::info!(
            terminal = %terminal,
            agent = agent.as_str(),
            path = %path.display(),
            "tailing this session's own transcript for its prose"
        );

        let this = self.clone();
        let loop_alive = alive.clone();
        tokio::spawn(async move {
            let started = tokio::task::spawn_blocking(move || {
                TranscriptTail::new().follow(
                    path,
                    from,
                    move |text| {
                        if !alive.load(Ordering::Relaxed) {
                            return;
                        }
                        sink(terminal, vec![AgentEvent::Message { role: Role::Agent, text, parent: None }]);
                    },
                    loop_alive,
                )
            })
            .await;

            match started {
                Ok(true) => {}
                Ok(false) => {
                    tracing::debug!(
                        terminal = %terminal,
                        "a transcript tail could not be started; the next payload for this terminal will retry"
                    );
                    this.tails.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
                }
                Err(error) => {
                    tracing::warn!(
                        terminal = %terminal,
                        %error,
                        "the task starting a transcript tail did not finish; the next payload for this terminal will retry"
                    );
                    this.tails.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
                }
            }
        });
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
        let on_events = self.install_sink(on_events);
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
        // `?Sized`, because `listen` now hands this the SAME erased sink
        // `start_transcript_tail` reaches through `self.sink` — a trait
        // object behind a reference, not a concrete closure — rather than
        // `Arc::new`-ing a fresh one of the generic `F` this function used to
        // be instantiated with. One sink behind both paths is the whole
        // point: `install_sink`'s own doc says why a tail cannot borrow the
        // one `serve` gets for the length of a single 400ms connection.
        F: Fn(Uuid, Vec<AgentEvent>) + ?Sized,
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
            self.start_transcript_tail(terminal, hook.agent, &f);

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
/// A pane whose conversation already arrives over its shim.
///
/// **One ring per terminal, fed by one transport.** A pane in `Agent` mode has
/// an ACP shim in it reporting every message over its own socket into
/// `AgentSupervisor::apply`; a hook routed to that same terminal appends the
/// same prose again through `record`, interleaved into the one ring, and the
/// user reads every assistant turn twice.
///
/// Guarding both of `terminal_for`'s routes with one predicate rather than
/// only the route that bites today, because both are live and for different
/// reasons. The announcement route is codex's and cursor's by default: their
/// hooks are project-local, so `install_project_hooks` puts them in every
/// worktree Far Cooler makes and they fire in agent-mode panes too. The
/// claimants route reaches an agent-mode pane whenever one carries an
/// `agent_session_id` — claude's from `--session-id` at launch, and codex's
/// from the shim's own `Established` report, which `set_pane_mode` stores.
///
/// This was deferred once, in `agent_supervisor`'s
/// `a_recorded_event_continues_the_transcript_the_shim_started`, on the
/// premise that "nothing in this tree writes any of those three files yet".
/// Six commits later the same branch wrote them. A deferral is only as good as
/// its premise, and a premise about what the tree contains has to be
/// re-checked by whoever makes the tree contain it.
fn is_a_chat(terminal: &Terminal) -> bool {
    terminal.pane_mode == farcooler_store::models::PaneMode::Agent
}

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

    // -- start_transcript_tail: the side effect `serve` reaches for codex
    // and cursor -- exercised directly against `install_sink`, without a
    // real socket, since `listen` never returns except on a broken listener.

    fn ingress_for_test() -> HookIngress {
        let store = Arc::new(Store::open_in_memory().expect("store"));
        let inventory: Arc<dyn RuntimeInventory> =
            Arc::new(farcooler_core::inventory::FakeInventory::default());
        HookIngress::new(store, inventory)
    }

    /// Poll rather than a fixed sleep: both the initial catch-up read and the
    /// fallback poll inside `TranscriptTail::follow` (`WAIT_POLL_FALLBACK`)
    /// deliver on their own schedule, not this test's.
    async fn until_len(events: &Arc<Mutex<Vec<(Uuid, AgentEvent)>>>, n: usize, deadline_ms: u64) {
        let start = std::time::Instant::now();
        loop {
            if events.lock().unwrap().len() >= n
                || start.elapsed() > std::time::Duration::from_millis(deadline_ms)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    /// The nearest wrong implementation starts a fresh `TranscriptTail` on
    /// every payload, which would deliver one appended line as two events —
    /// the exact doubling this module's own doc on `tails` warns about.
    #[tokio::test]
    async fn a_second_payload_for_the_same_terminal_does_not_start_a_second_tail() {
        let ingress = ingress_for_test();
        let events: Arc<Mutex<Vec<(Uuid, AgentEvent)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        ingress.install_sink(move |terminal, batch| {
            let mut events = sink_events.lock().unwrap();
            for event in batch {
                events.push((terminal, event));
            }
        });

        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(&path, "").expect("seed");
        let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
        let terminal = Uuid::from_u128(101);

        ingress.start_transcript_tail(terminal, Agent::Codex, &f);
        ingress.start_transcript_tail(terminal, Agent::Codex, &f);
        assert!(ingress.is_tailing(terminal), "the first call must have started one");

        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "agent_message", "phase": "commentary", "message": "checking" },
            })
        )
        .expect("append");

        until_len(&events, 1, 20_000).await;
        let seen = events.lock().unwrap();
        assert_eq!(
            seen.len(),
            1,
            "one append must reach the sink once, however many times start_transcript_tail was called: {seen:?}"
        );
        assert!(
            matches!(
                &seen[0],
                (t, AgentEvent::Message { role: Role::Agent, text, parent: None })
                    if *t == terminal && text == "checking"
            ),
            "got {seen:?}"
        );
    }

    /// `forget` must silence a running tail, not merely leave `is_tracking`
    /// (the assembler map) looking clean. The nearest wrong implementation
    /// removes only the assembler entry, which is `forget`'s OLD body — a
    /// tail started after that would keep delivering into a transcript whose
    /// terminal no longer exists.
    #[tokio::test]
    async fn forget_silences_a_running_tail() {
        let ingress = ingress_for_test();
        let events: Arc<Mutex<Vec<(Uuid, AgentEvent)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        ingress.install_sink(move |terminal, batch| {
            let mut events = sink_events.lock().unwrap();
            for event in batch {
                events.push((terminal, event));
            }
        });

        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(&path, "").expect("seed");
        let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
        let terminal = Uuid::from_u128(102);
        ingress.start_transcript_tail(terminal, Agent::Codex, &f);

        use std::io::Write;
        let append = |text: &str| {
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(
                file,
                "{}",
                serde_json::json!({
                    "type": "event_msg",
                    "payload": { "type": "agent_message", "phase": "commentary", "message": text },
                })
            )
            .unwrap();
        };

        append("before forget");
        until_len(&events, 1, 20_000).await;
        assert_eq!(events.lock().unwrap().len(), 1, "the tail must be delivering before forget");

        ingress.forget(terminal);
        assert!(!ingress.is_tailing(terminal), "forget must clear the tracked tail");

        append("after forget");
        // Longer than `transcript_tail::WAIT_POLL_FALLBACK` (1s): if the
        // background thread is still delivering, its own periodic fallback
        // alone would have picked this up well within this wait.
        tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
        assert_eq!(
            events.lock().unwrap().len(),
            1,
            "a line appended after forget must never reach the sink, however long this waits"
        );
    }

    /// The dedup invariant `codex_final_answer_is_dropped_because_stop_
    /// already_sends_it` (in `transcript_tail`'s own tests) can only see
    /// from the decode side: it proves `assistant_text` returns `None` for a
    /// `final_answer` line, which is true whether or not anything ELSE ever
    /// produces that text. It cannot see the other producer.
    ///
    /// This drives BOTH real inputs into one terminal — codex's own
    /// `final_answer` line, sitting in the rollout the tail is reading, AND
    /// a `Stop` hook payload carrying the identical text as
    /// `last_assistant_message` — and asserts exactly one `Message` reaches
    /// the terminal's transcript. The nearest wrong implementation drops the
    /// phase filter (forwards every `agent_message`/`AgentMessage`
    /// regardless of phase): that implementation draws the same answer
    /// twice, once from each producer, and this is the one test in the suite
    /// that would actually see it, because it is the only one that gives
    /// both producers something to say.
    #[tokio::test]
    async fn codexs_own_final_answer_line_and_its_stop_payload_do_not_both_land() {
        let ingress = ingress_for_test();
        let events: Arc<Mutex<Vec<(Uuid, AgentEvent)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        ingress.install_sink(move |terminal, batch| {
            let mut events = sink_events.lock().unwrap();
            for event in batch {
                events.push((terminal, event));
            }
        });

        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        // Empty, not merely absent: nothing is written to this file before
        // the tail starts, so whatever `from` the tail computes for itself
        // is 0 regardless of exactly when it computes it — the append below
        // races nothing, because there is nothing in the file yet to have
        // already been "caught up" on.
        std::fs::write(&path, "").expect("seed");
        let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
        let terminal = Uuid::from_u128(106);

        ingress.start_transcript_tail(terminal, Agent::Codex, &f);

        let answer = "TCP slow start is a congestion-control mechanism.";
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "event_msg",
                "payload": {
                    "type": "agent_message",
                    "phase": "final_answer",
                    "message": answer,
                },
            })
        )
        .expect("append");

        // Longer than `transcript_tail::WAIT_POLL_FALLBACK` (1s): the point
        // is to give the tail's own poll loop every chance it would ever
        // get to (wrongly) forward this line BEFORE the Stop payload below
        // gives the real producer its turn — a race the other direction (Stop
        // firing before the tail has even looked at the file) would prove
        // nothing, since a dropped implementation and a correct one would
        // both show one Message either way.
        tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
        assert!(
            events.lock().unwrap().is_empty(),
            "codex's own final_answer line must never reach the sink by itself: {:?}",
            events.lock().unwrap()
        );

        let stop_events = ingress.accept(
            terminal,
            Agent::Codex,
            "Stop",
            &serde_json::json!({
                "hook_event_name": "Stop",
                "session_id": "s",
                "turn_id": "t",
                "last_assistant_message": answer,
            }),
            None,
        );
        events.lock().unwrap().extend(stop_events.into_iter().map(|e| (terminal, e)));

        let seen = events.lock().unwrap();
        let messages: Vec<_> = seen
            .iter()
            .filter(|(_, e)| matches!(e, AgentEvent::Message { role: Role::Agent, .. }))
            .collect();
        assert_eq!(
            messages.len(),
            1,
            "the rollout's own final_answer line and Stop's last_assistant_message name the \
             same turn's closing answer; exactly one of the two producers may land it: {seen:?}"
        );
        assert!(
            matches!(
                messages[0],
                (t, AgentEvent::Message { text, parent: None, .. }) if *t == terminal && text == answer
            ),
            "got {:?}",
            messages[0]
        );
    }

    /// A tail that could not start must RELEASE the terminal's slot rather
    /// than leave it claimed. The nearest wrong implementation is the one
    /// this round replaced: insert into `tails` and never look at whether
    /// anything actually started, which marks the terminal tailed for the
    /// rest of the daemon's life with nothing running behind it and no later
    /// payload able to try again — the whole feature silently off for that
    /// terminal, at `debug!`.
    ///
    /// `/` is the cheapest transcript path with no parent directory to watch,
    /// which is the one case `TranscriptTail::follow` still refuses outright
    /// rather than degrading to polling.
    #[tokio::test]
    async fn a_tail_that_could_not_start_releases_the_terminal_for_the_next_payload() {
        let ingress = ingress_for_test();
        ingress.install_sink(|_, _| {});

        let f = Facts { transcript_path: Some(PathBuf::from("/")), ..Facts::default() };
        let terminal = Uuid::from_u128(107);
        ingress.start_transcript_tail(terminal, Agent::Codex, &f);

        // The slot is claimed synchronously and released by the spawned task,
        // so this polls rather than reading once: what is being asserted is
        // that the release happens at all, not how quickly.
        let start = std::time::Instant::now();
        while ingress.is_tailing(terminal) && start.elapsed() < std::time::Duration::from_secs(10) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(
            !ingress.is_tailing(terminal),
            "a terminal whose tail never started must be left free for its next payload to retry"
        );
    }

    /// Claude already streams through `MessageDisplay` — starting a tail for
    /// it too would draw its answers a second time from the transcript.
    #[tokio::test]
    async fn claude_never_gets_a_transcript_tail() {
        let ingress = ingress_for_test();
        ingress.install_sink(|_, _| {});

        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "").expect("seed");
        let f = Facts { transcript_path: Some(path), ..Facts::default() };
        let terminal = Uuid::from_u128(103);

        ingress.start_transcript_tail(terminal, Agent::Claude, &f);
        assert!(!ingress.is_tailing(terminal), "claude's own MessageDisplay already streams its prose");
    }

    /// A payload naming no transcript at all — every real one does
    /// (`Facts`'s own doc), but nothing here should assume it.
    #[tokio::test]
    async fn no_transcript_path_starts_no_tail() {
        let ingress = ingress_for_test();
        ingress.install_sink(|_, _| {});

        let terminal = Uuid::from_u128(104);
        ingress.start_transcript_tail(terminal, Agent::Codex, &Facts::default());
        assert!(!ingress.is_tailing(terminal));
    }

    /// Before `listen` (or, here, `install_sink`) has run, `self.sink` is
    /// `None`. A tail started anyway would be a background thread doing work
    /// for a sink that will never exist — reachable only from a test that
    /// calls `accept` or `start_transcript_tail` directly, since `serve` is
    /// only ever reached through `listen`, but worth refusing rather than
    /// assuming.
    #[tokio::test]
    async fn no_sink_installed_starts_no_tail() {
        let ingress = ingress_for_test();
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(&path, "").expect("seed");
        let f = Facts { transcript_path: Some(path), ..Facts::default() };
        let terminal = Uuid::from_u128(105);

        ingress.start_transcript_tail(terminal, Agent::Codex, &f);
        assert!(!ingress.is_tailing(terminal), "nothing was installed to deliver to");
    }
}
