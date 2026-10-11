//! What every agent pane's hooks have said, as one transcript per terminal.
//!
//! **It owns the transcript.** `recent` is the conversation as the hooks
//! reported it, held in memory here and bounded by `TRANSCRIPT_LIMIT`. The
//! hook ingress records into it (`Service::start_hook_ingress`), and
//! `terminal.agent_subscribe` replays it: a phone's permission bar and the
//! watch read a pane's asks and its words from here, and `needs_you` reads
//! the ask a pane is waiting on.
//!
//! The consequence, said here because this is the first place anyone reads:
//! **a daemon restart empties every transcript on this runner.** Terminals do
//! not work that way: they are tmux panes, and `runtime.rs` rebuilds a pane's
//! replay from tmux's own scrollback at attach. The conversation view does not
//! read this either; it is folded from the agent's own transcript on disk
//! (`session_projectors`), which a restart does not touch.
//!
//! It held chat mode's conversation as well, fed by a shim over a socket per
//! terminal, until chat mode was retired (ov-377). `chat_migration` is what
//! is left of that: it moves a pane still in it to its TUI.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use farcooler_agent_core::event::{AgentEvent, AgentGapReason, EndReason, PermissionOption, Seq, Sequenced};
use farcooler_core::activity;
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::Store;
use uuid::Uuid;

use crate::activity_source;

/// How much of a conversation the daemon keeps per terminal.
///
/// Past it the front is dropped and a `Gap` takes its place, which is the only
/// honest way to serve a transcript that no longer starts at the beginning.
const TRANSCRIPT_LIMIT: usize = 4096;

/// Apply one event to a terminal's activity.
///
/// The observation is `activity_source`'s; the FOLD is `core::activity`'s,
/// and deliberately so. `Done` must mean the same thing whether it came from a
/// screen or from a protocol, or a Mac badge and a phone notification will
/// disagree about the same terminal.
pub fn fold_activity(current: AgentActivity, event: &AgentEvent) -> AgentActivity {
    // A failed turn is news even when nothing before it said the agent was
    // working. A key refused on the first request arrives as the end and
    // nothing else, and `advance(Idle, Idle)` is `Idle` — a row that never
    // turned up in front of anybody. It used to pass through `Working` only
    // because the adapter's error was drawn as the agent speaking first.
    if matches!(event, AgentEvent::TurnEnded { reason: EndReason::Failed { .. } }) {
        return activity::advance(activity::advance(current, AgentActivity::Working), AgentActivity::Idle);
    }
    match activity_source::observe(event) {
        Some(observed) => activity::advance(current, observed),
        None => current,
    }
}

#[derive(Debug, Default)]
struct SessionState {
    activity: AgentActivity,
    /// What the agent calls this conversation.
    ///
    /// Kept because it is the only description of a pane that describes the
    /// WORK. Every other name available — the preset, the process, the harness
    /// — says what is running, and a fleet of eight panes all called "claude"
    /// tells a user nothing about which is which.
    title: Option<String>,
    /// The last turn ended `EndReason::Failed`. Held until the next turn
    /// ends, and read only while the row is `Done` (`wire::failure_narrowed`).
    turn_failed: bool,
    /// How many turns have ended `Failed`, ever. A count rather than a flag
    /// so `watch` can tell a NEW failure from the one it already showed.
    failed_turns: u64,
}

/// What `AgentSupervisor::on_asks_moved` installs.
type AsksMoved = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Default)]
pub struct AgentSupervisor {
    sessions: Arc<Mutex<HashMap<Uuid, SessionState>>>,
    /// The transcripts, by terminal, oldest first. Bounded to
    /// `TRANSCRIPT_LIMIT` per terminal.
    recent: Arc<Mutex<HashMap<Uuid, Vec<Sequenced>>>>,
    /// When each `Permission` in `recent` arrived, by terminal and ask id, so
    /// an open ask can say how long it has waited. A `Sequenced` carries no
    /// time. Pruned when the ask's `Resolved` arrives and when the terminal is
    /// forgotten.
    asked_at: Arc<Mutex<HashMap<Uuid, HashMap<String, std::time::SystemTime>>>>,
    /// Told whenever a `Permission` or `Resolved` is recorded: the watcher's
    /// `announce_needs_you`, since both move the needs-you list. `None` until
    /// a watcher installs it.
    asks_moved: Arc<Mutex<Option<AsksMoved>>>,
    /// Where a turn's spend is filed (`crate::usage`), when there is a store
    /// to file it in. `None` for a supervisor nobody gave a store to, which is
    /// every test that is not about it.
    records: Option<Arc<Store>>,
}

impl AgentSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    /// The supervisor a daemon runs: one that can file what it learns.
    pub fn with_records(store: Arc<Store>) -> Self {
        Self { records: Some(store), ..Self::default() }
    }

    pub fn activity(&self, terminal: Uuid) -> AgentActivity {
        self.sessions
            .lock()
            .ok()
            .and_then(|s| s.get(&terminal).map(|st| st.activity))
            .unwrap_or(AgentActivity::Unspecified)
    }

    /// What the agent has named this conversation, if it has named it.
    pub fn title(&self, terminal: Uuid) -> Option<String> {
        self.sessions.lock().ok().and_then(|s| s.get(&terminal).and_then(|st| st.title.clone()))
    }

    /// Events at and after `from_seq`, and which run of the transcript that is.
    ///
    /// Empty for a terminal no hook has reported for: attaching to one is not
    /// an error, it just has nothing to show yet.
    ///
    /// A reader passes the epoch it last saw. The epoch is this daemon's life
    /// (`epoch::boot`), so a reader holding one from an earlier life gets the
    /// whole transcript back and replaces what it had. Within one epoch the
    /// cursor means what it says.
    pub fn replay(&self, terminal: Uuid, from_seq: Seq, client_epoch: u64) -> (u64, Vec<Sequenced>) {
        let all: Vec<Sequenced> = self
            .recent
            .lock()
            .ok()
            .and_then(|r| r.get(&terminal).cloned())
            .unwrap_or_default();

        // 0 for a pane with nothing to show: the phones read it as "no session
        // yet". Once its window holds events it answers this life's epoch.
        let epoch = if all.is_empty() { 0 } else { epoch::boot() };
        if client_epoch != epoch {
            return (epoch, replay_fit::newest_that_fit(all));
        }
        (epoch, replay_fit::newest_that_fit(all.into_iter().filter(|e| e.seq >= from_seq).collect()))
    }

    /// Call `f` whenever an ask appears or ends in any terminal's ring.
    /// One listener; a second replaces the first.
    pub fn on_asks_moved(&self, f: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.asks_moved.lock() {
            *slot = Some(Arc::new(f));
        }
    }

    /// The ask `terminal` is waiting on, if any: the last `Permission` in its
    /// transcript with no later `Resolved` of the same id and no turn ending
    /// or session starting after it, as (id, options, when it arrived).
    ///
    /// Scoped to the turn and not to the agent's activity: a sibling
    /// subagent's tool call, or the `Resolved` of a later ask, folds the pane
    /// to Working while this ask still waits. A closed pane's ring goes with
    /// its row.
    ///
    /// A claude TUI's held hook ask lands here through the hook ingress's
    /// sink, with its `Resolved` recorded by `HookAsks::settle`.
    pub fn open_permission(
        &self,
        terminal: Uuid,
    ) -> Option<(String, Vec<PermissionOption>, std::time::SystemTime)> {
        let (id, options) = {
            let recent = self.recent.lock().ok()?;
            let mut resolved = HashSet::new();
            let mut open = None;
            for s in recent.get(&terminal)?.iter().rev() {
                match &s.event {
                    AgentEvent::Resolved { id, .. } => {
                        resolved.insert(id.clone());
                    }
                    AgentEvent::Permission { id, options, .. } if !resolved.contains(id) => {
                        open = Some((id.clone(), options.clone()));
                        break;
                    }
                    // An ask lives no longer than its turn. A turn that ended,
                    // cancelled or not, answered nothing, and a new session is
                    // a new conversation; either way an ask from before it is
                    // not waiting on anybody.
                    AgentEvent::TurnEnded { .. } | AgentEvent::SessionStarted { .. } => break,
                    _ => {}
                }
            }
            open?
        };
        let at = self
            .asked_at
            .lock()
            .ok()
            .and_then(|a| a.get(&terminal).and_then(|asks| asks.get(&id)).copied())
            .unwrap_or_else(std::time::SystemTime::now);
        Some((id, options, at))
    }

    /// Fold, number and fan out a batch of events for one terminal.
    ///
    /// The hook ingress's sink: every event an agent's hooks reported for a
    /// terminal comes through here, and the position in this transcript is
    /// worked out below.
    pub fn record<F>(&self, terminal: Uuid, mut events: Vec<AgentEvent>, on_events: &F)
    where
        F: Fn(Uuid, Vec<Sequenced>),
    {
        // A turn's spend is filed, never numbered or sent (`crate::usage`).
        let heard = events.len();
        events.retain(|event| match event {
            AgentEvent::TurnUsage { usage } => {
                if let Some(store) = self.records.as_ref() {
                    crate::usage::record_chat(store, terminal, usage);
                }
                false
            }
            _ => true,
        });
        if events.is_empty() && heard > 0 {
            return;
        }
        if let Ok(mut sessions) = self.sessions.lock() {
            let entry = sessions.entry(terminal).or_default();
            for event in &events {
                entry.activity = fold_activity(entry.activity, event);
                match event {
                    AgentEvent::SessionInfo { title } if !title.is_empty() => {
                        entry.title = Some(title.clone());
                    }
                    AgentEvent::TurnEnded { reason } => {
                        entry.turn_failed = matches!(reason, EndReason::Failed { .. });
                        entry.failed_turns += u64::from(entry.turn_failed);
                        // The backend's own words go to the log, nowhere else.
                        if let EndReason::Failed { kind, detail } = reason {
                            tracing::warn!(
                                terminal = %terminal,
                                kind = ?kind,
                                detail = %detail,
                                "an agent turn failed"
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        let moved = events
            .iter()
            .any(|e| matches!(e, AgentEvent::Permission { .. } | AgentEvent::Resolved { .. }));
        if moved && let Some(tell) = self.asks_moved.lock().ok().and_then(|slot| slot.clone()) {
            tell();
        }
        if let Ok(mut asked_at) = self.asked_at.lock() {
            for event in &events {
                match event {
                    AgentEvent::Permission { id, .. } => {
                        asked_at.entry(terminal).or_default().insert(id.clone(), std::time::SystemTime::now());
                    }
                    AgentEvent::Resolved { id, .. } => {
                        if let Some(asks) = asked_at.get_mut(&terminal) {
                            asks.remove(id);
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut renumbered: Vec<Sequenced> = Vec::new();
        if let Ok(mut recent) = self.recent.lock() {
            let entry = recent.entry(terminal).or_default();
            // Numbered one past the newest event held, never by length.
            //
            // The daemon is the only thing that numbers these, so a number
            // means one position in one transcript and nothing else. Numbering
            // by length renumbered every trim, leaving a reader's forward-only
            // cursor past the end of a window counting from below it (ov-380).
            // An emptied window restarts at zero: only `forget` empties one,
            // and it ends the stream a cursor counted in.
            let base = entry.last().map_or(0, |last| last.seq + 1);
            renumbered = events
                .into_iter()
                .enumerate()
                .map(|(i, event)| Sequenced { seq: base + i as u64, event })
                .collect();
            entry.extend(renumbered.iter().cloned());

            // Oldest first, so trimming the front keeps the most recent
            // `TRANSCRIPT_LIMIT`.
            //
            // The trim leaves a `Gap` behind, and that is not decoration.
            // Dropping the front erases every trace that anything was there,
            // so a client asking from zero would receive a shorter transcript
            // and no reason to doubt it. A derived transcript is only
            // defensible because it can say where it is incomplete; silently
            // losing history is the one thing this design forbids.
            //
            // The gap takes the number of the newest event it replaces, so
            // numbers stay strictly increasing and nothing is renumbered. A
            // reader past the gap never sees it, and one before it reads it
            // first, which is what it should do.
            if entry.len() > TRANSCRIPT_LIMIT {
                let dropped = entry.len() - TRANSCRIPT_LIMIT + 1;
                let seq = entry[dropped - 1].seq;
                entry.drain(0..dropped);
                entry.insert(
                    0,
                    Sequenced { seq, event: AgentEvent::Gap { reason: AgentGapReason::RingTrimmed } },
                );
                // What was just handed out can lose its own head to the trim
                // when a batch is larger than the window.
                let oldest = entry[1].seq;
                renumbered.retain(|item| item.seq >= oldest);
            }
        }

        on_events(terminal, replay_fit::newest_that_fit(renumbered));
    }

    /// Drop everything held for a terminal whose record is gone.
    ///
    /// The counterpart to `record`, which creates both of these through
    /// `or_default()` for whatever terminal it is handed: every terminal a
    /// live session routes a hook to, which is most of them. A row and up to
    /// `TRANSCRIPT_LIMIT` events each, for the life of the daemon, and every
    /// one of them answers correctly the whole time — it is simply never asked
    /// again. `HookIngress::forget` is called from the same line of the same
    /// delete path, against the same class of leak.
    pub fn forget(&self, terminal: Uuid) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&terminal);
        }
        if let Ok(mut asked_at) = self.asked_at.lock() {
            asked_at.remove(&terminal);
        }
        if let Ok(mut recent) = self.recent.lock() {
            recent.remove(&terminal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent_core::event::{AgentEvent, EndReason, PermissionOption, Role};
    use farcooler_protocol::v1::AgentActivity;

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

    fn ask(id: &str) -> AgentEvent {
        AgentEvent::Permission {
            id: id.into(),
            tool_call: String::new(),
            options: vec![
                PermissionOption { id: "allow".into(), name: "Allow touch x".into(), kind: "allow_once".into() },
                PermissionOption { id: "deny".into(), name: "Deny".into(), kind: "reject_once".into() },
            ],
        }
    }

    // An answered chat ask leaving the open asks is tested through the real
    // answer path, `terminal.agent_answer` over a socket with a shim on the
    // other end: `answering_a_chat_ask_takes_it_off_the_list`.

    #[test]
    fn open_permission_returns_the_later_of_two_unresolved_asks() {
        let supervisor = AgentSupervisor::new();
        let pane = Uuid::now_v7();
        supervisor.record(pane, vec![ask("p1")], &|_, _| {});
        supervisor.record(pane, vec![ask("p2")], &|_, _| {});
        assert_eq!(supervisor.open_permission(pane).map(|o| o.0).as_deref(), Some("p2"));
        // And with the later one answered, the earlier one is what's left.
        supervisor.record(pane, vec![AgentEvent::Resolved { id: "p2".into(), chosen: "deny".into() }], &|_, _| {});
        assert_eq!(supervisor.open_permission(pane).map(|o| o.0).as_deref(), Some("p1"));
    }

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
}

mod epoch;
mod failure;
#[cfg(test)]
mod monotonic_seq_tests;
mod replay_fit;
#[cfg(test)]
mod replay_fit_tests;
#[cfg(test)]
mod failed_turn_tests;
#[cfg(test)]
mod gap_tests;
