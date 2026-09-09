//! The daemon's half of every agent session.
//!
//! **It owns the transcript.** `recent` is the conversation, not a cache in
//! front of one: it is held in memory here, bounded by `TRANSCRIPT_LIMIT`, and
//! nothing asks the shim for anything older because the shim has nothing older
//! to give. It has to outlive the shim, which restarts on every pane-mode
//! toggle while the conversation does not.
//!
//! The consequence, said here because this is the first place anyone reads:
//! **a daemon restart discards every agent conversation on this runner.**
//! Terminals do not work that way and the difference is easy to assume away —
//! they are tmux panes, and `runtime.rs` rebuilds a pane's replay from tmux's
//! own scrollback at attach, so they come back from tmux rather than from
//! anything held here. Nothing rebuilds a transcript. Whatever was typed and
//! not yet sent goes too, which is what `ToggleRefusal::TurnInFlight` below
//! already refuses a toggle over. So restarting the daemon — to install an
//! update, say — is not free, and anything that offers to do it has to say so
//! first. The Mac app's `DaemonSkew` is written against this paragraph.
//!
//! (This module doc used to claim the opposite: "It owns no transcript. The
//! shim holds the ring ... so a daemon restart costs no history and needs no
//! `session/load`." That was left over from the design this file replaced —
//! `TRANSCRIPT_LIMIT` has flagged the same leftover just below for as long as
//! it has existed — and it is the more dangerous of the two contradictory
//! claims, because it is the one on top and it says a restart is safe.)
//!
//! What else lives here is the bookkeeping only the daemon can do: which
//! terminals are in agent pane mode, what each one's activity is, and fanning
//! events out to however many clients are watching.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use farcooler_agent::event::{AgentEvent, AgentGapReason, Seq, Sequenced};
use farcooler_agent::link::{AgentFailure, DaemonMessage, ShimMessage, decode_line, encode_line};
use farcooler_agent::activity_source;
use farcooler_core::activity;
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::Store;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use uuid::Uuid;

/// How much of a conversation the daemon keeps per terminal.
///
/// The daemon owns this transcript outright — it is not a cache in front of the
/// shim's ring, and nothing asks the shim for anything older. It has to outlive
/// the shim, because the shim restarts on every pane-mode toggle and the
/// conversation does not.
///
/// So this bound is where a conversation actually ends. Past it the front is
/// dropped and a `Gap` takes its place, which is the only honest way to serve a
/// transcript that no longer starts at the beginning.
///
/// (This carried a second, contradictory doc comment describing it as
/// "deliberately small" next to the shim's ring, with older events being "the
/// shim's ring to replay" — left over from the design this file replaced. There
/// is no such fallback, and there never was one in this direction.)
const TRANSCRIPT_LIMIT: usize = 4096;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ToggleRefusal {
    /// Naming the queue as well as the turn, because both are lost.
    ///
    /// The shim holds unsent prompts in memory (`RunningSession::queue`) and
    /// dies with the pane, so forcing a switch discards anything written and
    /// not yet delivered along with the turn in progress. The message used to
    /// mention only the turn, which meant a user could force the switch having
    /// been warned about the wrong thing — they lose words they wrote, not just
    /// work the agent was doing.
    #[error("a turn is in flight, and any queued messages will be discarded with it; cancel it or force the switch")]
    TurnInFlight,
}

/// Where a terminal's shim dials.
///
/// Per terminal and per runtime directory, so two daemons on one host never
/// collide and a stale socket never adopts a new session.
///
/// SHORT, and that is not tidiness. A Unix socket path cannot exceed
/// `sun_path` — 104 bytes on macOS — and the default runtime directory is
/// `~/Library/Application Support/com.farcooler.Far Cooler`, which is already
/// 66 of them. A full uuid took the total to 113: `bind` failed, the daemon
/// never listened, the shim dialled a socket nobody was on, and the chat sat
/// blank forever with the pane cheerfully reporting "connected". It worked in
/// every test because test runtime directories are short.
///
/// The last 12 hex digits of a v7 uuid are its random tail — the same bytes
/// `short` shows a user — so this is as collision-resistant as the ids people
/// already type at the CLI, in 18 characters instead of 47.
pub fn socket_path(runtime_dir: &Path, terminal: Uuid) -> PathBuf {
    let hex = terminal.simple().to_string();
    runtime_dir.join(format!("a-{}.sock", &hex[hex.len() - 12..]))
}

/// The longest path `bind` will accept, minus a byte for the NUL.
///
/// Named rather than inlined so the test that guards it and the code it
/// guards cannot drift apart.
pub const MAX_SOCKET_PATH: usize = 103;

/// Apply one event to a terminal's activity.
///
/// The observation is `farcooler_agent`'s; the FOLD is `core::activity`'s, and
/// deliberately so. `Done` must mean the same thing whether it came from a
/// screen or from a protocol, or a Mac badge and a phone notification will
/// disagree about the same terminal.
pub fn fold_activity(current: AgentActivity, event: &AgentEvent) -> AgentActivity {
    match activity_source::observe(event) {
        Some(observed) => activity::advance(current, observed),
        None => current,
    }
}

/// Whether a pane-mode toggle may proceed.
pub fn guard_toggle(current: AgentActivity, force: bool) -> Result<(), ToggleRefusal> {
    if force {
        return Ok(());
    }
    match current {
        AgentActivity::Working => Err(ToggleRefusal::TurnInFlight),
        _ => Ok(()),
    }
}

#[derive(Debug, Default)]
struct SessionState {
    activity: AgentActivity,
    session_id: Option<String>,
    agent_mode: Option<String>,
    /// What the agent calls this conversation.
    ///
    /// Kept because it is the only description of a pane that describes the
    /// WORK. Every other name available — the preset, the process, the harness
    /// — says what is running, and a fleet of eight panes all called "claude"
    /// tells a user nothing about which is which.
    title: Option<String>,
    available_modes: Vec<String>,
    /// Why this pane has no agent in it, when it has none.
    ///
    /// A stable word from the shim, held so every client that asks gets the
    /// same answer. `None` is "nothing has said this pane failed" — which for
    /// a pane that is starting normally is also what it looks like, so a
    /// client draws the spinner until this is set or the transcript arrives.
    failure: Option<AgentFailure>,
    /// Which run of the shim this transcript belongs to.
    ///
    /// The same idea as a terminal's `epoch`, and for the same reason. A shim
    /// numbers events by their position in ITS ring, and every pane-mode
    /// toggle starts a new shim counting from zero — so a cursor a client
    /// holds is meaningless the moment that happens. Rather than trying to
    /// reconcile the two numberings (which failed four different ways), the
    /// stream simply admits it is a new stream: the epoch changes, and every
    /// reader knows to take the whole thing again.
    epoch: u64,
}

#[derive(Clone, Default)]
pub struct AgentSupervisor {
    sessions: Arc<Mutex<HashMap<Uuid, SessionState>>>,
    writers: Arc<Mutex<HashMap<Uuid, tokio::sync::mpsc::UnboundedSender<DaemonMessage>>>>,
    /// The fast-attach window described at the top of this file. Bounded to
    /// `RECENT_WINDOW` per terminal, oldest first.
    recent: Arc<Mutex<HashMap<Uuid, Vec<Sequenced>>>>,
    /// Terminals whose socket is already bound.
    ///
    /// Without this, a second `set_pane_mode` would bind the same path again
    /// and the shim's reconnect would land on whichever listener won — so a
    /// session's events would arrive at a supervisor nobody is reading.
    listening: Arc<Mutex<HashSet<Uuid>>>,
    /// Where a conversation's NAME is written down, when there is a store to
    /// write it to.
    ///
    /// This does not contradict the transcript rule at the top of this file.
    /// The transcript is runtime state and stays in memory; an
    /// `agent_session_id` is intent, in the same sense as `pane_mode` and
    /// `command_preset` — `store.rs`'s own column test says so — and it is the
    /// only thing that lets a pane be reopened onto the conversation it was
    /// already having.
    ///
    /// `None` for a supervisor nobody gave a store to, which is every test
    /// that is not about this write. A supervisor without one behaves exactly
    /// as it did before, so the tests either side of this remain about what
    /// they were about.
    records: Option<Arc<Store>>,
}

impl AgentSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    /// The supervisor a daemon runs: one that can write down what it learns.
    pub fn with_records(store: Arc<Store>) -> Self {
        Self { records: Some(store), ..Self::default() }
    }

    /// Write down the conversation a shim says this pane actually ended up in.
    ///
    /// The shim's `Established` id is the only true one — `session/load` can
    /// fail and the adapter opens a fresh conversation instead — and until
    /// this existed it lived in the map above and nowhere else. It reached
    /// SQLite only on the NEXT `set_pane_mode`, so a pane switched into chat
    /// once and then lost carried its conversation in memory to the grave: a
    /// restart read a NULL column and started a new one.
    ///
    /// Whatever the preset, deliberately. A pane on the `shell` preset that a
    /// person typed `claude` into and then opened as a chat is exactly the
    /// case with no id from anywhere else — `create_terminal` mints one only
    /// for a preset that starts with `claude`, and adoption finds one only
    /// when a transcript already exists that is newer than the process in the
    /// pane. This is also the only way codex or cursor ever gets a real id,
    /// since neither can be handed one at launch.
    ///
    /// Reuses `store.set_pane_mode` because it is the only write that can
    /// reach the column at all — `update_terminal` does not name it — and its
    /// `COALESCE(?2, agent_session_id)` means an id is never cleared by
    /// passing `None`. The mode is written back as it was read, so this
    /// changes one column and the version.
    ///
    /// Failures are logged and dropped rather than retried. A version conflict
    /// means somebody else wrote the row between the read and the write, and
    /// the next `Established` or the next toggle writes it again; a shim
    /// reporting a session is not a place to block or to surface an error to
    /// anyone.
    fn remember_session(&self, terminal: Uuid, session_id: &str) {
        let Some(store) = self.records.as_ref() else { return };
        let Ok(term) = store.get_terminal(terminal) else { return };
        // The row already says this. Skipped so that a reconnecting shim
        // re-reporting the same session does not bump `resource_version` and
        // wake every watching client for no change.
        if term.agent_session_id.as_deref() == Some(session_id) {
            return;
        }
        if let Err(e) = store.set_pane_mode(
            terminal,
            term.resource_version,
            term.pane_mode,
            Some(session_id.to_string()),
            // The pane is not replaced by learning its conversation's name.
            // Bumping the epoch here would hand every attached client a full
            // re-read of a terminal that did not change.
            false,
        ) {
            tracing::warn!(
                terminal = %terminal,
                session = %session_id,
                error = %e,
                "could not write down which conversation this pane is in"
            );
        }
    }

    pub fn activity(&self, terminal: Uuid) -> AgentActivity {
        self.sessions
            .lock()
            .ok()
            .and_then(|s| s.get(&terminal).map(|st| st.activity))
            .unwrap_or(AgentActivity::Unspecified)
    }

    pub fn session_id(&self, terminal: Uuid) -> Option<String> {
        self.sessions.lock().ok().and_then(|s| s.get(&terminal).and_then(|st| st.session_id.clone()))
    }

    /// What the agent has named this conversation, if it has named it.
    pub fn title(&self, terminal: Uuid) -> Option<String> {
        self.sessions.lock().ok().and_then(|s| s.get(&terminal).and_then(|st| st.title.clone()))
    }

    pub fn agent_mode(&self, terminal: Uuid) -> Option<String> {
        self.sessions.lock().ok().and_then(|s| s.get(&terminal).and_then(|st| st.agent_mode.clone()))
    }

    /// Why this pane has no agent in it, as a stable word, when it has none.
    pub fn failure(&self, terminal: Uuid) -> Option<AgentFailure> {
        self.sessions.lock().ok().and_then(|s| s.get(&terminal).and_then(|st| st.failure))
    }

    pub fn available_modes(&self, terminal: Uuid) -> Vec<String> {
        self.sessions
            .lock()
            .ok()
            .map(|s| s.get(&terminal).map(|st| st.available_modes.clone()).unwrap_or_default())
            .unwrap_or_default()
    }

    /// Events at and after `from_seq`, from the daemon's recent window.
    ///
    /// Empty for a terminal with no session — attaching to a pane that is not
    /// in agent mode is not an error, it just has nothing to show yet.
    /// The transcript this terminal has, and which run of it that is.
    ///
    /// A reader passes the epoch it last saw. If it does not match, the cursor
    /// it holds counts positions in a stream that no longer exists, so the
    /// whole transcript comes back and the reader replaces what it had. Within
    /// one epoch the cursor means what it says.
    pub fn replay(&self, terminal: Uuid, from_seq: Seq, client_epoch: u64) -> (u64, Vec<Sequenced>) {
        let epoch = self
            .sessions
            .lock()
            .ok()
            .and_then(|s| s.get(&terminal).map(|st| st.epoch))
            .unwrap_or(0);
        let all: Vec<Sequenced> = self
            .recent
            .lock()
            .ok()
            .and_then(|r| r.get(&terminal).cloned())
            .unwrap_or_default();

        if client_epoch != epoch {
            return (epoch, all);
        }
        (epoch, all.into_iter().filter(|e| e.seq >= from_seq).collect())
    }

    /// Hand a message to this terminal's shim, and say whether it got there.
    ///
    /// This returned nothing at all, which made every caller's silence look
    /// like a delivery. The one that matters is `terminal.agent_prompt`: it
    /// dropped the words a person had typed and replied with the terminal read
    /// back, so the message simply vanished out of the composer with nothing
    /// anywhere saying it had.
    ///
    /// Both ways of missing are reported. No writer is a pane whose shim has
    /// not dialed yet — or has stopped being a chat, see `left_agent_mode`.
    /// A writer whose `send` fails is a `serve` loop that has already returned
    /// and dropped its receiver: the entry is stale, and treating it as a
    /// delivery is the same lie with one more step in it.
    #[must_use = "a message the shim never got is not a message that was sent"]
    pub fn send(&self, terminal: Uuid, message: DaemonMessage) -> bool {
        let Ok(writers) = self.writers.lock() else { return false };
        let Some(tx) = writers.get(&terminal) else { return false };
        tx.send(message).is_ok()
    }

    /// Accept the shim for one terminal and pump it until the pane dies.
    ///
    /// `on_events` is how the daemon fans out; it is a callback rather than a
    /// channel so that the existing event bus stays the only fanout in the
    /// process.
    /// Start accepting this terminal's shim, once.
    ///
    /// Nothing worked until this existed. `listen` was written, tested and
    /// never called, so the socket was never bound: every shim retried
    /// `connect` forever, no events reached the daemon, and a client polling
    /// `agent_subscribe` got an empty batch from a session that was in fact
    /// running perfectly. The whole feature was inert and nothing said so.
    ///
    /// Idempotent, because both the pane-mode switch and daemon startup
    /// legitimately want to guarantee a listener exists.
    pub fn ensure_listening(&self, runtime_dir: &Path, terminal: Uuid) {
        {
            let Ok(mut listening) = self.listening.lock() else { return };
            if !listening.insert(terminal) {
                return;
            }
        }
        let this = self.clone();
        let dir = runtime_dir.to_path_buf();
        tokio::spawn(async move {
            if let Err(e) = this.listen(&dir, terminal, |_, _| {}).await {
                tracing::warn!(terminal = %terminal, error = %e, "agent listener stopped");
            }
            // Released so a later switch back into agent mode can bind again.
            if let Ok(mut listening) = this.listening.lock() {
                listening.remove(&terminal);
            }
        });
    }

    pub async fn listen<F>(
        &self,
        runtime_dir: &Path,
        terminal: Uuid,
        on_events: F,
    ) -> std::io::Result<()>
    where
        F: Fn(Uuid, Vec<Sequenced>) + Send + 'static,
    {
        let path = socket_path(runtime_dir, terminal);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).inspect_err(|e| {
            // Loud, because the symptom is silence. A failed bind here leaves
            // the shim dialling a socket nobody is on, the pane reporting
            // "connected", and the chat blank — with nothing anywhere saying
            // why. Path length is the cause worth naming first.
            tracing::error!(
                error = %e,
                path = %path.display(),
                bytes = path.as_os_str().len(),
                limit = MAX_SOCKET_PATH,
                "could not bind the agent socket; this terminal's chat will stay empty"
            );
        })?;

        loop {
            let (stream, _) = listener.accept().await?;
            // No cursor is read here, and that is the point: a connection is a
            // new stream and the only honest place to resume one is the start.
            // This used to look up the remembered cursor and hand it down, and
            // `serve` discarded it — residue of the design that tried to
            // reconcile two numberings and failed four different ways.
            if let Err(e) = self.serve(stream, terminal, &on_events).await {
                tracing::warn!(terminal = %terminal, error = %e, "agent shim link ended");
            }
        }
    }

    async fn serve<F>(
        &self,
        stream: UnixStream,
        terminal: Uuid,
        on_events: &F,
    ) -> std::io::Result<()>
    where
        F: Fn(Uuid, Vec<Sequenced>) + Send + 'static,
    {
        let (read_half, mut write_half) = stream.into_split();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DaemonMessage>();
        if let Ok(mut writers) = self.writers.lock() {
            writers.insert(terminal, tx);
        }

        // From the beginning, every time, and the window is dropped first.
        //
        // `seq` is a position in a SHIM's ring, not in the conversation, so a
        // respawned pane starts counting at zero again with a shorter ring.
        // Asking it to resume from a cursor this daemon remembered — 40, say,
        // into a ring that now holds 30 — returns nothing at all, and every
        // message sent before the toggle simply disappeared. A connection is a
        // new stream; the only honest cursor for one is 0.
        //
        // There is nothing to reset beside the window any more: this used to
        // keep a `cursor` per session as well, and nothing ever read it.
        if let Ok(mut recent) = self.recent.lock() {
            recent.remove(&terminal);
        }
        let subscribe = encode_line(&DaemonMessage::Subscribe { from_seq: 0 })
            .unwrap_or_else(|_| "\n".to_string());
        write_half.write_all(subscribe.as_bytes()).await?;

        let mut lines = BufReader::new(read_half).lines();
        loop {
            tokio::select! {
                outgoing = rx.recv() => {
                    let Some(message) = outgoing else { return Ok(()) };
                    if let Ok(line) = encode_line(&message) {
                        write_half.write_all(line.as_bytes()).await?;
                    }
                }
                line = lines.next_line() => {
                    let Some(line) = line? else { return Ok(()) };
                    let Ok(message) = decode_line::<ShimMessage>(&line) else { continue };
                    self.apply(terminal, message, on_events);
                }
            }
        }
    }

    fn apply<F>(&self, terminal: Uuid, message: ShimMessage, on_events: &F)
    where
        F: Fn(Uuid, Vec<Sequenced>) + Send + 'static,
    {
        let batch = match message {
            ShimMessage::Events { events } => events,
            // The shim's own ring overflowed before the daemon could read it.
            //
            // `AgentReplay::Gap` reports the drop but carries no event saying
            // so, and this used to forward the events alone — which is how a
            // client came to hold a transcript missing history with nothing to
            // mark it. The gap is prepended here, where the loss is known.
            ShimMessage::Trimmed { resumed_at, dropped, events } => {
                tracing::info!(terminal = %terminal, resumed_at, dropped, "agent ring trimmed");
                let mut with_gap = vec![Sequenced {
                    seq: 0,
                    event: AgentEvent::Gap { reason: AgentGapReason::RingTrimmed },
                }];
                with_gap.extend(events);
                with_gap
            }
            ShimMessage::Established { session_id, available_modes } => {
                // A new shim means a new stream, numbered from zero again.
                //
                // `seq` belongs to a shim's ring, not to the conversation, so
                // a respawned pane starts counting at 0 while this window
                // still holds events numbered far higher. The dedupe below
                // then discards everything new as "already seen", and a client
                // reading from its own cursor asks for events past the end of
                // a stream that just restarted — so a toggle looked like it
                // erased every message sent before it.
                if let Ok(mut recent) = self.recent.lock() {
                    recent.remove(&terminal);
                }
                // Persisted here, at the moment it is known, and not only in
                // the map below. See `remember_session`. Done before the
                // `sessions` lock is taken rather than inside it: this is a
                // SQLite write, and no lock in this file is worth holding
                // across one.
                self.remember_session(terminal, &session_id);
                if let Ok(mut sessions) = self.sessions.lock() {
                    let entry = sessions.entry(terminal).or_default();
                    entry.session_id = Some(session_id);
                    entry.available_modes = available_modes;
                    // A session that established is not a session that failed.
                    // A pane that failed, was toggled back to a terminal and
                    // toggled in again would otherwise keep reporting the old
                    // failure over a chat that is working.
                    entry.failure = None;
                    // Nor is it mid-turn. A shim that has just finished its
                    // handshake has done nothing yet, and the activity here is
                    // whatever the pane's LAST life left behind — the same
                    // stale `Working` the `Failed` arm below already resets,
                    // arrived at down the other road.
                    //
                    // It costs more than a wrong badge. `guard_toggle` refuses
                    // a switch out of agent mode while activity says
                    // `Working`, so a pane whose previous shim died mid-turn
                    // came back with a perfectly good agent in it that could
                    // not be switched away from without forcing, over a turn
                    // that ended with a process that no longer exists.
                    //
                    // `Unspecified` rather than `Idle`: nothing has been
                    // observed of this session yet, and `Idle` is a claim that
                    // it is waiting for a person. The first event the shim
                    // sends folds it onward from here.
                    entry.activity = AgentActivity::Unspecified;
                    // A new shim is a new stream. Readers holding a cursor into
                    // the old one are told by the change, rather than being
                    // left to work it out from numbers that silently mean
                    // something else now.
                    entry.epoch += 1;
                }
                return;
            }
            ShimMessage::Failed { failure } => {
                // Recorded, not merely logged. This handler was dead code —
                // nothing anywhere constructed `Failed` — and a warning in a
                // log is not a state a client can render, so the pane showed a
                // spinner forever whatever the daemon knew.
                //
                // The pane STAYS in agent mode. Flipping it back to
                // `PaneMode::Terminal` from here would respawn the pane under
                // whatever the user was typing into it; a client renders this
                // word as a failure row and offers the switch as an action the
                // user chooses.
                tracing::warn!(
                    terminal = %terminal,
                    failure = failure.code(),
                    "this pane is in agent mode with no agent in it"
                );
                if let Ok(mut sessions) = self.sessions.lock() {
                    let entry = sessions.entry(terminal).or_default();
                    entry.failure = Some(failure);
                    // An agent that never started is not working, and the
                    // activity left over from the pane's last life would
                    // otherwise refuse the toggle that gets the user out of
                    // here — see `guard_toggle`.
                    entry.activity = AgentActivity::Unspecified;
                }
                return;
            }
        };

        // The shim's numbering is dropped here, and nothing is kept in its
        // place. It counts positions in THAT shim's ring rather than in this
        // transcript — the distinction the epoch above exists for — and
        // `record` works the position out from the transcript itself.
        self.record(terminal, batch.into_iter().map(|s| s.event).collect(), on_events);
    }

    /// Fold, number and fan out a batch of events for one terminal.
    ///
    /// Split out of `apply` so the hook ingress and the shim share it. They are
    /// two transports for one conversation and must not number two rings — see
    /// the epoch discussion above for what happens when a cursor means more
    /// than one thing.
    ///
    /// Takes bare events rather than `Sequenced`, because only one of the two
    /// transports has a number to offer and neither number is the one this
    /// keeps: the position in this transcript is worked out below.
    pub fn record<F>(&self, terminal: Uuid, events: Vec<AgentEvent>, on_events: &F)
    where
        F: Fn(Uuid, Vec<Sequenced>),
    {
        if let Ok(mut sessions) = self.sessions.lock() {
            let entry = sessions.entry(terminal).or_default();
            for event in &events {
                entry.activity = fold_activity(entry.activity, event);
                // `SessionStarted` and `ModeSet` are the only events that name
                // the ACP mode; anything else leaves it as it was.
                match event {
                    AgentEvent::SessionStarted { agent_mode, available_modes, .. } => {
                        if agent_mode.is_some() {
                            entry.agent_mode = agent_mode.clone();
                        }
                        if !available_modes.is_empty() {
                            // Ids only: this feeds the proto's repeated-string
                            // field. The human names ride on the event itself,
                            // which is what the pickers read.
                            entry.available_modes =
                                available_modes.iter().map(|m| m.id.clone()).collect();
                        }
                    }
                    AgentEvent::ModeSet { agent_mode } => {
                        entry.agent_mode = Some(agent_mode.clone());
                    }
                    AgentEvent::SessionInfo { title } if !title.is_empty() => {
                        entry.title = Some(title.clone());
                    }
                    _ => {}
                }
            }
        }

        let mut renumbered: Vec<Sequenced> = Vec::new();
        if let Ok(mut recent) = self.recent.lock() {
            let entry = recent.entry(terminal).or_default();
            // Numbered by this transcript's own length.
            //
            // The daemon is the only thing that numbers these, so a number
            // means one position in one transcript and nothing else. The shim
            // renumbers from zero every time it restarts, which is what made
            // every cursor in the system a lie after a toggle; the epoch above
            // is what tells a reader that happened, and there is nothing left
            // here to deduplicate against.
            let base = entry.len() as u64;
            renumbered = events
                .into_iter()
                .enumerate()
                .map(|(i, event)| Sequenced { seq: base + i as u64, event })
                .collect();
            entry.extend(renumbered.iter().cloned());

            // Oldest first, so trimming the front keeps the most recent
            // `TRANSCRIPT_LIMIT`.
            //
            // The trim leaves a `Gap` behind, and that is not decoration. This
            // window is renumbered by position, so dropping the front erases
            // every trace that anything was there — a client would receive a
            // shorter transcript with contiguous numbers and no reason to
            // doubt it. A derived transcript is only defensible because it can
            // say where it is incomplete; silently losing history is the one
            // thing this design forbids.
            if entry.len() > TRANSCRIPT_LIMIT {
                let excess = entry.len() - TRANSCRIPT_LIMIT;
                entry.drain(0..excess);
                entry[0] = Sequenced {
                    seq: 0,
                    event: AgentEvent::Gap { reason: AgentGapReason::RingTrimmed },
                };
                // Renumbered from the gap forward, so the numbers still mean
                // "position in this transcript" — the property every cursor in
                // the system depends on.
                for (index, item) in entry.iter_mut().enumerate() {
                    item.seq = index as u64;
                }
                // What was just handed out is renumbered too, or a live
                // subscriber's next cursor would point past the end.
                renumbered = entry[entry.len().saturating_sub(renumbered.len())..].to_vec();
            }
        }

        on_events(terminal, renumbered);
    }

    /// Drop everything held for a terminal whose record is gone.
    ///
    /// The counterpart to `record`, which creates both of these through
    /// `or_default()` for whatever terminal it is handed. On the shim path
    /// that was only ever a pane somebody had toggled into agent mode; the
    /// hook path hands it every terminal a live session routes to, which is
    /// most of them. A row and up to `TRANSCRIPT_LIMIT` events each, for the
    /// life of the daemon, and every one of them answers correctly the whole
    /// time — it is simply never asked again. `HookIngress::forget` is called
    /// from the same line of the same delete path, against the same class of
    /// leak.
    ///
    /// Not `Established`'s job and not covered by it: that clears the window
    /// because a shim restarted and its numbering began again, which is a
    /// different event from a terminal ending, and on the hook path no shim
    /// ever establishes anything.
    ///
    /// `writers` goes too. It is inserted in `serve` and removed nowhere else,
    /// and a terminal whose record is deleted is one nothing can address.
    ///
    /// `listening` deliberately stays. It stands for a task that is still
    /// running — nothing cancels the spawned `listen` — so clearing the flag
    /// would advertise a socket path that is still bound. Terminal ids are
    /// never reused, so nothing will ask to bind that path again, and what is
    /// left behind is one `Uuid` and one task blocked on `accept` for a socket
    /// no shim will ever dial.
    pub fn forget(&self, terminal: Uuid) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&terminal);
        }
        if let Ok(mut recent) = self.recent.lock() {
            recent.remove(&terminal);
        }
        if let Ok(mut writers) = self.writers.lock() {
            writers.remove(&terminal);
        }
    }

    /// A pane has stopped being a chat.
    ///
    /// Everything the supervisor holds for a terminal used to survive the
    /// toggle out of agent mode, because nothing anywhere told it that the
    /// shim it was describing had been killed. `Service::set_pane_mode`
    /// respawns the pane as a TUI and the shim dies with it; the map went on
    /// answering for it.
    ///
    /// The activity is the expensive one. A pane forced out of agent mode
    /// mid-turn kept `Working` for a session that no longer exists, and
    /// `guard_toggle` refuses the switch back IN on that word — so the way out
    /// of a stuck chat needed `force` and a warning about discarding a turn
    /// that had already been discarded. `failure` is the same shape: it names
    /// why a shim that is gone could not start one, and a pane switched back
    /// in would draw that old failure over a new chat until `Established`
    /// cleared it. `session_id`, `agent_mode` and `available_modes` all
    /// describe the dead shim's session; the id is safe to drop because the
    /// toggle that got here has just written it to the row.
    ///
    /// `writers` goes for a reason of its own: the channel belongs to that
    /// shim's `serve` loop, and while it is in the map `send` looks like a
    /// delivery. See `send`.
    ///
    /// Three things deliberately stay.
    ///
    /// `epoch` is kept because it must never go backwards. Removing the whole
    /// entry would reset it to 0, and a client holding epoch 1 from the old
    /// stream would then match the NEXT shim's epoch 1 and keep a cursor into
    /// a stream it has never read.
    ///
    /// `recent` — the transcript — is kept because it is the conversation, not
    /// the shim. A client may still read the chat's history while the pane is
    /// showing a terminal, and `serve` clears it when a new shim connects.
    ///
    /// `title` is kept for the same reason: it names the CONVERSATION, which
    /// outlives every shim that has ever hosted it, and `Established` does not
    /// clear it either.
    ///
    /// `listening` stays for the reason `forget` gives above.
    pub fn left_agent_mode(&self, terminal: Uuid) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(entry) = sessions.get_mut(&terminal) {
                entry.activity = AgentActivity::Unspecified;
                entry.failure = None;
                entry.session_id = None;
                entry.agent_mode = None;
                entry.available_modes = Vec::new();
            }
        }
        if let Ok(mut writers) = self.writers.lock() {
            writers.remove(&terminal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent::event::{AgentEvent, EndReason, PermissionOption, Role};
    use farcooler_protocol::v1::{AgentActivity, TerminalIntent};

    #[test]
    fn a_socket_path_is_per_terminal_and_not_guessable_across_daemons() {
        let a = socket_path(Path::new("/run/farcooler"), Uuid::now_v7());
        let b = socket_path(Path::new("/run/farcooler"), Uuid::now_v7());
        assert_ne!(a, b);
        assert!(a.starts_with("/run/farcooler"));
    }

    #[test]
    fn activity_folds_through_core_so_done_still_means_unseen() {
        // The rule that makes a notification worth sending lives in core and is
        // not reimplemented here. Working -> Idle is what produces Done.
        let mut current = AgentActivity::Unspecified;
        current = fold_activity(current, &AgentEvent::Message { role: Role::Agent, text: "x".into(), parent: None });
        assert_eq!(current, AgentActivity::Working);
        current = fold_activity(current, &AgentEvent::TurnEnded { reason: EndReason::EndTurn });
        assert_eq!(current, AgentActivity::Done);
    }

    #[test]
    fn a_refused_turn_stops_the_row_saying_working() {
        // The owner's report, at the rung it is actually visible on: "there's
        // a lot of weird space below the 'Working…' indicator, which is also
        // weird because codex most definitely isn’t working given that it’s
        // requesting us to log in."
        //
        // A backend that cannot run the turn now answers with its own sentence
        // and then `TurnEnded`, in that order. Folded in order the pane passes
        // through Working and lands on Done — finished and unseen, which is
        // what puts it in front of somebody. Reversing those two events would
        // leave the row on Working, so the order is the assertion.
        let mut current = AgentActivity::Working;
        current = fold_activity(current, &AgentEvent::Message {
            role: Role::Agent,
            text: "The agent couldn’t run that turn: Not authenticated.".into(),
            parent: None,
        });
        current = fold_activity(current, &AgentEvent::TurnEnded { reason: EndReason::Refusal });
        assert_eq!(current, AgentActivity::Done, "a stopped agent must not report itself working");
    }

    #[test]
    fn a_permission_request_blocks_the_row_immediately() {
        let e = AgentEvent::Permission {
            id: "r".into(),
            tool_call: "t".into(),
            options: vec![PermissionOption { id: "a".into(), name: "Yes".into(), kind: "allow_once".into() }],
        };
        assert_eq!(fold_activity(AgentActivity::Working, &e), AgentActivity::Blocked);
    }

    #[test]
    fn switching_to_terminal_mode_mid_turn_is_refused_unless_forced() {
        // `claude --resume` cannot attach to a turn in flight, so a quiet
        // switch would discard work the user is watching.
        assert!(matches!(
            guard_toggle(AgentActivity::Working, false),
            Err(ToggleRefusal::TurnInFlight)
        ));
        assert!(guard_toggle(AgentActivity::Working, true).is_ok());
        assert!(guard_toggle(AgentActivity::Idle, false).is_ok());
    }

    #[test]
    fn a_pane_that_failed_can_still_be_switched_back_to_a_terminal() {
        // The way OUT of a failed chat is the toggle to terminal mode, and
        // `guard_toggle` refuses that while activity says `Working`. Activity
        // is written in one place and cleared nowhere, so a pane whose agent
        // died mid-turn and came back unable to start would report `Working`
        // for an agent that does not exist — and refuse the one action that
        // fixes it, with a message about a turn in flight that is not.
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        supervisor.apply(
            terminal,
            ShimMessage::Events {
                events: vec![Sequenced {
                    seq: 0,
                    event: AgentEvent::Message {
                        role: Role::Agent,
                        text: "working on it".into(),
                        parent: None,
                    },
                }],
            },
            &|_, _| {},
        );
        assert_eq!(
            supervisor.activity(terminal),
            AgentActivity::Working,
            "the fixture must start from a turn in flight"
        );

        supervisor.apply(
            terminal,
            ShimMessage::Failed { failure: AgentFailure::AdapterSilent },
            &|_, _| {},
        );

        assert_eq!(supervisor.failure(terminal), Some(AgentFailure::AdapterSilent));
        assert!(
            guard_toggle(supervisor.activity(terminal), false).is_ok(),
            "a pane with no agent in it must not refuse the switch that gets the user out"
        );
    }

    #[test]
    fn a_shim_that_has_just_established_is_not_still_mid_turn() {
        // The other half of the stale-`Working` problem. `Failed` resets the
        // activity; `Established` did not, so a pane whose previous shim died
        // in the middle of a turn came back with a working agent in it and an
        // activity that still said `Working` — and `guard_toggle` refuses the
        // switch out of agent mode on exactly that word, over a turn that
        // ended with a process that no longer exists.
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        supervisor.apply(
            terminal,
            ShimMessage::Events {
                events: vec![Sequenced {
                    seq: 0,
                    event: AgentEvent::Message {
                        role: Role::Agent,
                        text: "half a turn".into(),
                        parent: None,
                    },
                }],
            },
            &|_, _| {},
        );
        assert_eq!(
            supervisor.activity(terminal),
            AgentActivity::Working,
            "the fixture must start from a turn in flight"
        );

        supervisor.apply(
            terminal,
            ShimMessage::Established { session_id: "s".into(), available_modes: Vec::new() },
            &|_, _| {},
        );

        assert_eq!(
            supervisor.activity(terminal),
            AgentActivity::Unspecified,
            "a shim that has just finished its handshake has done nothing yet"
        );
        assert!(
            guard_toggle(supervisor.activity(terminal), false).is_ok(),
            "and the toggle out must not be refused over a turn that is not running"
        );
    }

    #[test]
    fn a_session_that_establishes_stops_reporting_the_failure_before_it() {
        // A pane that failed, was switched back to a terminal and switched in
        // again would otherwise keep drawing a failure row over a chat that is
        // working perfectly.
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        supervisor.apply(
            terminal,
            ShimMessage::Failed { failure: AgentFailure::NoAdapter },
            &|_, _| {},
        );
        assert_eq!(supervisor.failure(terminal), Some(AgentFailure::NoAdapter));

        supervisor.apply(
            terminal,
            ShimMessage::Established { session_id: "s".into(), available_modes: Vec::new() },
            &|_, _| {},
        );
        assert_eq!(supervisor.failure(terminal), None);
    }

    #[test]
    fn leaving_agent_mode_drops_the_dead_shims_state_and_keeps_the_conversations() {
        // Nothing used to tell the supervisor that a toggle out of agent mode
        // had killed the shim, so every word it held went on describing a
        // process that no longer exists.
        //
        // The two halves are asserted together on purpose: what must go, and
        // what must not. Removing the whole entry would take the epoch with
        // it, and an epoch that goes back to 0 lets a client holding 1 from
        // the old stream match the NEXT shim's 1 and keep a cursor into a
        // stream it has never read.
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();

        supervisor.apply(
            terminal,
            ShimMessage::Established {
                session_id: "the-conversation".into(),
                available_modes: vec!["ask".into(), "code".into()],
            },
            &|_, _| {},
        );
        supervisor.record(
            terminal,
            vec![AgentEvent::SessionInfo { title: "porting the parser".into() }],
            &|_, _| {},
        );
        supervisor.apply(
            terminal,
            ShimMessage::Events {
                events: vec![Sequenced {
                    seq: 0,
                    event: AgentEvent::Message {
                        role: Role::Agent,
                        text: "half a turn".into(),
                        parent: None,
                    },
                }],
            },
            &|_, _| {},
        );
        let epoch_before = supervisor.replay(terminal, 0, u64::MAX).0;
        assert_eq!(
            supervisor.activity(terminal),
            AgentActivity::Working,
            "the fixture must start from a turn in flight"
        );
        assert_eq!(supervisor.session_id(terminal).as_deref(), Some("the-conversation"));

        supervisor.left_agent_mode(terminal);

        assert_eq!(
            supervisor.activity(terminal),
            AgentActivity::Unspecified,
            "a pane forced out mid-turn kept `Working` forever, and `guard_toggle` refuses on it"
        );
        assert!(
            guard_toggle(supervisor.activity(terminal), false).is_ok(),
            "so the way back into a chat must not need forcing"
        );
        assert_eq!(supervisor.session_id(terminal), None, "that shim's session is over");
        assert!(
            supervisor.available_modes(terminal).is_empty(),
            "and the modes it offered belonged to its adapter"
        );

        assert_eq!(
            supervisor.replay(terminal, 0, u64::MAX).0,
            epoch_before,
            "the epoch must never go backwards; a client would keep a cursor into another stream"
        );
        assert_eq!(
            supervisor.title(terminal).as_deref(),
            Some("porting the parser"),
            "the title names the conversation, which outlives every shim that hosts it"
        );
        assert_eq!(
            supervisor.replay(terminal, 0, epoch_before).1.len(),
            2,
            "and the transcript is the conversation, still readable while the pane is a terminal"
        );
    }

    #[test]
    fn a_failure_does_not_outlive_the_chat_it_was_reported_against() {
        // The way out of a failed chat is the toggle to terminal mode. If the
        // word survives it, switching back in draws the OLD failure over a new
        // chat until `Established` happens to clear it — which for a chat that
        // is starting normally is precisely the window the user is watching.
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        supervisor.apply(
            terminal,
            ShimMessage::Failed { failure: AgentFailure::NotAuthenticated },
            &|_, _| {},
        );
        assert_eq!(supervisor.failure(terminal), Some(AgentFailure::NotAuthenticated));

        supervisor.left_agent_mode(terminal);

        assert_eq!(supervisor.failure(terminal), None);
    }

    #[test]
    fn a_message_only_counts_as_sent_when_something_is_there_to_take_it() {
        // `send` returned nothing, so both ways of missing looked exactly like
        // a delivery to every caller. The RPC layer then answered a prompt
        // nobody received with the terminal read back — the success reply.
        //
        // Two ways of missing, and the second is the quieter one: a `serve`
        // loop that has already returned leaves its entry in the map with the
        // receiving end of the channel dropped, so the lookup succeeds and the
        // push does not.
        let supervisor = AgentSupervisor::new();
        let connected = Uuid::now_v7();
        let never_dialed = Uuid::now_v7();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        supervisor.writers.lock().unwrap().insert(connected, tx);

        assert!(
            supervisor.send(connected, DaemonMessage::Cancel),
            "a shim on the other end is the case this all exists for"
        );
        assert!(
            !supervisor.send(never_dialed, DaemonMessage::Cancel),
            "a pane whose shim has not dialed received nothing"
        );

        drop(rx);
        assert!(
            !supervisor.send(connected, DaemonMessage::Cancel),
            "and neither did one whose serve loop has already returned"
        );
    }

    #[test]
    fn a_socket_path_fits_in_a_unix_socket() {
        // The failure this prevents was invisible: `bind` returns an error the
        // daemon logs and moves on from, the shim keeps dialling, and the only
        // symptom is a chat that never fills in. Measured against the real
        // default runtime directory, which is the one that broke.
        let real = Path::new("/Users/some-long-user-name/Library/Application Support/com.farcooler.FarCooler");
        let path = socket_path(real, Uuid::now_v7());
        assert!(
            path.as_os_str().len() <= MAX_SOCKET_PATH,
            "{} bytes is too long for a unix socket: {}",
            path.as_os_str().len(),
            path.display()
        );
    }

    #[test]
    fn two_terminals_do_not_share_a_socket() {
        let dir = Path::new("/run/farcooler");
        assert_ne!(socket_path(dir, Uuid::now_v7()), socket_path(dir, Uuid::now_v7()));
    }

    #[test]
    fn a_respawned_shim_restarts_the_window_rather_than_being_deduped_away() {
        // `seq` is a shim's ring position, not a place in the conversation, so
        // a respawn counts from zero again. Without clearing, the dedupe reads
        // those as already-seen and drops them, and a toggle appears to erase
        // everything said before it.
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        let batch = |texts: &[&str]| ShimMessage::Events {
            events: texts
                .iter()
                .enumerate()
                .map(|(i, t)| Sequenced {
                    seq: i as u64,
                    event: AgentEvent::Message { role: Role::Agent, text: (*t).into(), parent: None },
                })
                .collect(),
        };

        supervisor.apply(terminal, batch(&["one", "two", "three"]), &|_, _| {});
        assert_eq!(supervisor.replay(terminal, 0, u64::MAX).1.len(), 3);

        // The pane is toggled: a new shim announces itself and starts over.
        supervisor.apply(
            terminal,
            ShimMessage::Established {
                session_id: "s".into(),
                available_modes: Vec::new(),
            },
            &|_, _| {},
        );
        supervisor.apply(terminal, batch(&["fresh"]), &|_, _| {});

        let (_, replayed) = supervisor.replay(terminal, 0, u64::MAX);
        assert_eq!(replayed.len(), 1, "the new stream must not be deduped away");
    }

    #[test]
    fn a_reconnect_asks_from_the_beginning_rather_than_a_remembered_cursor() {
        // The window is dropped on connect so a fresh stream refills it. Asking
        // a respawned shim to resume from a cursor past the end of its new,
        // shorter ring returned nothing, and every message sent before the
        // toggle vanished.
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        let batch = |n: u64| ShimMessage::Events {
            events: (0..n)
                .map(|seq| Sequenced {
                    seq,
                    event: AgentEvent::Message { role: Role::Agent, text: format!("m{seq}"), parent: None },
                })
                .collect(),
        };

        supervisor.apply(terminal, batch(40), &|_, _| {});
        assert_eq!(supervisor.replay(terminal, 0, u64::MAX).1.len(), 40);

        // A respawn: fewer events, numbered from zero again.
        supervisor.apply(
            terminal,
            ShimMessage::Established { session_id: "s".into(), available_modes: Vec::new() },
            &|_, _| {},
        );
        supervisor.apply(terminal, batch(5), &|_, _| {});
        assert_eq!(
            supervisor.replay(terminal, 0, u64::MAX).1.len(),
            5,
            "the new stream must replace the old, not be filtered against it"
        );
    }

    #[test]
    fn a_toggle_changes_the_epoch_and_hands_back_the_whole_transcript() {
        // The bug this design replaces: a shim numbers events by position in
        // its own ring, and a pane-mode toggle starts a new shim counting from
        // zero. A client holding a cursor into the old stream then asked for
        // events past the end of the new one and got nothing, so its
        // conversation appeared to be erased. Rather than reconciling two
        // numberings — which failed four separate ways — the stream says it is
        // a different stream, and the reader takes the whole thing.
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        let batch = |n: u64| ShimMessage::Events {
            events: (0..n)
                .map(|seq| Sequenced {
                    seq,
                    event: AgentEvent::Message { role: Role::Agent, text: format!("m{seq}"), parent: None },
                })
                .collect(),
        };

        supervisor.apply(terminal, batch(10), &|_, _| {});
        let (first_epoch, events) = supervisor.replay(terminal, 0, 0);
        assert_eq!(events.len(), 10);

        // Caught up: nothing new to send.
        let (_, nothing) = supervisor.replay(terminal, 10, first_epoch);
        assert!(nothing.is_empty());

        // The pane is toggled. A new shim announces itself and replays a
        // conversation numbered from zero again.
        supervisor.apply(
            terminal,
            ShimMessage::Established { session_id: "s".into(), available_modes: Vec::new() },
            &|_, _| {},
        );
        supervisor.apply(terminal, batch(4), &|_, _| {});

        // The client still asks with the cursor and epoch it held.
        let (second_epoch, after) = supervisor.replay(terminal, 10, first_epoch);
        assert_ne!(second_epoch, first_epoch, "a new shim is a new stream");
        assert_eq!(after.len(), 4, "a stale cursor must not hide the new transcript");
    }

    /// Both sources number one ring.
    ///
    /// The shim and the hook path are different transports for the same
    /// conversation. If each numbered its own, a client's cursor would mean
    /// two things and the transcript would interleave two sequences of zeros —
    /// the exact failure the epoch exists to make visible, arriving by a new
    /// route.
    #[test]
    fn events_recorded_from_a_hook_continue_the_same_numbering() {
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        // Collected rather than discarded, because a `record` that numbered
        // perfectly and never called `on_events` would leave every watching
        // client waiting on a poll it has no reason to make. Nothing else in
        // this file asserts the fan-out happens at all.
        let fanned: Mutex<Vec<(Uuid, Vec<Sequenced>)>> = Mutex::new(Vec::new());
        let sink = |t: Uuid, batch: Vec<Sequenced>| fanned.lock().unwrap().push((t, batch));

        supervisor.record(
            terminal,
            vec![AgentEvent::Message { role: Role::User, text: "one".to_string(), parent: None }],
            &sink,
        );
        supervisor.record(
            terminal,
            vec![AgentEvent::Message { role: Role::Agent, text: "two".to_string(), parent: None }],
            &sink,
        );

        let (_, events) = supervisor.replay(terminal, 0, 0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 0);
        assert_eq!(events[1].seq, 1, "the second call continues the first's numbering");

        let fanned = fanned.into_inner().unwrap();
        assert_eq!(
            fanned.iter().map(|(t, b)| (*t, b.iter().map(|s| s.seq).collect::<Vec<_>>())).collect::<Vec<_>>(),
            vec![(terminal, vec![0]), (terminal, vec![1])],
            "each batch is handed on as it is numbered, or a subscriber hears nothing"
        );
    }

    /// And it is the ring the SHIM numbers, not a second one beside it.
    ///
    /// The test above passes just as happily against a `record` that kept a
    /// window of its own, because nothing in it ever goes near `apply`. This
    /// one crosses the two transports, which is the property the split exists
    /// for.
    ///
    /// It settles the NUMBERING and not the duplication, and the two are easy
    /// to read as one question. They are not: one cursor pointing at one
    /// transcript says nothing about whether that transcript holds each
    /// message once. A pane fed by both transports has every assistant turn
    /// appended twice — by `apply` from its shim and by `record` from its
    /// hooks — interleaved into this one ring, and the user reads it twice.
    ///
    /// **That refusal now exists, and it is `hook_ingress::is_a_chat`.** It
    /// guards BOTH of `terminal_for`'s routes: a hook is never routed to a
    /// terminal whose `pane_mode` is `Agent`, whichever route found it.
    ///
    /// This comment used to end differently, and the way it was wrong is
    /// worth keeping. It said: "Nothing in this tree writes any of those three
    /// files yet, so none of it is reachable today ... whoever writes the
    /// installer inherits the question, and this comment is the only place it
    /// is written down." Both halves were true when written and the reasoning
    /// was sound. Six commits later, on this same branch, Task 10's
    /// `install_project_hooks` wrote `.codex/hooks.json` and
    /// `.cursor/hooks.json` into every worktree Far Cooler makes — and the
    /// premise the deferral rested on was gone with nothing anywhere noticing,
    /// because a comment cannot fail. The double-record was live for codex
    /// from that commit until the guard landed.
    ///
    /// A deferral is only as good as its premise, and a premise about what the
    /// tree CONTAINS has to be re-checked by whoever later makes the tree
    /// contain it. Nothing enforced that here, which is why the note reads as
    /// history rather than as a plan: the test that would have caught it —
    /// a pane in agent mode with a hook delivered for it — did not exist, and
    /// the commit that documented the hazard is the one that declared it
    /// untestable. `hook_ingress`'s
    /// `a_pane_in_agent_mode_is_never_handed_a_hook_as_well` is that test now.
    #[test]
    fn a_recorded_event_continues_the_transcript_the_shim_started() {
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();

        supervisor.apply(
            terminal,
            ShimMessage::Events {
                events: (0..3)
                    .map(|seq| Sequenced {
                        seq,
                        event: AgentEvent::Message { role: Role::Agent, text: format!("m{seq}"), parent: None },
                    })
                    .collect(),
            },
            &|_, _| {},
        );
        supervisor.record(
            terminal,
            vec![AgentEvent::Message { role: Role::User, text: "typed".to_string(), parent: None }],
            &|_, _| {},
        );

        let (_, events) = supervisor.replay(terminal, 0, 0);
        assert_eq!(events.len(), 4, "one transcript, not two");
        for (index, item) in events.iter().enumerate() {
            assert_eq!(item.seq, index as u64, "numbered by position, whichever transport brought it");
        }
        assert!(
            matches!(&events[3].event, AgentEvent::Message { text, .. } if text == "typed"),
            "the recorded event belongs at the end of the shim's transcript, got {:?}",
            events[3].event
        );
    }

    /// Activity is folded for recorded events too, onto what was already there.
    ///
    /// A card renders this word and nothing else on the hook path writes it.
    /// The second assertion is the one with teeth, and the rule it leans on is
    /// `activity::advance`: an `Idle` observation becomes `Done` only from
    /// `Working` or `Blocked` (and a row already `Done` stays `Done`); from
    /// anything else, `Unspecified` included, it is plain `Idle`. So a
    /// `record` that folded each batch from scratch would leave a finished
    /// turn sitting on `Idle` — a turn that never asks for anybody.
    #[test]
    fn recording_folds_activity_onto_what_the_terminal_already_had() {
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();

        supervisor.record(
            terminal,
            vec![AgentEvent::Message { role: Role::Agent, text: "working".to_string(), parent: None }],
            &|_, _| {},
        );
        assert_eq!(
            supervisor.activity(terminal),
            AgentActivity::Working,
            "activity is folded for hook events too, or no card ever updates"
        );

        supervisor.record(
            terminal,
            vec![AgentEvent::TurnEnded { reason: EndReason::EndTurn }],
            &|_, _| {},
        );
        assert_eq!(
            supervisor.activity(terminal),
            AgentActivity::Done,
            "finished and unseen, which is what puts the pane in front of somebody"
        );
    }

    // ---- writing down the conversation a shim reports ----

    /// A store with one terminal in it, on whatever preset is asked for.
    ///
    /// Real, not a double, because the whole point of these two tests is what
    /// SQLite ends up holding. A supervisor handed a struct it built itself
    /// would prove nothing: the column could stay NULL forever with both of
    /// them green.
    fn store_with_a_terminal(preset: &str) -> (Arc<Store>, Uuid) {
        let store = Arc::new(Store::open_in_memory().expect("store"));
        let host = Uuid::now_v7();
        let root = store.create_repository_root(host, "/repos/one", 1_000).expect("root");
        let repo =
            store.create_repository(host, root.id, "name", "/gitdir", "origin").expect("repo");
        let ws = store.create_workspace(repo.id, "feature/x", "/wt/one", false).expect("workspace");
        let term = store
            .create_terminal(ws.id, "t", preset, TerminalIntent::Running, 120, 40)
            .expect("terminal");
        (store, term.id)
    }

    /// The hand-typed pane, which is the only one this reaches.
    ///
    /// `shell`, deliberately, and that is the "whatever the preset" claim in
    /// executable form: `create_terminal` mints a session id only for a preset
    /// starting with `claude`, so this row starts with a NULL column, nothing
    /// but the shim can fill it, and any gate on the preset fails the
    /// assertion below rather than passing quietly. Read back out of SQLite
    /// rather than off anything the supervisor returned — the write is the
    /// claim.
    #[test]
    fn a_shim_that_establishes_writes_its_session_into_the_record() {
        let (store, terminal) = store_with_a_terminal("shell");
        assert_eq!(
            store.get_terminal(terminal).expect("row").agent_session_id,
            None,
            "the fixture must start with nothing to find"
        );

        let supervisor = AgentSupervisor::with_records(store.clone());
        supervisor.apply(
            terminal,
            ShimMessage::Established {
                session_id: "0199-hand-typed".into(),
                available_modes: Vec::new(),
            },
            &|_, _| {},
        );

        let row = store.get_terminal(terminal).expect("row");
        assert_eq!(
            row.agent_session_id.as_deref(),
            Some("0199-hand-typed"),
            "the id the shim reported has to be in the column a restart reads"
        );
        assert_eq!(
            row.pane_mode,
            farcooler_store::models::PaneMode::Terminal,
            "a shim reporting its session says which conversation and nothing else; the mode \
             a pane is in is the person's to change, and `set_pane_mode` is the only write \
             that can reach this column, so it has to be handed back what it was given"
        );
    }

    /// A shim reconnects on every pane-mode toggle and re-reports the same
    /// session. Writing that again would bump `resource_version` and push a
    /// fresh terminal to every watching client for a row that did not change.
    #[test]
    fn re_reporting_the_same_session_does_not_touch_the_record() {
        let (store, terminal) = store_with_a_terminal("shell");
        let supervisor = AgentSupervisor::with_records(store.clone());
        let established = || ShimMessage::Established {
            session_id: "0199-same".into(),
            available_modes: Vec::new(),
        };

        supervisor.apply(terminal, established(), &|_, _| {});
        let after_first = store.get_terminal(terminal).expect("row").resource_version;

        supervisor.apply(terminal, established(), &|_, _| {});
        assert_eq!(
            store.get_terminal(terminal).expect("row").resource_version,
            after_first,
            "a second report of one session is not a change to the row"
        );
    }
}

#[cfg(test)]
mod gap_tests {
    use super::*;

    /// A transcript that has lost its head says so.
    ///
    /// The window is renumbered by position, so trimming the front erases every
    /// trace that anything was there: a client would receive a shorter
    /// transcript with contiguous numbers and no reason to doubt it. A derived
    /// transcript is only defensible because it can say where it is incomplete.
    #[test]
    fn trimming_the_window_leaves_a_gap_rather_than_a_shorter_story() {
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();

        // Comfortably past the limit, so the front is dropped several times.
        let mut sent = 0;
        while sent < TRANSCRIPT_LIMIT + 500 {
            let batch: Vec<Sequenced> = (0..250)
                .map(|i| Sequenced {
                    seq: i,
                    event: AgentEvent::Message {
                        role: farcooler_agent::event::Role::Agent,
                        text: format!("line {}", sent + i as usize), parent: None },
                })
                .collect();
            supervisor.apply(terminal, ShimMessage::Events { events: batch }, &|_, _| {});
            sent += 250;
        }

        let (_, events) = supervisor.replay(terminal, 0, 0);
        assert_eq!(events.len(), TRANSCRIPT_LIMIT, "the window is bounded");
        assert!(
            matches!(
                events[0].event,
                AgentEvent::Gap { reason: AgentGapReason::RingTrimmed }
            ),
            "a trimmed transcript must open with the gap that says so, got {:?}",
            events[0].event
        );
        // Still a position in this transcript, which is what every cursor in
        // the system counts on.
        for (index, item) in events.iter().enumerate() {
            assert_eq!(item.seq, index as u64);
        }
    }

    /// A transcript filled by recorded events is bounded and says so too.
    ///
    /// The trim, the gap and the renumber are the rest of the tail `record`
    /// was split out of. A `record` that appended and numbered but stopped
    /// short of them would pass every other test in this file and grow for as
    /// long as the daemon stayed up — and the loss would arrive as a shorter
    /// story with contiguous numbers, which is the one thing this design says
    /// it will never do.
    #[test]
    fn a_transcript_filled_by_recorded_events_is_trimmed_and_says_so() {
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();

        let mut sent = 0;
        while sent < TRANSCRIPT_LIMIT + 500 {
            let batch: Vec<AgentEvent> = (0..250)
                .map(|i| AgentEvent::Message {
                    role: farcooler_agent::event::Role::Agent,
                    text: format!("line {}", sent + i),
                    parent: None,
                })
                .collect();
            supervisor.record(terminal, batch, &|_, _| {});
            sent += 250;
        }

        let (_, events) = supervisor.replay(terminal, 0, 0);
        assert_eq!(events.len(), TRANSCRIPT_LIMIT, "the window is bounded on this path as well");
        assert!(
            matches!(
                events[0].event,
                AgentEvent::Gap { reason: AgentGapReason::RingTrimmed }
            ),
            "a trimmed transcript must open with the gap that says so, got {:?}",
            events[0].event
        );
        for (index, item) in events.iter().enumerate() {
            assert_eq!(item.seq, index as u64);
        }
    }

    /// What the fan-out is handed across a trim is numbered like the
    /// transcript it came out of.
    ///
    /// This exists for one line — `renumbered = entry[entry.len() -
    /// renumbered.len()..].to_vec()`, the last statement of the trim — and
    /// that line was previously deletable in silence across the whole
    /// workspace. Every other sink in this file is `|_, _| {}`, and the one
    /// that collects is handed two batches of one event that never come near
    /// `TRANSCRIPT_LIMIT`. Without it a subscriber is handed the numbers the
    /// batch had BEFORE the front was dropped — positions past the end of the
    /// transcript it can ask for — and its next cursor sits there, which is
    /// the same class of failure as a cursor that survived a toggle.
    ///
    /// The length assertion is not decoration either: it is what stops
    /// `on_events(terminal, renumbered)` being narrowed to the last event
    /// alone, which the collecting test above cannot see because every batch
    /// it submits is one event long.
    #[test]
    fn a_batch_handed_out_across_a_trim_carries_the_numbers_it_will_be_asked_for() {
        const OVERFLOW: usize = 10;
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        let message = |n: usize| AgentEvent::Message {
            role: farcooler_agent::event::Role::Agent,
            text: format!("line {n}"),
            parent: None,
        };
        let fanned: Mutex<Vec<Vec<Sequenced>>> = Mutex::new(Vec::new());
        let sink = |_: Uuid, batch: Vec<Sequenced>| fanned.lock().unwrap().push(batch);

        // Exactly full, and deliberately not yet over: the limit is a maximum,
        // so this batch is handed out untouched and the NEXT one is the one
        // that drops a front.
        supervisor.record(terminal, (0..TRANSCRIPT_LIMIT).map(message).collect(), &sink);
        supervisor.record(terminal, (0..OVERFLOW).map(message).collect(), &sink);

        let fanned = fanned.into_inner().unwrap();
        let last = fanned.last().expect("both batches reached the fan-out");
        assert_eq!(last.len(), OVERFLOW, "every event submitted is handed on, not merely the last");
        assert_eq!(
            last.first().unwrap().seq,
            (TRANSCRIPT_LIMIT - OVERFLOW) as u64,
            "the batch starts where the trimmed transcript's tail starts"
        );
        assert_eq!(
            last.last().unwrap().seq,
            (TRANSCRIPT_LIMIT - 1) as u64,
            "and ends at its end: a subscriber's next cursor is built from this number \
             and must not point past a transcript it can ask for"
        );

        // The same events, by the same numbers, as a client that asked instead
        // of being told.
        let (_, replayed) = supervisor.replay(terminal, 0, 0);
        assert_eq!(
            last.as_slice(),
            &replayed[replayed.len() - OVERFLOW..],
            "the live batch and the replayed transcript disagree about the same events"
        );
    }
}
