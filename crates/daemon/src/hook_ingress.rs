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

use farcooler_agent_core::event::{AgentEvent, Role};
use farcooler_agent_hooks::Agent;
use farcooler_agent_hooks::assemble::MessageAssembler;
use farcooler_agent_hooks::facts::{Facts, facts};
use farcooler_agent_hooks::ask::{is_gate, permission_ask};
use farcooler_agent_hooks::wire::{HookLine, LONGEST_HOLD, Reply, decode_line, encode_line, is_fence, is_followup};
use farcooler_core::derive;
use farcooler_core::inventory::{RuntimeInventory, RuntimeSnapshot};
use farcooler_core::session_log::projector::held::HELD_ASK;
use farcooler_protocol::v1::TerminalState;
use farcooler_store::{Store, Terminal};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use uuid::Uuid;

use crate::hook_asks::{AnswerRefused, AskShape, HookAsks, Settled};
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
    /// Where a cursor `stop` hook asks for a message (`followup`).
    followups: Arc<Mutex<Option<followup::Followups>>>,
    /// Subagents asked to stop (`stopping`, ov-872).
    stopping: Arc<stopping::Stopping>,
    /// Admission for connection tasks and transcript tails (cutover join).
    gate: Option<Arc<crate::writer_gate::WriterGate>>,
    /// Held while a frame that names a launch acts on its pane, and while a
    /// relaunch cancels what the pane's previous launch left (ov-886). One per
    /// stripe of panes rather than one for the runner, so a pane whose hook is
    /// slow holds up no other pane's. See `held_for`.
    launch_gates: Arc<Vec<Mutex<()>>>,
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

/// How many stripes the launch gate has (`HookIngress::launch_gates`).
const LAUNCH_GATES: usize = 64;

/// Pane `terminal`'s stripe of the launch gate.
fn launch_gate(gates: &[Mutex<()>], terminal: Uuid) -> &Mutex<()> {
    &gates[(terminal.as_u128() % gates.len() as u128) as usize]
}

/// What `HookIngress::held_for` hands back: the gate, held, for a frame that
/// names a launch, and nothing to hold for one that doesn't.
pub(crate) enum Held<'a> {
    Free,
    Gated(#[allow(dead_code)] std::sync::MutexGuard<'a, ()>),
}

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
            followups: Arc::default(),
            stopping: Arc::default(),
            gate: None,
            launch_gates: Arc::new((0..LAUNCH_GATES).map(|_| Mutex::new(())).collect()),
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
                        "this pane is still a chat, moving to its terminal; the hook is dropped"
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

    /// The terminal a frame goes to: by its launch when it names one, and by
    /// `terminal_for` when it doesn't (ov-886).
    pub fn route(&self, f: &Facts, agent: Agent, launch: Option<Uuid>) -> Option<Uuid> {
        match launch {
            None => self.terminal_for(f, agent),
            Some(launch) => self.launched_terminal(f, agent, launch),
        }
    }

    /// The pane whose current launch is `launch`, if this frame belongs there.
    ///
    /// A frame that names its launch is judged on that alone, and never falls
    /// back to the session or the worktree: those are exactly what can't tell
    /// a restarted pane's killed process from its replacement, which may have
    /// resumed the very same session. A launch no pane holds is one a restart
    /// retired, and its frame reaches nothing.
    ///
    /// The launch id is inherited by everything the agent starts, so the pane
    /// must also be running this agent, and the frame must be its own:
    ///
    /// - Claude: the session the row names, or one Claude's registry places in
    ///   this pane (`/clear`, or a restart that had nothing to resume and so
    ///   began a new session). A `claude` someone runs by hand inside the pane
    ///   is neither, and isn't pulled in.
    /// - Codex and Cursor report their launch directory, which must be the
    ///   pane's worktree.
    fn launched_terminal(&self, f: &Facts, agent: Agent, launch: Uuid) -> Option<Uuid> {
        let session = f.session_id.as_deref()?;
        let terminal = match self.store.terminal_for_launch(launch) {
            Ok(Some(terminal)) => terminal,
            Ok(None) => {
                tracing::debug!(%launch, "a hook from a launch no pane holds any more; it reaches nothing");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, %launch, "could not read which pane holds this launch; the hook is dropped");
                return None;
            }
        };
        let row = self.store.get_terminal(terminal).ok()?;
        if is_a_chat(&row) || preset_agent(&row.command_preset) != Some(agent) {
            tracing::debug!(terminal = %terminal, agent = agent.as_str(), "a hook carrying a pane's launch from an agent that pane isn't running");
            return None;
        }
        let own = match agent {
            Agent::Claude => {
                row.agent_session_id.as_deref() == Some(session) || {
                    let snapshot = self.inventory.snapshot();
                    crate::registry_binding::bind_pane(self.claude_registry(), &self.store, &snapshot, session)
                        == Some(terminal)
                }
            }
            Agent::Codex | Agent::Cursor => {
                let cwd = f.cwd.as_deref()?;
                let worktree = self.store.get_worktree(row.worktree_id).ok()?;
                canonical(cwd) == canonical(Path::new(&worktree.worktree_path))
            }
        };
        if !own {
            tracing::debug!(terminal = %terminal, session, "a hook carrying a pane's launch from a session that isn't the pane's");
        }
        own.then_some(terminal)
    }

    /// Whether `launch` is any pane's current launch.
    fn launch_is_current(&self, launch: Uuid) -> bool {
        matches!(self.store.terminal_for_launch(launch), Ok(Some(_)))
    }

    /// The launch gate, held, if a frame from `launch` may still act on
    /// `terminal`: `launch` is still its current one, or the frame names none.
    ///
    /// Every effect a frame has on its pane happens while this is held, and
    /// the check is made again inside it, so a restart can't land between the
    /// check and the effect. `relaunched` takes the same gate after the new
    /// launch is committed: either a frame's effects finished first and
    /// `relaunched` cancels what they left, or they start after and see the
    /// new launch. Never held across an `.await`.
    ///
    /// A frame that names no launch takes no gate: it's routed as it always
    /// was, and a hook that predates launches mustn't start waiting on one.
    /// Nothing slow runs under it: `observe_cwd`, which can wait on the claims
    /// ledger, runs after it's released.
    pub(crate) fn held_for(&self, terminal: Uuid, launch: Option<Uuid>) -> Option<Held<'_>> {
        let Some(launch) = launch else { return Some(Held::Free) };
        let held = launch_gate(&self.launch_gates, terminal).lock().unwrap_or_else(|e| e.into_inner());
        (self.store.current_launch(terminal).ok().flatten() == Some(launch)).then_some(Held::Gated(held))
    }

    /// Cancel what pane `terminal`'s previous launch left in flight, once a
    /// new launch is current (ov-886): its held ask, settled unanswered so a
    /// process still alive defers to its keyboard; its half-assembled message;
    /// and its transcript tail. Under the launch gate, for `held_for`'s reason.
    ///
    /// Narrower than `forget`, which is for a pane that's gone: the pane's
    /// conversation and its subagents' rows carry on into the new launch.
    pub fn relaunched(&self, terminal: Uuid) {
        let _held = launch_gate(&self.launch_gates, terminal).lock().unwrap_or_else(|e| e.into_inner());
        self.asks.forget(terminal);
        self.assemblers.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal);
        if let Some(alive) = self.tails.lock().unwrap_or_else(|e| e.into_inner()).remove(&terminal) {
            alive.store(false, Ordering::Relaxed);
        }
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
        // claude's and codex's hooks both have a projection to move (ov-378).
        match agent {
            Agent::Claude => crate::session_projectors::global().hook(terminal, event, payload),
            Agent::Codex => crate::session_projectors::global().codex_hook(terminal, event, payload),
            Agent::Cursor => {}
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
        crate::session_projectors::global().forget_terminal(terminal);
        crate::subagent_rows::global().forget(terminal);
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

        let Ok(ticket) = self.admit() else { release_tail_slot(&self.tails, terminal, &alive); return };
        let this = self.clone();
        let loop_alive = alive.clone();
        // Kept back so a start that fails below can also stop whatever it
        // did get running: a slot freed while a loop still holds `alive` true
        // is a tail nothing can stop, and the next payload starts another.
        let stop = alive.clone();
        let launch_gates = self.launch_gates.clone();
        tokio::spawn(async move {
            let started = tokio::task::spawn_blocking(move || {
                TranscriptTail::new().follow(
                    path,
                    from,
                    move |text| {
                        // Under the launch gate, so `relaunched` can't stop
                        // this between the check and the delivery.
                        let _held = launch_gate(&launch_gates, terminal).lock().unwrap_or_else(|e| e.into_inner());
                        if !alive.load(Ordering::Relaxed) {
                            return;
                        }
                        sink(terminal, vec![AgentEvent::Message { role: Role::Agent, text, parent: None }]);
                    },
                    loop_alive,
                )
            })
            .await;
            let clean = started.is_ok();

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
            if let (true, Some(t)) = (clean, ticket) { t.finish(); }
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
                limit = crate::paths::MAX_SOCKET_PATH,
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
            let Ok(ticket) = self.admit() else { continue };
            let this = self.clone();
            let on_events = on_events.clone();
            // One task per connection: a hook that is waiting on a decision
            // must not hold up every other hook on the runner.
            tokio::spawn(async move {
                if let Err(e) = this.serve(stream, on_events.as_ref()).await {
                    tracing::debug!(error = %e, "a hook connection ended");
                }
                if let Some(t) = ticket { t.finish(); }
            });
        }
    }

    pub(crate) async fn serve<F>(&self, stream: UnixStream, on_events: &F) -> std::io::Result<()>
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
            let Ok(mut hook) = decode_line::<HookLine>(line.trim()) else {
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
            // A held ask's id is the runner's to give (`hold_ask`), never a
            // frame's: one a pane wrote would put a real hold, and so who
            // answered it, on a row of the pane's own making (ov-743).
            if let Some(payload) = hook.payload.as_object_mut() {
                payload.remove(HELD_ASK);
            }
            let f = facts(hook.agent, &hook.payload);
            // A frame that names a launch no pane holds any more came from a
            // process a restart replaced (ov-886). It reaches nothing: no
            // fence mark, no ask, no followup, no event. One that waits is
            // told no decision at once rather than left to its 400 ms. A
            // launch that isn't a uuid was never one this runner made.
            let launch = match hook.launch.as_deref().map(Uuid::parse_str) {
                None => None,
                Some(Ok(launch)) if self.launch_is_current(launch) => Some(launch),
                Some(_) => {
                    tracing::debug!(event = %hook.event, "a hook from a retired launch; it reaches nothing");
                    let waits = is_fence(hook.agent, &hook.event)
                        || is_gate(hook.agent, &hook.event)
                        || (is_followup(hook.agent, &hook.event) && hook.takes_followup());
                    if waits {
                        write_reply(&mut write, &Reply::verdict(None)).await?;
                    }
                    continue;
                }
            };
            // The fence (`hook_asks`): marked and answered before claude can
            // draw a dialog for this call; the call's end, or a turn's, ends
            // the mark.
            let session = f.session_id.as_deref();
            if let Some(session) = session {
                self.asks.heard(session, fence::raises_dialog(hook.agent, &hook.event, &hook.payload));
            }
            if is_fence(hook.agent, &hook.event) {
                let refused = self.stop_verdict(hook.agent, &hook.payload); // a subagent asked to stop
                fence::answer(&self.asks, session, &hook.payload, &mut write, refused).await?;
            } else if let Some(session) = session {
                // A queued message's `UserPromptSubmit` clears no call: it
                // bounds no turn (`fence::ended`).
                fence::ended(&self.asks, session, &hook.event, &hook.payload);
            }
            self.stopped_ended(hook.agent, &hook.event, &hook.payload);
            let terminal = self.route(&f, hook.agent, launch);
            // Cursor's `stop` waits for this line, on every path (`followup`).
            if is_followup(hook.agent, &hook.event) && hook.takes_followup() {
                let taken = terminal.and_then(|t| {
                    let _held = self.held_for(t, launch)?;
                    self.followup_for(t)
                });
                let reply = match &taken {
                    Some(taken) => Reply::Followup { followup_message: taken.text.clone() },
                    None => Reply::verdict(None),
                };
                // Settled only once written: a hook gone leaves it claimed.
                write_reply(&mut write, &reply).await?;
                if let Some(taken) = taken {
                    (taken.delivered)();
                }
            }
            if is_gate(hook.agent, &hook.event) {
                let Some(terminal) = terminal else {
                    // Said at once. Silence here costs the agent the hook's
                    // whole 400 ms for an ask nobody can be shown: a session
                    // no pane claims, an ambiguous one, or a pane still in
                    // chat mode.
                    tracing::debug!(session = ?f.session_id, "an ask no terminal can take");
                    write_reply(&mut write, &Reply::verdict(None)).await?;
                    continue;
                };
                // A question or a plan only for a hook that prints its
                // answer (`HookLine::takes`); an older one gets the keyboard
                // at once, as before ov-370.
                let tool = hook.payload["tool_name"].as_str();
                if !AskShape::of(tool, &serde_json::Value::Null).is_permission() && !hook.takes_updated_input() {
                    write_reply(&mut write, &Reply::verdict(None)).await?;
                    continue;
                }
                return self.hold_ask(terminal, launch, hook, f, reader, write).await;
            }
            let Some(terminal) = terminal else {
                tracing::debug!(session = ?f.session_id, "a hook from a session no terminal claims");
                continue;
            };
            // Everything from here acts on the pane, so it's done while the
            // pane's launch is still this frame's (`held_for`).
            let Some(held) = self.held_for(terminal, launch) else {
                tracing::debug!(terminal = %terminal, "the pane was relaunched under this hook; it reaches nothing");
                continue;
            };
            // A turn cannot end or begin with claude's dialog up, so whatever
            // was asked on this pane has been answered at the keyboard.
            // A queued message's too: it was typed into the box, so no dialog
            // was up either.
            if fence::bounds_turn(hook.agent, &hook.event, &hook.payload) {
                self.asks.turn_boundary(terminal);
            }
            if let ("PostToolUse" | "PostToolUseFailure", Some(tool)) = (hook.event.as_str(), hook.payload["tool_name"].as_str()) {
                self.asks.dialog_call_ended(terminal, tool);
            }
            self.start_transcript_tail(terminal, hook.agent, &f);

            let events =
                self.accept(terminal, hook.agent, &hook.event, &hook.payload, f.session_id.as_deref());
            if !events.is_empty() {
                on_events(terminal, events);
            }
            drop(held);
            // A store read, and a write when it claims. Nothing waits on it
            // here: a hook that isn't gating hangs up once it has written. The
            // gating hook, which does wait, is `hold_ask`'s, and there this
            // runs after the reply. Outside the launch gate, since it can wait
            // on the claims ledger.
            self.observe_cwd(terminal, hook.agent, &f);
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
        launch: Option<Uuid>,
        hook: HookLine,
        f: Facts,
        mut reader: BufReader<OwnedReadHalf>,
        mut write: OwnedWriteHalf,
    ) -> std::io::Result<()> {
        // The tool's name rides on the lock screen's card ("Bash · Billing");
        // `push::WireAsk` decides whether it may.
        let tool = hook.payload["tool_name"].as_str();
        let shape = AskShape::of(tool, &hook.payload["tool_input"]);
        let permission = shape.is_permission();
        // Held while the pane's launch is still this frame's, under the launch
        // gate, so a relaunch either finds it to settle or comes first.
        let held = {
            let Some(_held) = self.held_for(terminal, launch) else {
                return write_reply(&mut write, &Reply::verdict(None)).await;
            };
            self.asks.hold_shaped(terminal, tool, shape, self.hold)
        };
        let (id, mut settled) = held;
        if let Err(e) = write_reply(&mut write, &Reply::hold(self.hold)).await {
            // The hook missed its deadline and has gone. Nobody was offered
            // this ask, so it ends with nothing to take back.
            self.asks.withdraw(terminal, &id);
            return Err(e);
        }
        // `false` when something ended it already; `settled` then has that
        // ending, and the select below writes it at once.
        //
        // A question or a plan is held but offered to no surface that
        // answers with Allow and Deny (`AskShape::is_permission`): it is
        // answered from its row in a conversation view, which reads the
        // hold's id from the request folded below (ov-370).
        if permission {
            self.asks.offer(terminal, &id, permission_ask(&id, &hook.payload));
        }
        let mut payload = hook.payload;
        payload[HELD_ASK] = serde_json::Value::String(id.clone());
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let Some(held) = this.held_for(terminal, launch) else { return };
            this.start_transcript_tail(terminal, hook.agent, &f);
            let events = this.accept(terminal, hook.agent, &hook.event, &payload, f.session_id.as_deref());
            let sink = this.sink.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let (false, Some(sink)) = (events.is_empty(), sink) {
                sink(terminal, events);
            }
            drop(held);
            this.observe_cwd(terminal, hook.agent, &f);
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

/// A pane still in chat mode, waiting to move to its TUI (`chat_migration`).
///
/// **One ring per terminal, fed by one transport.** A chat pane's
/// conversation came over its shim, never its hooks; a hook routed to it as
/// well put every assistant turn in the ring twice. Chat mode is retired
/// (ov-377) and nothing reads the shim now, but a pane left in chat mode by
/// the build before keeps its shim until its next idle, and its hooks stay
/// dropped until then: the TUI it moves to reads the same conversation back
/// from its transcript, so a turn taken in from a hook first would be shown
/// twice after all.
///
/// Both of `terminal_for`'s routes are guarded: the announcement route, which
/// codex's and cursor's project-local hooks take, and the claimants route,
/// which a chat pane reaches whenever it carries an `agent_session_id`.
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
#[path = "hook_ingress_tests.rs"]
mod tests;

mod fence;
pub(crate) mod followup;
mod refusals;
pub(crate) mod stopping;
mod writer;
use refusals::{Refusals, transient};
#[cfg(test)]
use refusals::REFUSAL_REPORT_EVERY;
#[cfg(test)]
#[path = "hook_fence_tests.rs"]
mod hook_fence_tests;

#[cfg(test)]
#[path = "hook_ingress_held_tests.rs"]
mod held_tests;

#[cfg(test)]
#[path = "hook_ingress_launch_tests.rs"]
mod launch_tests;
