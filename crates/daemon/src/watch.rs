//! Watching what the agents are doing, and telling everyone.
//!
//! Two jobs that belong together because they share one loop:
//!
//! 1. **Derive each agent's activity** by reading its screen. This has to be
//!    the daemon's job. A phone has no screen to inspect, and a Mac that
//!    inspected one would be a second authority disagreeing with the host about
//!    the same terminal.
//!
//! 2. **Push changes to every connected client.** Clients used to poll, which
//!    forces a choice between latency and cost and gets both wrong: too slow to
//!    notice an agent asking a question, too expensive for a phone on a battery
//!    over SSH.
//!
//! The daemon still samples on a timer — there is no way to be told that a
//! pane's pixels changed — but that sampling is local, cheap, and happens once
//! for all clients instead of once per client per interval.
//!
//! Only CHANGES are broadcast. A fleet where nothing is happening produces no
//! traffic at all, which is what makes a phone holding an SSH session overnight
//! reasonable.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use overnight_core::activity;
use overnight_protocol::v1::{AgentActivity, Event, TerminalState};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::service::Service;
use crate::wire;

/// How often to look.
///
/// A second is well under the time it takes a human to notice, and far above
/// what `capture-pane` costs. Faster would sample the same screen repeatedly
/// for nothing; slower would make a fleet feel stale in the one moment the
/// product exists for.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// How many events a slow client may fall behind before it starts losing them.
///
/// Losing them is the right failure. Dropping the connection would be worse
/// than the gap — the next event still arrives, and a client that needs
/// certainty re-reads.
const EVENT_BACKLOG: usize = 256;

/// The daemon's live view of what every agent is doing.
pub struct Watcher {
    service: Arc<Service>,
    events: broadcast::Sender<Event>,
    /// Last reported activity per terminal, which is what makes `Done`
    /// possible: it exists only as a transition out of `Working`.
    state: tokio::sync::Mutex<HashMap<Uuid, Observed>>,
}

#[derive(Debug, Clone)]
struct Observed {
    activity: AgentActivity,
    changed_at: i64,
    /// The terminal's process state when last announced.
    ///
    /// Watched as well as activity, and that omission was a real bug: a
    /// terminal you ended with Ctrl-D went from `running` to `exited`, which is
    /// not an activity change, so nothing was broadcast. Clients no longer poll,
    /// so the dead terminal simply stayed in the list forever.
    state: TerminalState,
    /// What is running in the pane, so a shell someone typed `claude` into
    /// announces itself the moment it becomes an agent.
    command: String,
    /// Whether this pane can be rendered as a chat.
    ///
    /// Decided here because deciding it needs the screen, and the screen is
    /// already being read on this pass. A client cannot answer it at all.
    chat_capable: bool,
}

impl Watcher {
    pub fn new(service: Arc<Service>) -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_BACKLOG);
        Arc::new(Self { service, events, state: tokio::sync::Mutex::new(HashMap::new()) })
    }

    /// Subscribe a connection to the push stream.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// What the watcher last decided about a terminal.
    pub async fn activity(&self, terminal: Uuid) -> (AgentActivity, Option<i64>) {
        match self.state.lock().await.get(&terminal) {
            Some(observed) => (observed.activity, Some(observed.changed_at)),
            None => (AgentActivity::Unspecified, None),
        }
    }

    /// What is running in a terminal, as the pane reports it.
    pub async fn command(&self, terminal: Uuid) -> Option<String> {
        self.state.lock().await.get(&terminal).map(|o| o.command.clone())
    }

    /// Whether this terminal could be shown as a chat, as last observed.
    pub async fn chat_capable(&self, terminal: Uuid) -> bool {
        self.state.lock().await.get(&terminal).is_some_and(|o| o.chat_capable)
    }

    /// Mark a terminal as looked at.
    ///
    /// `Done` is defined as idle-and-unseen, so this is what ends it. Called
    /// when a client opens a terminal — not when it merely appears in a list,
    /// which would clear a notification the user never actually read.
    pub async fn mark_seen(&self, terminal: Uuid) {
        let mut state = self.state.lock().await;
        let Some(observed) = state.get_mut(&terminal) else { return };
        let next = activity::seen(observed.activity);
        if next == observed.activity {
            return;
        }
        observed.activity = next;
        observed.changed_at = now_millis();
        let snapshot = observed.clone();
        drop(state);
        self.announce(terminal, snapshot).await;
    }

    /// Push a workspace's tiling to every connected client.
    ///
    /// Layout is not sampled — nothing changes it but an explicit request — so
    /// unlike activity this is announced by whoever made the change rather than
    /// discovered by the loop. It still goes through the watcher because the
    /// broadcast channel is here, and one sender means one place where "only
    /// changes are sent" stays true.
    pub fn publish_layout(&self, workspace: Uuid, groups: &[crate::layout::LayoutView]) {
        let _ = self.events.send(Event {
            event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
            sequence: 0,
            payload: Some(overnight_protocol::v1::event::Payload::LayoutChanged(
                wire::pane_group_list(workspace, groups),
            )),
        });
    }

    /// Run until cancelled.
    pub async fn run(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(SAMPLE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            self.sample().await;
        }
    }

    async fn sample(&self) {
        // Refresh the inventory first: a terminal that died since the last tick
        // must not have its stale screen re-read and reported as working.
        self.service.inventory.refresh().await;

        let Ok(fleet) = self.service.fleet().await else { return };
        let runtime = self.service.runtime();
        let panes = self.service.inventory_snapshot();

        // One `ps` for the whole host, not one per pane: this is the sampling
        // loop, and a fleet of thirty panes must not mean thirty processes a
        // second.
        let foreground = crate::foreground::read().await;

        let mut live = Vec::new();
        for workspace in &fleet {
            for terminal in &workspace.terminals {
                let id = terminal.terminal.id;
                // What is RUNNING, not what it was launched as — and with its
                // arguments where there are any. `pane_current_command` is a
                // process NAME, so `pnpm dev` arrives as `node`; the foreground
                // process group of the pane's tty has the argv that distinguishes
                // one pane from another.
                let pane = panes.panes.iter().find(|p| p.terminal_id == id);
                let command = pane
                    .and_then(|p| {
                        foreground.get(p.tty.trim_start_matches("/dev/")).cloned()
                    })
                    .or_else(|| pane.map(|p| p.command.clone()))
                    .unwrap_or_default();
                live.push((
                    id,
                    command,
                    terminal.state(),
                    terminal.terminal.pane_mode,
                    terminal.terminal.command_preset.clone(),
                ));
            }
        }

        // Terminals that are gone stop being tracked, or a restarted one would
        // inherit the activity of the process it replaced.
        {
            let mut state = self.state.lock().await;
            let ids: std::collections::HashSet<Uuid> =
                live.iter().map(|(id, ..)| *id).collect();
            state.retain(|id, _| ids.contains(id));
        }

        for (id, command, terminal_state, pane_mode, preset) in live {
            // The screen is read for any live terminal, not only one whose
            // process name we recognise. That is the point: Claude Code renames
            // itself to its version, so a pane reporting `2.1.220` is an agent
            // that process matching alone would never find.
            let (label, observed, chat_capable) = if !matches!(terminal_state, TerminalState::Running)
            {
                (activity::describe(&command, ""), AgentActivity::None, false)
            } else if pane_mode == overnight_store::models::PaneMode::Agent {
                // The protocol, not the screen. An agent-mode pane shows the
                // shim's status log, which matches no agent signature, so the
                // classifier would report `None` and this row would never say
                // it needs you — the one failure the whole feature exists to
                // prevent. The supervisor already folded these through
                // `activity::advance`, so `Done` means what it always means.
                // Named after the agent it is hosting. Calling every agent
                // pane "agent" is the one label they all share, so it
                // distinguishes nothing — and `set_pane_mode` recorded which
                // harness this is at the moment the pane could still say.
                (
                    if crate::service::chat_capable(&preset) {
                        preset.clone()
                    } else {
                        "agent".to_string()
                    },
                    self.service.agents().activity(id),
                    // Already a chat. The switch it offers is back to terminal.
                    true,
                )
            } else {
                // The screen is read for any live terminal, not only one whose
                // process name we recognise: Claude Code renames itself to its
                // version, so a pane reporting `2.1.220` is an agent that
                // process matching alone would never find.
                match runtime.screen(id).await {
                    Ok((screen, _, _)) => (
                        activity::describe(&command, &screen),
                        activity::classify(&command, &screen),
                        // Recognised AND hostable. Codex is recognised here and
                        // has no adapter, so offering it a chat would hand the
                        // user a Claude session in its place.
                        activity::identify(&command, &screen)
                            .is_some_and(|rules| crate::service::chat_capable(rules.preset)),
                    ),
                    Err(_) => (
                        activity::describe(&command, ""),
                        AgentActivity::Unspecified,
                        false,
                    ),
                }
            };
            // Resolved here, and sent resolved. A client has no screen to
            // inspect, so working out what an agent is called has to happen on
            // the host — the same reason its activity does.
            let command = label;

            let mut state = self.state.lock().await;
            let previous = state.get(&id).cloned();
            let next_activity = match &previous {
                Some(p) => activity::advance(p.activity, observed),
                None => observed,
            };

            // Announce on ANY of the three changing. State was the one that
            // used to be missed, and it is the one that removes a dead terminal
            // from a client's list.
            let changed = match &previous {
                None => true,
                Some(p) => {
                    p.activity != next_activity
                        || p.state != terminal_state
                        || p.command != command
                }
            };
            if !changed {
                continue;
            }

            let record = Observed {
                activity: next_activity,
                // The clock only moves when the ACTIVITY moved. A process that
                // exits should not reset "blocked for 12 minutes" to zero.
                changed_at: match &previous {
                    Some(p) if p.activity == next_activity => p.changed_at,
                    _ => now_millis(),
                },
                state: terminal_state,
                command: command.clone(),
                chat_capable,
            };
            state.insert(id, record.clone());
            drop(state);

            tracing::info!(
                terminal = %id,
                state = ?terminal_state,
                activity = ?next_activity,
                command = %command,
                "terminal changed"
            );
            self.announce(id, record).await;
        }
    }

    /// Broadcast one terminal's current state.
    ///
    /// The whole `Terminal` goes out, not a diff. A client that missed an event
    /// then converges on the next one instead of applying a delta to a state it
    /// may not have.
    async fn announce(&self, terminal: Uuid, observed: Observed) {
        let Ok(fleet) = self.service.fleet().await else { return };
        for workspace in &fleet {
            let Some(view) = workspace.terminals.iter().find(|t| t.terminal.id == terminal) else {
                continue;
            };
            let mut message = wire::terminal(view);
            message.activity = observed.activity as i32;
            message.activity_changed_at = Some(wire::timestamp(observed.changed_at));
            message.current_command = observed.command.clone();
            message.chat_capable = observed.chat_capable;

            // A send with no subscribers is not a failure: it is the ordinary
            // case of a host nobody is watching, which still has to keep
            // deriving so the first client to connect sees the truth.
            let _ = self.events.send(Event {
                event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
                sequence: 0,
                payload: Some(overnight_protocol::v1::event::Payload::TerminalChanged(message)),
            });
            return;
        }
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
