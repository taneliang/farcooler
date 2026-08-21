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
use farcooler_agent::link::{DaemonMessage, ShimMessage, decode_line, encode_line};
use farcooler_agent::activity_source;
use farcooler_core::activity;
use farcooler_protocol::v1::AgentActivity;
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
    cursor: Seq,
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
}

impl AgentSupervisor {
    pub fn new() -> Self {
        Self::default()
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

    pub fn send(&self, terminal: Uuid, message: DaemonMessage) {
        if let Ok(writers) = self.writers.lock() {
            if let Some(tx) = writers.get(&terminal) {
                let _ = tx.send(message);
            }
        }
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
        if let Ok(mut recent) = self.recent.lock() {
            recent.remove(&terminal);
        }
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.entry(terminal).or_default().cursor = 0;
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
                if let Ok(mut sessions) = self.sessions.lock() {
                    let entry = sessions.entry(terminal).or_default();
                    entry.session_id = Some(session_id);
                    entry.available_modes = available_modes;
                    entry.cursor = 0;
                    // A new shim is a new stream. Readers holding a cursor into
                    // the old one are told by the change, rather than being
                    // left to work it out from numbers that silently mean
                    // something else now.
                    entry.epoch += 1;
                }
                return;
            }
            ShimMessage::Failed { reason } => {
                tracing::warn!(terminal = %terminal, %reason, "agent adapter failed to start");
                return;
            }
        };

        if let Ok(mut sessions) = self.sessions.lock() {
            let entry = sessions.entry(terminal).or_default();
            for s in &batch {
                entry.activity = fold_activity(entry.activity, &s.event);
                // `SessionStarted` and `ModeSet` are the only events that name
                // the ACP mode; anything else leaves it as it was.
                match &s.event {
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
                entry.cursor = s.seq + 1;
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
            renumbered = batch
                .into_iter()
                .enumerate()
                .map(|(i, e)| Sequenced { seq: base + i as u64, event: e.event })
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent::event::{AgentEvent, EndReason, PermissionOption, Role};
    use farcooler_protocol::v1::AgentActivity;

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
}
