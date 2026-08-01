//! The daemon's half of every agent session.
//!
//! It owns no transcript. The shim holds the ring, because the shim lives
//! exactly as long as the pane whose liveness is already authoritative — so a
//! daemon restart costs no history and needs no `session/load`.
//!
//! What lives here is the bookkeeping only the daemon can do: which terminals
//! are in agent pane mode, what each one's activity is, and fanning events out
//! to however many clients are watching. `recent` is a small fast-attach
//! window only — the shim's ring, not this one, is what a client falls back on
//! when it asks for history older than this holds.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use overnight_agent::event::{AgentEvent, Seq, Sequenced};
use overnight_agent::link::{DaemonMessage, ShimMessage, decode_line, encode_line};
use overnight_agent::activity_source;
use overnight_core::activity;
use overnight_protocol::v1::AgentActivity;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use uuid::Uuid;

/// Events kept per terminal for a client that attaches fresh.
///
/// Deliberately small next to the shim's `AGENT_RING_EVENTS` (4096): this
/// window only has to cover the common case of a client opening a pane that
/// is already mid-conversation, not survive a daemon restart. Anything older
/// than this is the shim's ring to replay, not the daemon's.
const RECENT_WINDOW: usize = 512;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ToggleRefusal {
    #[error("a turn is in flight; cancel it or force the switch")]
    TurnInFlight,
}

/// Where a terminal's shim dials.
///
/// Per terminal and per runtime directory, so two daemons on one host never
/// collide and a stale socket never adopts a new session.
pub fn socket_path(runtime_dir: &Path, terminal: Uuid) -> PathBuf {
    runtime_dir.join(format!("agent-{terminal}.sock"))
}

/// Apply one event to a terminal's activity.
///
/// The observation is `overnight_agent`'s; the FOLD is `core::activity`'s, and
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
    available_modes: Vec<String>,
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
    pub fn replay(&self, terminal: Uuid, from_seq: Seq) -> Vec<Sequenced> {
        self.recent
            .lock()
            .ok()
            .and_then(|r| r.get(&terminal).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.seq >= from_seq)
            .collect()
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
        let listener = UnixListener::bind(&path)?;

        loop {
            let (stream, _) = listener.accept().await?;
            let cursor = self
                .sessions
                .lock()
                .ok()
                .and_then(|s| s.get(&terminal).map(|st| st.cursor))
                .unwrap_or(0);
            if let Err(e) = self.serve(stream, terminal, cursor, &on_events).await {
                tracing::warn!(terminal = %terminal, error = %e, "agent shim link ended");
            }
        }
    }

    async fn serve<F>(
        &self,
        stream: UnixStream,
        terminal: Uuid,
        cursor: Seq,
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

        let subscribe = encode_line(&DaemonMessage::Subscribe { from_seq: cursor })
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
            // The gap is already the first entry; the counters are for logs.
            ShimMessage::Trimmed { resumed_at, dropped, events } => {
                tracing::info!(terminal = %terminal, resumed_at, dropped, "agent ring trimmed");
                events
            }
            ShimMessage::Established { session_id, available_modes } => {
                if let Ok(mut sessions) = self.sessions.lock() {
                    let entry = sessions.entry(terminal).or_default();
                    entry.session_id = Some(session_id);
                    entry.available_modes = available_modes;
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
                    _ => {}
                }
                entry.cursor = s.seq + 1;
            }
        }

        if let Ok(mut recent) = self.recent.lock() {
            let entry = recent.entry(terminal).or_default();
            // Only what we have not already seen.
            //
            // A shim replays from a cursor whenever it reconnects, and a
            // daemon restart or a second connection means the same seqs arrive
            // twice. Appending blindly put the whole conversation in the
            // transcript two and three times over — visibly, since seq is
            // monotonic and authoritative, so a repeat is always a replay
            // rather than new speech.
            let already = entry.last().map(|e| e.seq);
            entry.extend(
                batch
                    .iter()
                    .filter(|e| already.is_none_or(|last| e.seq > last))
                    .cloned(),
            );
            // Oldest first, so trimming the front keeps the most recent
            // `RECENT_WINDOW` — the same "drop the oldest" the shim's ring
            // does, just at a smaller size and without a Gap: the shim's ring
            // is still the authority a client falls back to.
            if entry.len() > RECENT_WINDOW {
                let excess = entry.len() - RECENT_WINDOW;
                entry.drain(0..excess);
            }
        }

        on_events(terminal, batch);
    }

    /// A client looked at this terminal, which is what ends `Done`.
    pub fn seen(&self, terminal: Uuid) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(entry) = sessions.get_mut(&terminal) {
                entry.activity = activity::seen(entry.activity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use overnight_agent::event::{AgentEvent, EndReason, PermissionOption, Role};
    use overnight_protocol::v1::AgentActivity;

    #[test]
    fn a_socket_path_is_per_terminal_and_not_guessable_across_daemons() {
        let a = socket_path(Path::new("/run/overnight"), Uuid::now_v7());
        let b = socket_path(Path::new("/run/overnight"), Uuid::now_v7());
        assert_ne!(a, b);
        assert!(a.starts_with("/run/overnight"));
    }

    #[test]
    fn activity_folds_through_core_so_done_still_means_unseen() {
        // The rule that makes a notification worth sending lives in core and is
        // not reimplemented here. Working -> Idle is what produces Done.
        let mut current = AgentActivity::Unspecified;
        current = fold_activity(current, &AgentEvent::Message { role: Role::Agent, text: "x".into() });
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
    fn a_replayed_batch_does_not_land_in_the_window_twice() {
        // A shim replays from a cursor on every reconnect, so the same seqs
        // arrive again after a daemon restart or a second connection.
        // Appending blindly put the whole conversation into the transcript two
        // and three times over. seq is monotonic and authoritative, so a
        // repeat is always a replay rather than new speech.
        let supervisor = AgentSupervisor::new();
        let terminal = Uuid::now_v7();
        let batch: Vec<Sequenced> = (0..3)
            .map(|seq| Sequenced {
                seq,
                event: AgentEvent::Message { role: Role::Agent, text: format!("m{seq}") },
            })
            .collect();

        supervisor.apply(terminal, ShimMessage::Events { events: batch.clone() }, &|_, _| {});
        supervisor.apply(terminal, ShimMessage::Events { events: batch.clone() }, &|_, _| {});
        assert_eq!(supervisor.replay(terminal, 0).len(), 3);

        // And genuinely new events still land.
        supervisor.apply(
            terminal,
            ShimMessage::Events {
                events: vec![Sequenced {
                    seq: 3,
                    event: AgentEvent::Message { role: Role::Agent, text: "new".into() },
                }],
            },
            &|_, _| {},
        );
        assert_eq!(supervisor.replay(terminal, 0).len(), 4);
    }
}
