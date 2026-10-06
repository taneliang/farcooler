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
//! saying plainly. `farcooler hook` bounds its first contact at 400ms (a held
//! permission ask waits up to a minute more, and only on this side's word),
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
use farcooler_agent_hooks::ask::{is_gate, permission_ask};
use farcooler_agent_hooks::wire::{HookLine, LONGEST_HOLD, Reply, decode_line, encode_line, is_fence};
use farcooler_core::derive;
use farcooler_core::inventory::{RuntimeInventory, RuntimeSnapshot};
use farcooler_protocol::v1::TerminalState;
use farcooler_store::{Store, Terminal};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use uuid::Uuid;

use crate::hook_asks::{AnswerRefused, HookAsks, Settled};
use crate::transcript_tail::TranscriptTail;

/// How long a connection may say nothing at all before it is dropped.
///
/// NOT a deadline on a decision. `farcooler hook` bounds its first contact at
/// 400ms, and a held permission ask is bounded by `LONGEST_HOLD` in
/// `hold_ask`, which does not read under this at all. So this is deliberately
/// far longer than either.
///
/// It exists because nothing else reclaims a connection. A hook process that
/// leaked, or a peer that connected and died, holds a descriptor and a task
/// for the life of the daemon; enough of those is the descriptor shortage that
/// `listen` now has to survive.
const IDLE: std::time::Duration = std::time::Duration::from_secs(600);

/// Tools whose `PermissionRequest` is not an ordinary "may this tool run" ask,
/// so it is never held or offered to a phone.
///
/// The gate's `matcher` is `"*"`, so it fires for every tool, and claude
/// routes more than tool permissions through this hook: a question it asks
/// the person (`AskUserQuestion`), and its request to leave plan mode
/// (`ExitPlanMode`). A phone offers only Allow and Deny, and what an Allow does
/// to one of these is unmeasured. It could answer a question nobody read, or
/// approve a plan nobody saw. So these are told "no decision" at once, and the
/// dialog stays with the keyboard. An explicit list, not a guess from the
/// name: a tool added here is one somebody has decided a phone must not answer.
const NOT_TOOL_PERMISSIONS: &[&str] = &["AskUserQuestion", "ExitPlanMode"];

/// How long to wait before accepting again after a refusal.
///
/// `LiveInventory::RETRY_PAUSE`'s reasoning, for the same kind of condition:
/// not a backoff, just long enough that the retry is asking about a different
/// moment. Immediately retrying `EMFILE` is a spin at full CPU.
const ACCEPT_RETRY_PAUSE: std::time::Duration = std::time::Duration::from_millis(150);

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
    /// bounds a conversation at 400ms, or a minute when held — while a tail must keep
    /// delivering for as long as the terminal exists. `None` until `listen`
    /// has run once; a hook connection reached any other way (a test calling
    /// `accept` directly, say) starts no tail rather than spawning one with
    /// nowhere to deliver to.
    sink: Arc<Mutex<Option<EventSink>>>,
    /// `Service`'s claims ledger, so a Claude session's `cwd` can claim the
    /// worktree it is working in (`observe_cwd`).
    claims: Arc<crate::claims::Ledger>,
    /// Claude's permission asks, held while a phone may answer them. Shares
    /// `sink`, so an ask's `Resolved` lands where its `Permission` did.
    asks: Arc<HookAsks>,
    /// How long a held ask waits for a device. `LONGEST_HOLD`, except in
    /// tests (`with_hold`).
    hold: std::time::Duration,
    /// Claude's session registry, which binds a claude session no row names.
    /// Shared by every clone, so a test can swap it under a running service.
    registry: Arc<std::sync::RwLock<&'static crate::claude_registry::Registry>>,
}

/// Free `terminal`'s `tails` slot, but only while it still holds `mine`,
/// the `alive` flag the start that is giving up inserted.
///
/// A start's failure is reported after the slot has been out of its hands:
/// `forget` can remove its entry while the start is still running, and the
/// next hook payload can then insert a new start's entry under the same
/// terminal. Removing by key alone would take that newer entry out. Its tail
/// keeps delivering with nothing left that can stop it, and the payload after
/// that starts another tail on the same file, so every line arrives twice.
fn release_tail_slot(tails: &Mutex<HashMap<Uuid, Arc<AtomicBool>>>, terminal: Uuid, mine: &Arc<AtomicBool>) {
    let mut tails = tails.lock().unwrap_or_else(|e| e.into_inner());
    if tails.get(&terminal).is_some_and(|entry| Arc::ptr_eq(entry, mine)) {
        tails.remove(&terminal);
    }
}

/// The erased shape of a `listen`/`start_transcript_tail` sink. Named so
/// neither call site spells out the `Arc<dyn Fn(...) + Send + Sync>` clippy's
/// `type_complexity` lint (CI's `-D warnings`) refuses inline.
pub(crate) type EventSink = Arc<dyn Fn(Uuid, Vec<AgentEvent>) + Send + Sync>;

impl HookIngress {
    pub fn new(
        store: Arc<Store>,
        inventory: Arc<dyn RuntimeInventory>,
        claims: Arc<crate::claims::Ledger>,
    ) -> Self {
        let sink = Arc::new(Mutex::new(None));
        Self {
            store,
            inventory,
            assemblers: Arc::new(Mutex::new(HashMap::new())),
            tails: Arc::new(Mutex::new(HashMap::new())),
            asks: Arc::new(HookAsks::new(sink.clone())),
            sink,
            claims,
            hold: LONGEST_HOLD,
            registry: Arc::new(std::sync::RwLock::new(crate::claude_registry::global())),
        }
    }

    /// The same ingress, reading `registry` rather than this machine's.
    pub fn with_registry(self, registry: &'static crate::claude_registry::Registry) -> Self {
        *self.registry.write().unwrap_or_else(|e| e.into_inner()) = registry;
        self
    }

    /// The registry the daemon reads: the watcher's log join and adoption ask
    /// this one too, so one swap reaches every call site.
    pub fn claude_registry(&self) -> &'static crate::claude_registry::Registry {
        *self.registry.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The same ingress, holding each ask for `hold` rather than
    /// `LONGEST_HOLD`, so a test of the hold running out takes less than a
    /// minute.
    pub fn with_hold(mut self, hold: std::time::Duration) -> Self {
        self.hold = hold.min(LONGEST_HOLD);
        self
    }

    /// A device's answer to the ask held under `id` on `terminal`: `allow`
    /// or `deny`, and `decider`, the device's name, which a deny carries.
    ///
    /// Returns once the verdict is on the hook's socket.
    pub async fn answer(
        &self,
        terminal: Uuid,
        id: &str,
        option: &str,
        decider: &str,
    ) -> Result<(), AnswerRefused> {
        self.asks.answer(terminal, id, option, decider).await
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
    /// row claims is one started by hand, or the new session `/clear` begins
    /// in the same pane. A worktree-mate is never guessed for it; claude's own
    /// registry says which pane it is in, and binds the row to it
    /// (`registry_binding::bind_pane`). Codex and cursor have no such flag
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
    /// `terminals` has no DECLARED index, and none on `worktree_id` — the
    /// `sqlite_autoindex` behind its `BLOB PRIMARY KEY` serves `get_terminal`
    /// and nothing on this path — so a hook costs a scan of `worktrees`, one
    /// `canonicalize` syscall per worktree to compare it, and a scan of
    /// `terminals` per matching worktree. At a flush every couple of seconds
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
            let snapshot = self.inventory.snapshot();
            return crate::registry_binding::bind_pane(self.claude_registry(), &self.store, &snapshot, f.session_id.as_deref()?);
        }
        let cwd = canonical(f.cwd.as_deref()?);
        // Hidden rows included, deliberately. `hide_worktree` sets a flag and
        // never touches git or tmux, so an agent in a hidden worktree keeps
        // running and keeps firing hooks; `list_all_worktrees` filters
        // `hidden = 0` and says in its own doc that it is for summaries.
        // Reading through that one would leave every codex and cursor session
        // in a hidden worktree unattached while claude, which never consults a
        // worktree, kept working — an asymmetry nobody could guess from the
        // symptom.
        let worktrees = match self.store.list_worktrees_in_order() {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "could not read the worktrees; the announcement is dropped");
                return None;
            }
        };
        // Once, so every candidate is judged against one view of the runner.
        let snapshot = self.inventory.snapshot();

        let mut candidates: Vec<Uuid> = Vec::new();
        for ws in worktrees.iter().filter(|w| canonical(Path::new(&w.worktree_path)) == cwd) {
            let terminals = match self.store.list_terminals_for_worktree(ws.id) {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        worktree_id = %ws.id,
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
        if agent == Agent::Claude {
            crate::session_projectors::global().hook(terminal, event, payload);
        }
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
        self.asks.forget(terminal);
        self.assemblers.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
        crate::session_projectors::global().forget(terminal);
        if let Some(alive) = self.tails.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal) {
            alive.store(false, Ordering::Relaxed);
        }
    }

    /// Claiming's second signal: where a Claude Code session says it is
    /// working claims that worktree for its pane's workspace
    /// (`claims::observe_in`).
    ///
    /// Claude only, and on the events already registered, which is what the
    /// spike measured (spec, "Spike findings (2026-09-27)"). Claude's `cwd`
    /// follows a `cd` in its Bash tool and an `EnterWorktree`, and every
    /// later hook reports it, `Stop` included, so a move is seen by the end
    /// of the turn it happened in. Codex's `cwd` is its launch directory
    /// whatever its commands do. Cursor's registered events carry no `cwd`,
    /// and `facts` falls back to `workspace_roots[0]`, its launch directory
    /// too. For both, that is the pane's own worktree, which is already
    /// known without a hook; acting on it would only repeat the pane's
    /// placement as though it were news. Their moves are left to the
    /// process walk (`claims::scan`).
    ///
    /// A Claude hook reaches a terminal by its session id, through
    /// `terminal_for`'s claimants: `preset_command` launches every Claude
    /// Far Cooler starts with `--session-id`, and the row holds it. A Claude
    /// someone started by hand has no row naming its session, so it claims
    /// nothing, as it routes nothing.
    fn observe_cwd(&self, terminal: Uuid, agent: Agent, f: &Facts) {
        if agent != Agent::Claude {
            return;
        }
        let Some(cwd) = f.cwd.as_deref() else { return };
        let source = farcooler_store::models::ClaimSource::Hook;
        if let Err(e) = crate::claims::observe_in(&self.store, &self.claims, terminal, cwd, source) {
            tracing::warn!(error = %e, %terminal, "could not judge where a session is working");
        }
    }

    /// Whether any assembly state is held for a terminal. For tests and for
    /// logs.
    pub fn is_tracking(&self, terminal: Uuid) -> bool {
        self.assemblers.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&terminal)
    }

    /// The held permission asks, which `terminal.agent_answer` answers.
    pub fn asks(&self) -> &Arc<HookAsks> {
        &self.asks
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
    /// is removed again, if it is still this start's (`release_tail_slot`),
    /// so the NEXT payload gets another attempt — a failed
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
            release_tail_slot(&self.tails, terminal, &alive);
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
        // Kept back so a start that fails below can also stop whatever it
        // did get running: a slot freed while a loop still holds `alive` true
        // is a tail nothing can stop, and the next payload starts another.
        let stop = alive.clone();
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
                    release_tail_slot(&this.tails, terminal, &stop);
                }
                Err(error) => {
                    tracing::warn!(
                        terminal = %terminal,
                        %error,
                        "the task starting a transcript tail did not finish; the next payload for this terminal will retry"
                    );
                    stop.store(false, Ordering::Relaxed);
                    release_tail_slot(&this.tails, terminal, &stop);
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
        // one `serve` gets for the length of a single short connection.
        F: Fn(Uuid, Vec<AgentEvent>) + ?Sized,
    {
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
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
            // The fence (`hook_asks`): marked and answered before claude can
            // draw a dialog for this call; the call's end, or a turn's, ends
            // the mark.
            let session = f.session_id.as_deref();
            if let Some(session) = session {
                self.asks.heard(session, fence::raises_dialog(hook.agent, &hook.event, &hook.payload));
            }
            if is_fence(hook.agent, &hook.event) {
                fence::answer(&self.asks, session, &hook.payload, &mut write).await?;
            } else if let Some(session) = session {
                fence::ended(&self.asks, session, &hook.event, &hook.payload);
            }
            let terminal = self.terminal_for(&f, hook.agent);
            if is_gate(hook.agent, &hook.event) {
                let Some(terminal) = terminal else {
                    // Said at once. Silence here costs the agent the hook's
                    // whole 400 ms for an ask nobody can be shown: a session
                    // no pane claims, an ambiguous one, or a chat pane, whose
                    // asks arrive over its shim.
                    tracing::debug!(session = ?f.session_id, "an ask no terminal can take");
                    write_reply(&mut write, &Reply::verdict(None)).await?;
                    continue;
                };
                if hook.payload["tool_name"].as_str().is_some_and(|t| NOT_TOOL_PERMISSIONS.contains(&t)) {
                    tracing::debug!(%terminal, "a PermissionRequest that is not a tool permission; not held");
                    write_reply(&mut write, &Reply::verdict(None)).await?;
                    continue;
                }
                return self.hold_ask(terminal, hook, f, reader, write).await;
            }
            let Some(terminal) = terminal else {
                tracing::debug!(session = ?f.session_id, "a hook from a session no terminal claims");
                continue;
            };
            // A turn cannot end or begin with claude's dialog up, so whatever
            // was asked on this pane has been answered at the keyboard.
            if hook.agent == Agent::Claude && matches!(hook.event.as_str(), "Stop" | "StopFailure" | "UserPromptSubmit") {
                self.asks.turn_boundary(terminal);
            }
            self.start_transcript_tail(terminal, hook.agent, &f);
            // A store read, and a write when it claims. Nothing waits on it
            // here: a hook that isn't gating hangs up once it has written. The
            // gating hook, which does wait, is `hold_ask`'s, and there this
            // runs after the reply.
            self.observe_cwd(terminal, hook.agent, &f);

            let events =
                self.accept(terminal, hook.agent, &hook.event, &hook.payload, f.session_id.as_deref());
            if !events.is_empty() {
                on_events(terminal, events);
            }
        }
    }

    /// Hold a gating hook's ask until a device, the keyboard or the clock
    /// ends it, and write the hook its verdict.
    ///
    /// In this order, because the hook hears nothing after its 400 ms:
    /// hold the ask, reply with the hold, and only then offer it (a
    /// `Permission` for an ask whose hook had already gone would be a button
    /// nothing answers) and check the claim. The offer goes through the
    /// ledger (`HookAsks::offer`), because anything can end the ask between
    /// the hold and the offer, and only the ledger can say it didn't. The claim check is a store read
    /// and maybe a write, under the claims ledger's lock, and it runs off this
    /// task, so none of that can make the reply late.
    ///
    /// One ask per connection: the hook reads its verdict and hangs up.
    async fn hold_ask(
        &self,
        terminal: Uuid,
        hook: HookLine,
        f: Facts,
        mut reader: BufReader<OwnedReadHalf>,
        mut write: OwnedWriteHalf,
    ) -> std::io::Result<()> {
        // The tool's name rides on the lock screen's card ("Bash · Billing");
        // `push::WireAsk` decides whether it may.
        let tool = hook.payload["tool_name"].as_str();
        let (id, mut settled) = self.asks.hold_for(terminal, tool, self.hold);
        if let Err(e) = write_reply(&mut write, &Reply::hold(self.hold)).await {
            // The hook missed its deadline and has gone. Nobody was offered
            // this ask, so it ends with nothing to take back.
            self.asks.withdraw(terminal, &id);
            return Err(e);
        }
        // `false` when something ended it already; `settled` then has that
        // ending, and the select below writes it at once.
        self.asks.offer(terminal, &id, permission_ask(&id, &hook.payload));
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            this.start_transcript_tail(terminal, hook.agent, &f);
            this.observe_cwd(terminal, hook.agent, &f);
            let events =
                this.accept(terminal, hook.agent, &hook.event, &hook.payload, f.session_id.as_deref());
            let sink = this.sink.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let (false, Some(sink)) = (events.is_empty(), sink) {
                sink(terminal, events);
            }
        });

        let ended = tokio::select! {
            ended = &mut settled => ended.ok(),
            () = tokio::time::sleep(self.hold) => {
                // Withdrawn, unless something ended it first; either way it
                // has ended now, and `settled` says how.
                self.asks.withdraw(terminal, &id);
                (&mut settled).await.ok()
            }
            () = hung_up(&mut reader) => {
                self.asks.withdraw(terminal, &id);
                return Ok(());
            }
        };
        let Some(Settled { decision, ack }) = ended else { return Ok(()) };
        write_reply(&mut write, &Reply::verdict(decision)).await?;
        // Only now is a device told its answer landed. A write that failed
        // drops `ack`, and the device hears that instead.
        if let Some(ack) = ack {
            let _ = ack.send(());
        }
        Ok(())
    }
}

/// One line to a hook.
async fn write_reply(write: &mut OwnedWriteHalf, reply: &Reply) -> std::io::Result<()> {
    let line = encode_line(reply).map_err(std::io::Error::other)?;
    write.write_all(line.as_bytes()).await
}

/// Resolves when the hook's side of the connection is gone.
///
/// A held hook writes nothing more, so anything it does send is discarded;
/// what matters is EOF or an error, which is claude exiting, the pane dying,
/// or the hook being killed at its `timeout`.
async fn hung_up(reader: &mut BufReader<OwnedReadHalf>) {
    let mut discard = String::new();
    loop {
        discard.clear();
        match reader.read_line(&mut discard).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
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
/// hooks are project-local, so they are installed in every worktree Far Cooler
/// makes and in any worktree one of those two is opened in
/// (`Service::prepare_launch_hooks`), and they fire in agent-mode panes too. The
/// claimants route reaches an agent-mode pane whenever one carries an
/// `agent_session_id` — claude's from `--session-id` at launch, and codex's
/// from the shim's own `Established` report, which
/// `AgentSupervisor::remember_session` writes down as it arrives.
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

    /// The hook hears back inside its 400 ms whatever the claim check costs.
    ///
    /// `observe_cwd` is a store read, a write when it claims, and the claims
    /// ledger's lock. A claude hook's `cwd` inside its pane's own worktree
    /// reaches that lock (`claims::observe_in` settles an owned worktree), so
    /// holding it here stands in for a claim check that is slow for any
    /// reason. Multi-threaded, because the check that waits on it must not
    /// take the only thread the reply could be written from.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_reply_does_not_wait_for_the_claim_check() {
        use farcooler_protocol::v1::TerminalIntent;
        use farcooler_store::models::{ClaimSource, PaneMode};

        let dir = tempfile::tempdir().expect("dir");
        let root = std::fs::canonicalize(dir.path()).expect("canonical");
        let repo_path = root.join("repo");
        std::fs::create_dir_all(&repo_path).expect("repo dir");
        let store = Store::open_in_memory().expect("store");
        let host = Uuid::now_v7();
        let root_row = store.create_repository_root(host, &root.to_string_lossy(), 1_000).unwrap();
        let repo = store
            .create_repository(host, root_row.id, "repo", &repo_path.join(".git").to_string_lossy(), "")
            .unwrap();
        let main = store.ensure_main_workspace(repo.id).unwrap();
        let checkout = store.create_worktree(repo.id, "main", &repo_path.to_string_lossy(), true).unwrap();
        store.claim_worktree(checkout.id, main.id, ClaimSource::Explicit).unwrap();
        let term = store.create_terminal(checkout.id, "pane", "claude", TerminalIntent::Running, 80, 24).unwrap();
        let term = store.set_terminal_workspace(term.id, main.id).unwrap();
        store
            .set_pane_mode(term.id, term.resource_version, PaneMode::Terminal, Some("sess-1".to_string()), false)
            .unwrap();

        let claims = Arc::new(crate::claims::Ledger::default());
        let inventory: Arc<dyn RuntimeInventory> =
            Arc::new(farcooler_core::inventory::FakeInventory::default());
        let ingress = HookIngress::new(Arc::new(store), inventory, claims.clone());
        let sock = tempfile::tempdir().expect("dir");
        {
            let ingress = ingress.clone();
            let sock = sock.path().to_path_buf();
            tokio::spawn(async move { ingress.listen(&sock, |_, _| {}).await });
        }
        let socket = HookIngress::socket_path(sock.path());
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let busy = claims.lock_for_test();
        let line = HookLine {
            agent: Agent::Claude,
            event: "PermissionRequest".to_string(),
            payload: serde_json::json!({
                "session_id": "sess-1",
                "cwd": repo_path,
                "tool_name": "Bash",
                "tool_input": { "command": "touch x" },
            }),
        };
        // A std socket with the kernel's read timeout, not tokio's timer:
        // with a worker parked in the claim check, nothing is certain to
        // drive tokio's time driver, and a test that hangs proves nothing.
        let frame = farcooler_agent_hooks::wire::encode_line(&line).expect("encode");
        let mut stream = std::os::unix::net::UnixStream::connect(&socket).expect("connect");
        stream.set_read_timeout(Some(std::time::Duration::from_secs(1))).expect("timeout");
        std::io::Write::write_all(&mut stream, frame.as_bytes()).expect("write");
        let mut reply = String::new();
        let heard = std::io::BufRead::read_line(&mut std::io::BufReader::new(&stream), &mut reply);
        drop(busy);
        assert!(
            matches!(heard, Ok(n) if n > 0),
            "the hook heard nothing while the claim check waited: {heard:?}"
        );
        assert!(reply.contains("hold_ms"), "{reply:?}");

        // And the ask it holds names claude's tool and ends when the hold
        // does, which is what the lock screen's card is sent (ov-57).
        let mut open = None;
        for _ in 0..200 {
            open = ingress.asks().open_on(term.id);
            if open.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let open = open.expect("the ask was offered");
        assert_eq!(open.tool.as_deref(), Some("Bash"));
        let left = open.until.duration_since(std::time::SystemTime::now()).expect("not over yet");
        assert!(left <= farcooler_agent_hooks::wire::LONGEST_HOLD, "{left:?}");
        assert!(left > farcooler_agent_hooks::wire::LONGEST_HOLD / 2, "{left:?}");
        drop(stream);
    }

    fn ingress_for_test() -> HookIngress {
        let store = Arc::new(Store::open_in_memory().expect("store"));
        let inventory: Arc<dyn RuntimeInventory> =
            Arc::new(farcooler_core::inventory::FakeInventory::default());
        HookIngress::new(store, inventory, Arc::default())
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

    /// A deleted terminal's held ask is released, not left for its minute: its
    /// hook would otherwise keep claude waiting on a pane nobody can answer.
    #[tokio::test]
    async fn forgetting_a_terminal_releases_its_held_ask() {
        let ingress = ingress_for_test();
        let terminal = Uuid::from_u128(103);
        let (_id, rx) = ingress.asks().hold(terminal);
        ingress.forget(terminal);
        assert!(!ingress.asks().is_holding(terminal));
        assert_eq!(rx.await.expect("the hook is released").decision, None);
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
        // Kept, to count who else holds it: the ingress, this test, and each
        // running tail's thread.
        let sink = ingress.install_sink(move |terminal, batch| {
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
        let with_the_tail = Arc::strong_count(&sink);

        ingress.forget(terminal);
        assert!(!ingress.is_tailing(terminal), "forget must clear the tracked tail");

        // The positive event: the tail's thread ends and lets go of the sink.
        // A sleep and an unchanged count would pass just as well for a tail
        // still running that had not polled yet. Once the thread is gone,
        // nothing is left that could deliver a line.
        let start = std::time::Instant::now();
        while Arc::strong_count(&sink) >= with_the_tail && start.elapsed() < std::time::Duration::from_secs(10) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(Arc::strong_count(&sink), with_the_tail - 1, "the forgotten tail's thread is still running");

        append("after forget");
        assert_eq!(
            events.lock().unwrap().len(),
            1,
            "a line appended after forget must never reach the sink"
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
        // A commentary line after it, which the tail does forward. The tail
        // reads the file in order, so once this has arrived the final_answer
        // line has been read too, and anything it was going to send has been
        // sent: the tail has had its turn BEFORE the Stop payload below gives
        // the real producer its turn. A race the other direction (Stop firing
        // before the tail has even looked at the file) would prove nothing,
        // since a dropped implementation and a correct one would both show
        // one Message either way. A fixed sleep here passed whether or not the
        // tail had got that far.
        let sentinel = "Reading the congestion window code.";
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "agent_message", "phase": "commentary", "message": sentinel },
            })
        )
        .expect("append");
        until_len(&events, 1, 20_000).await;
        assert!(
            matches!(
                events.lock().unwrap().as_slice(),
                [(_, AgentEvent::Message { text, .. })] if text == sentinel
            ),
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
            .filter(|(_, e)| matches!(e, AgentEvent::Message { role: Role::Agent, text, .. } if text != sentinel))
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

    /// A start that fails after `forget` has already taken its slot must not
    /// free the slot the NEXT start claimed since. The nearest wrong
    /// implementation removes by terminal alone: the newer tail keeps
    /// delivering with no entry left for `forget` to stop it by, and the
    /// payload after that starts a second tail on the same file.
    ///
    /// The first start fails in the `Err` arm: its task panics during the
    /// catch-up read, because the sink panics on the one line written
    /// between the first start's `stat` and its read. The sink holds that
    /// panic until `forget` and the second start have both landed, which
    /// fixes the order. The second start's `stat` comes after the line, so
    /// its tail never reads it.
    #[tokio::test]
    async fn a_start_that_panics_after_forget_leaves_the_next_starts_slot_alone() {
        let ingress = ingress_for_test();
        let reached = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let (sink_reached, sink_release) = (reached.clone(), release.clone());
        ingress.install_sink(move |_, batch| {
            if batch.iter().any(|e| matches!(e, AgentEvent::Message { text, .. } if text == "boom")) {
                sink_reached.store(true, Ordering::SeqCst);
                // A blocking-pool thread, so this wait stalls nothing the test needs.
                let start = std::time::Instant::now();
                while !sink_release.load(Ordering::SeqCst) && start.elapsed() < std::time::Duration::from_secs(10) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                panic!("the first start's catch-up read fails here, on purpose");
            }
        });

        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(&path, "").expect("seed");
        let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
        let terminal = Uuid::from_u128(108);

        ingress.start_transcript_tail(terminal, Agent::Codex, &f);
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "agent_message", "phase": "commentary", "message": "boom" },
            })
        )
        .expect("append");

        let start = std::time::Instant::now();
        while !reached.load(Ordering::SeqCst) && start.elapsed() < std::time::Duration::from_secs(10) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(reached.load(Ordering::SeqCst), "the first start never reached its catch-up read");

        ingress.forget(terminal);
        ingress.start_transcript_tail(terminal, Agent::Codex, &f);
        assert!(ingress.is_tailing(terminal), "the second start claimed the slot");
        release.store(true, Ordering::SeqCst);

        // The `Err` arm runs on this runtime once the panic has unwound and
        // this test yields. A second is ample for that.
        let start = std::time::Instant::now();
        while ingress.is_tailing(terminal) && start.elapsed() < std::time::Duration::from_secs(1) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(
            ingress.is_tailing(terminal),
            "the first start's failure freed the slot the second start holds"
        );
    }

    /// The same race through the `Ok(false)` arm: `/` has no parent directory
    /// to watch, so the first start returns `false` (as in
    /// `a_tail_that_could_not_start_releases_the_terminal_for_the_next_payload`).
    /// There is no signal for when it has, so this waits a second, which is
    /// far longer than a failed read of `/` takes.
    #[tokio::test]
    async fn a_start_that_could_not_start_after_forget_leaves_the_next_starts_slot_alone() {
        let ingress = ingress_for_test();
        ingress.install_sink(|_, _| {});

        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(&path, "").expect("seed");
        let terminal = Uuid::from_u128(109);

        let root = Facts { transcript_path: Some(PathBuf::from("/")), ..Facts::default() };
        ingress.start_transcript_tail(terminal, Agent::Codex, &root);
        ingress.forget(terminal);
        let f = Facts { transcript_path: Some(path), ..Facts::default() };
        ingress.start_transcript_tail(terminal, Agent::Codex, &f);
        assert!(ingress.is_tailing(terminal), "the second start claimed the slot");

        let start = std::time::Instant::now();
        while ingress.is_tailing(terminal) && start.elapsed() < std::time::Duration::from_secs(1) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(
            ingress.is_tailing(terminal),
            "the first start's failure freed the slot the second start holds"
        );
    }

    /// The helper behind all three failure exits, on its own: a release
    /// leaves an entry some other start inserted, and removes its own. The
    /// two race tests above can only wait and see that nothing happened, so
    /// this is the check that decides it.
    #[test]
    fn releasing_a_tail_slot_removes_only_the_releasers_own_entry() {
        let tails = Mutex::new(HashMap::new());
        let terminal = Uuid::from_u128(110);
        let a = Arc::new(AtomicBool::new(true));
        let b = Arc::new(AtomicBool::new(true));
        tails.lock().unwrap().insert(terminal, a.clone());

        release_tail_slot(&tails, terminal, &b);
        assert!(tails.lock().unwrap().contains_key(&terminal), "a release by another start took this entry");

        release_tail_slot(&tails, terminal, &a);
        assert!(!tails.lock().unwrap().contains_key(&terminal), "a release by the entry's own start left it");
    }

    /// A start whose task panics, with no `forget` racing it, frees its own
    /// slot through the `Err` arm, so the next payload can start a tail.
    #[tokio::test]
    async fn a_start_that_panics_releases_the_terminal_for_the_next_payload() {
        let ingress = ingress_for_test();
        ingress.install_sink(|_, batch| {
            if batch.iter().any(|e| matches!(e, AgentEvent::Message { text, .. } if text == "boom")) {
                panic!("the start's catch-up read fails here, on purpose");
            }
        });

        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(&path, "").expect("seed");
        let f = Facts { transcript_path: Some(path.clone()), ..Facts::default() };
        let terminal = Uuid::from_u128(111);

        ingress.start_transcript_tail(terminal, Agent::Codex, &f);
        assert!(ingress.is_tailing(terminal), "the start claimed the slot");
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "agent_message", "phase": "commentary", "message": "boom" },
            })
        )
        .expect("append");

        let start = std::time::Instant::now();
        while ingress.is_tailing(terminal) && start.elapsed() < std::time::Duration::from_secs(10) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(!ingress.is_tailing(terminal), "a start that panicked kept its slot");
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

mod fence;
mod refusals;
use refusals::{Refusals, transient};
#[cfg(test)]
use refusals::REFUSAL_REPORT_EVERY;
#[cfg(test)]
#[path = "hook_fence_tests.rs"]
mod hook_fence_tests;
