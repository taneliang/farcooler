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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use farcooler_core::activity;
use farcooler_protocol::v1::{AgentActivity, Event, TerminalState};
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

/// How often the backstop compares what it believes against what tmux says.
///
/// Rare on purpose. It is a defect DETECTOR, not a repair: the inventory is
/// meant to be kept current by control-mode notifications, and if this ever
/// fires the bug is in that path. Running it every tick would double the tmux
/// traffic to re-prove something that is almost always true; running it never —
/// which is what happened, since nothing called it — means the one class of bug
/// it exists to catch goes unreported forever.
const BACKSTOP_INTERVAL: Duration = Duration::from_secs(60);

/// How often every repository is reconciled regardless of what the gate says.
///
/// The gate watches one directory, and something could change a worktree
/// without touching it — a filesystem with no mtime granularity, a `git
/// worktree repair`, a restore from backup. Thirty seconds is well under how
/// long anyone would stare at a stale sidebar and far above what the scan costs.
const RECONCILE_BACKSTOP: Duration = Duration::from_secs(30);

/// How many events a slow client may fall behind before it starts losing them.
///
/// Losing them is the right failure. Dropping the connection would be worse
/// than the gap — the next event still arrives, and a client that needs
/// certainty re-reads.
const EVENT_BACKLOG: usize = 256;

/// Everything the relay is told about one agent changing state.
///
/// A struct rather than a tuple because `status` is not interchangeable with
/// the two sentences beside it: the relay switches on it to decide whether a
/// live card goes up or comes down, and three `String`s in a row is a
/// signature where transposing two of them still compiles and only shows up as
/// a lock screen that never clears.
struct Notice {
    title: String,
    subtitle: String,
    /// `"blocked"` or `"done"`, and nothing else — an empty or invented status
    /// tells the relay it is talking to a daemon it should stop trusting. It is
    /// `&'static str` so there is nowhere for a computed one to come from.
    status: &'static str,
}

/// What, if anything, is worth waking a phone for.
///
/// Split out from the sending so it can be tested at all: the rule it encodes —
/// two states out of five — is the difference between a product people keep
/// notifications on for and one they mute, and the sending half is a spawn and
/// a filesystem read that no test can reach through.
///
/// The status the phone acts on is decided HERE, next to the sentence the
/// person reads, and not anywhere else. Both come off the same `AgentActivity`,
/// and two matches on it in two files is the pair that drifts: the live card
/// would say one thing and the notification under it another, for the same
/// agent, in the same second.
///
/// `question` is what the agent is actually asking, when the screen was legible
/// enough to say. It is the whole point of a lock screen card: "claude needs
/// you / Do you want to create haiku.txt?" is something a person can answer
/// from the phone in their hand, and "claude needs you / Waiting for your
/// answer" is something they have to walk to a Mac to even read. The generic
/// line stays for when there is no question — a trust gate, or a prompt that
/// wrapped — because a card that says nothing is still better than no card.
fn notification(activity: AgentActivity, label: &str, question: Option<&str>) -> Option<Notice> {
    match activity {
        AgentActivity::Blocked => Some(Notice {
            title: format!("{label} needs you"),
            subtitle: match question {
                Some(q) if !q.trim().is_empty() => q.to_string(),
                _ => "Waiting for your answer".to_string(),
            },
            status: "blocked",
        }),
        AgentActivity::Done => Some(Notice {
            title: format!("{label} finished"),
            subtitle: String::new(),
            status: "done",
        }),
        _ => None,
    }
}

/// What to tell the relay when a command ends badly.
///
/// Reuses `"done"` rather than inventing a third status: the relay's whole
/// vocabulary is `"blocked"` (raise a live card) or `"done"` (dismiss one and
/// deliver a banner), and a failed run is not something to hold a live card
/// open for — it is a turn that is OVER, just over badly. The wording is what
/// tells a failure apart from a success; the status only tells the relay
/// whether to keep a card on the lock screen.
fn exit_notice(label: &str, exit_code: Option<i32>, exit_signal: Option<i32>) -> Notice {
    let subtitle = match (exit_code, exit_signal) {
        (_, Some(signal)) => format!("Stopped by signal {signal}"),
        (Some(code), _) => format!("Exit code {code}"),
        // Reached only if this is ever called outside `exited_into_failure`'s
        // guard, which already requires one of the two above.
        (None, None) => String::new(),
    };
    Notice { title: format!("{label} failed"), subtitle, status: "done" }
}

/// The supervisor's activity, reduced to a raw OBSERVATION.
///
/// The sampling loop folds whatever it observes through `activity::advance`,
/// and that fold is what creates and destroys `Done`. A screen classifier hands
/// it a raw sense of the pane — working, blocked, or sitting there — and never
/// `Done`, so the fold owns the state outright.
///
/// The agent supervisor is different: it has already folded its own events, so
/// its activity can BE `Done`. Feeding that back in folds it twice, and the
/// second fold resurrects what the first one's `seen` had just cleared —
/// `advance(Idle, Done)` is `Done`. That is a row you open, watch go quiet, and
/// watch light up again a second later, notification and all, forever.
///
/// So the supervisor's `Done` is dropped here. `Done` means idle-and-unseen,
/// and whether anyone has seen it is the watcher's business, not the
/// supervisor's — one owner, exactly as for a pane read off the screen.
fn agent_observation(folded: AgentActivity) -> AgentActivity {
    activity::seen(folded)
}

/// A tagged pane with no durable terminal record is not a third workspace pane.
/// It is residue from a close that removed the record before tmux collapsed the
/// split. Leaving it in the layout gives clients a real rectangle with nothing
/// they can render, which presents as a large blank column.
fn orphaned_pane(
    pane: &farcooler_core::inventory::TaggedPane,
    daemon: Uuid,
    terminals: &HashSet<Uuid>,
) -> bool {
    pane.daemon_id == daemon && !terminals.contains(&pane.terminal_id)
}

/// The daemon's live view of what every agent is doing.
pub struct Watcher {
    service: Arc<Service>,
    /// One client for the life of the daemon, so a night of notifications
    /// reuses a connection rather than opening one per agent.
    push: reqwest::Client,
    events: broadcast::Sender<Event>,
    /// Last reported activity per terminal, which is what makes `Done`
    /// possible: it exists only as a transition out of `Working`.
    state: tokio::sync::Mutex<HashMap<Uuid, Observed>>,
    /// Last observed mtime of each repository's `worktrees` directory.
    ///
    /// A std mutex: held only across a map lookup, never across an await.
    worktree_marks: std::sync::Mutex<HashMap<Uuid, std::time::SystemTime>>,
}

/// One terminal as this tick found it, before anything is made of it.
///
/// The host-wide reads — `ps`, `lsof`, the fleet, the pane inventory — happen
/// once and are joined here, so the classification pass below can run per
/// terminal without going back to the machine for anything but that terminal's
/// screen. It was a tuple until it needed a title and a purpose; seven
/// positional fields is not something a reader can hold.
struct Sampled {
    id: Uuid,
    /// What is running in the pane, arguments and all.
    command: String,
    /// The pane's OSC title, as written by whatever runs there. Untrusted, and
    /// sanitized where it is read — see `farcooler_core::title::parse`.
    title: String,
    /// What the pane serves, if it holds a listening socket.
    purpose: Option<String>,
    state: TerminalState,
    pane_mode: farcooler_store::models::PaneMode,
    preset: String,
    /// How the command ended, when it has. Threaded through from the terminal
    /// record rather than re-read at the push site: the record is behind the
    /// same store read every other field here already came from, and a second
    /// read taken later in the tick could see a different terminal than the
    /// one this pass is otherwise describing.
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
}

#[derive(Debug, Clone)]
struct Observed {
    activity: AgentActivity,
    /// When the CURRENT state began. Reset on every state change.
    ///
    /// This is `changed_at` under its real name. It answers "how long has this
    /// been stuck", which is the question that matters while Blocked.
    state_since: i64,
    /// When the user's request started, or `None` between turns.
    ///
    /// Held across Blocked: approving a tool call does not begin a new turn,
    /// and a clock that restarted on every approval reported a quarter-hour job
    /// as two seconds old.
    turn_started_at: Option<i64>,
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
    /// What the agent is asking, while it is asking.
    blocked_question: Option<String>,
    /// A candidate state and how many times running it has been seen.
    ///
    /// Hysteresis. A `capture-pane` taken while the footer is being rewritten
    /// can miss it, and one such sample used to be enough to fire a Done and
    /// the notification behind it.
    pending: Option<(AgentActivity, u8)>,
    /// The pane title as last sampled, and how many samples running it has been
    /// byte-identical. See `promoted_by_title`.
    title: String,
    title_repeats: u8,
}

/// Agreeing samples needed before a state change is published.
///
/// Two, which costs one sampling interval of latency on every transition except
/// the one that must never be delayed. See `observe`.
const CONFIRMATIONS: u8 = 2;

/// Samples a title may repeat before its claim to be working stops being believed.
///
/// A real spinner ANIMATES — codex cycles fourteen braille frames and claude four
/// quadrant circles, both several times a second — so two samples a second apart
/// differ. A frozen frame is byte-identical every tick, and that is what a title
/// looks like once the agent has stopped writing one: it crashed, or the user
/// turned `terminal_title` off mid-session, which `core::title` notes is a config
/// key and therefore something a user can do at any moment.
///
/// Five, which is five seconds at `SAMPLE_INTERVAL`. The number is chosen by
/// which way the risk runs, not by taste: failing to promote falls back to the
/// screen, which is usually right, and costs at most a few seconds of a row
/// reading Idle while it works. Believing a frozen frame forever means the folded
/// activity never returns to Idle, `activity::advance` never produces `Done`, and
/// no notification is ever sent for that pane again — the exact bug this change
/// exists to remove, re-entering through the title instead of the screen.
const STALE_TITLE_SAMPLES: u8 = 5;

/// What the title says about activity, where the screen had nothing to say.
///
/// One-directional, and that direction is the whole point. A footer drawing
/// `esc to interrupt` is the agent stating its own status, so a screen that
/// reported Working or Blocked is never overruled by a title frame that
/// disagrees — that frame is stale far more often than the footer is wrong.
/// Only `Idle`, which `classify` returns for a recognized agent whose footer
/// matched nothing, is open to being resolved by the title.
///
/// Pulled out of `sample()`'s loop body for the same reason `should_announce`
/// was: a test that has to re-implement the loop's condition to check it will
/// eventually be checking something the loop no longer does.
fn promoted_by_title(
    activity: AgentActivity,
    title: &str,
    command: &str,
    hostname: &str,
    title_repeats: u8,
) -> AgentActivity {
    if activity != AgentActivity::Idle || title_repeats >= STALE_TITLE_SAMPLES {
        return activity;
    }
    match farcooler_core::title::parse(title, command, hostname).status {
        farcooler_core::title::TitleStatus::Working => AgentActivity::Working,
        // NotWorking included: a resting glyph agrees with Idle, and a title
        // that claims to be blocked is not evidence of a question anyone can
        // answer — `blocked_question` needs a screen for that.
        _ => activity,
    }
}

impl Observed {
    /// A terminal seen for the first time.
    fn begin(activity: AgentActivity, now: i64) -> Self {
        Observed {
            activity,
            state_since: now,
            turn_started_at: (activity == AgentActivity::Working).then_some(now),
            state: TerminalState::Running,
            command: String::new(),
            chat_capable: false,
            blocked_question: None,
            pending: None,
            title: String::new(),
            title_repeats: 0,
        }
    }

    /// Fold this tick's title in, returning how long it has been unchanged.
    ///
    /// Zero means it just moved. Called on every sample, whether or not anything
    /// is announced, because the answer is about the title's liveness and not
    /// about the row.
    fn saw_title(&mut self, title: &str) -> u8 {
        if self.title == title {
            // Saturating, because the only question asked of this is whether it
            // has passed a small bound, and a wrapping counter would answer it
            // wrong once every 256 seconds.
            self.title_repeats = self.title_repeats.saturating_add(1);
        } else {
            self.title.clear();
            self.title.push_str(title);
            self.title_repeats = 0;
        }
        self.title_repeats
    }

    /// Move to `next`, keeping whichever clocks should survive it.
    fn advance_to(mut self, next: AgentActivity, now: i64) -> Self {
        // Not `use AgentActivity::*` here: the enum has its own `None` variant
        // (a plain shell), which would shadow `Option::None` in every arm below
        // and turn `self.turn_started_at`'s type into a compile error rather
        // than the value this comment used to describe.
        self.turn_started_at = match next {
            // A turn in progress. Blocked is part of it, not the end of it.
            AgentActivity::Working | AgentActivity::Blocked => self.turn_started_at.or(Some(now)),
            // Anything else has ended the turn, so there is no clock to run.
            //
            // This includes `Unspecified` and `Unknown` — a failed
            // `capture-pane` or a screen that matched nothing recognized — which
            // is deliberate rather than an oversight: those mean "could not
            // tell", and the row is already reporting degraded when they land.
            // Holding the turn clock open across a state we cannot read would
            // require inventing a promise ("this is still the same turn") the
            // daemon has no evidence for, which is worse than the clock
            // restarting on the next real observation.
            _ => None,
        };
        self.activity = next;
        self.state_since = now;
        self.pending = None;
        self
    }

    /// Fold one sample in, returning the new activity if it should be published.
    ///
    /// `None` means nothing changed, or something changed and has not been seen
    /// enough times to be believed yet.
    fn observe(&mut self, sample: AgentActivity, now: i64) -> Option<AgentActivity> {
        let next = activity::advance(self.activity, sample);
        if next == self.activity {
            self.pending = None;
            return None;
        }

        // Blocked publishes on its first sighting, always.
        //
        // The asymmetry is deliberate and is the whole shape of the trade: a
        // question shown a second late costs nothing, and a question never
        // shown is the failure this feature exists to prevent. Every other
        // transition can afford to be sure.
        if next != AgentActivity::Blocked {
            let seen = match self.pending {
                Some((p, n)) if p == next => n + 1,
                _ => 1,
            };
            if seen < CONFIRMATIONS {
                self.pending = Some((next, seen));
                return None;
            }
        }

        *self = self.clone().advance_to(next, now);
        Some(next)
    }

    /// Whether this observation is worth telling clients about.
    ///
    /// Four things can make a terminal newsworthy, and the fourth is easy to
    /// miss: an agent can replace its question without any of the other three
    /// moving — activity stays Blocked, the pane state stays Running, the
    /// command is unchanged — and a row that keeps answering the PREVIOUS
    /// question is worse than one that says only "Needs you". Pulled out of
    /// `sample()`'s loop body so a unit test can call the exact function the
    /// loop calls, rather than a copy of it that can drift out of sync with
    /// what the loop actually does.
    fn should_announce(
        &self,
        activity_moved: bool,
        state: TerminalState,
        command: &str,
        blocked_question: &Option<String>,
    ) -> bool {
        activity_moved
            || self.state != state
            || self.command != command
            || self.blocked_question != *blocked_question
    }
}

/// Whether THIS tick is the moment a command just failed, and worth a push.
///
/// A command failing is a state transition (Running -> Exited with a bad
/// code, or a signal), not an activity one, so it never reaches
/// `push_if_paired` through `activity_moved`. Split out for the same reason
/// `should_announce` and `promoted_by_title` are: the loop's condition has to
/// stay the exact thing a test exercises, not a copy of it that can drift.
///
/// `just_appeared` excludes the first sighting of an already-dead terminal —
/// restarting the daemon must not re-announce every failed build in the
/// fleet's history, the same reason a freshly begun `Observed` never fires an
/// activity push either. `previous_state != Exited` is what makes this fire
/// once: the tick that actually crosses into Exited, not every tick after.
fn exited_into_failure(
    just_appeared: bool,
    previous_state: TerminalState,
    state: TerminalState,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
) -> bool {
    !just_appeared
        && previous_state != TerminalState::Exited
        && state == TerminalState::Exited
        && activity::exit_wants_attention(exit_code, exit_signal)
}

impl Watcher {
    pub fn new(service: Arc<Service>) -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_BACKLOG);
        Arc::new(Self {
            service,
            events,
            state: tokio::sync::Mutex::new(HashMap::new()),
            worktree_marks: std::sync::Mutex::new(HashMap::new()),
            push: reqwest::Client::builder()
                // A notification is worth a few seconds and no more. The
                // sampling loop is not held either way — see
                // `push_if_paired`, which spawns rather than awaits — but a
                // request with no ceiling would leak a task per stuck relay.
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        })
    }

    /// Push an agent's state change to the owner's devices, if paired.
    ///
    /// Blocked and done only. A working agent is the normal case, and something
    /// that buzzes for the normal case is something people turn off — after
    /// which it cannot tell them the thing that mattered. See `push_notice` for
    /// why sending is detached rather than awaited here.
    fn push_if_paired(
        &self,
        terminal: Uuid,
        activity: AgentActivity,
        label: &str,
        question: Option<&str>,
    ) {
        let Some(notice) = notification(activity, label, question) else { return };
        self.push_notice(terminal, label, notice);
    }

    /// Push a failed exit to the owner's devices, if this machine is paired.
    ///
    /// The other reason `push_if_paired` exists, for a command rather than an
    /// agent: see `exited_into_failure` for exactly when this fires. Kept as
    /// its own entry point rather than folded into `push_if_paired` because
    /// the two are answering different questions about the terminal — one an
    /// `AgentActivity`, the other an exit code — and a single function taking
    /// both would have to explain which one wins when a caller supplied both.
    fn push_failed_exit(
        &self,
        terminal: Uuid,
        label: &str,
        exit_code: Option<i32>,
        exit_signal: Option<i32>,
    ) {
        self.push_notice(terminal, label, exit_notice(label, exit_code, exit_signal));
    }

    /// Send one notice to the relay, detached from the sampling loop.
    ///
    /// Deliberately NOT async, and this is the whole point of the function.
    /// The sampling loop calls it after the watcher's state mutex is already
    /// dropped, but an HTTP round trip is still not something the loop can
    /// afford to wait on: it would delay the NEXT terminal in the same pass,
    /// and a relay that is slow or unreachable would stack that delay across
    /// every agent finishing in the same tick. Spawning detaches it: the loop
    /// moves on immediately and the notification arrives, or does not, on its
    /// own time.
    ///
    /// Reading the pairing file happens in the spawned task for the same
    /// reason. It is blocking I/O, and it does not belong under a lock either.
    fn push_notice(&self, terminal: Uuid, label: &str, notice: Notice) {
        // Owned, because the spawned task outlives this call by design and the
        // label is borrowed from the sampling loop's stack. The sentences are
        // already owned — the label is the one the phone needs on its own, for
        // the half of the live card that is not a sentence.
        let label = label.to_string();
        // Cheap: a `reqwest::Client` is a handle to a shared pool, so this
        // clone keeps the connection reuse the one-client-per-daemon buys.
        let client = self.push.clone();
        tokio::spawn(async move {
            let Some(pairing) = crate::push::Pairing::load() else { return };
            crate::push::notify(
                &client,
                &pairing,
                &notice.title,
                &notice.subtitle,
                notice.status,
                &label,
                &terminal.to_string(),
            )
            .await;
        });
    }

    /// Subscribe a connection to the push stream.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// What the watcher last decided, with both clocks.
    pub async fn activity(&self, terminal: Uuid) -> (AgentActivity, Option<i64>, Option<i64>) {
        match self.state.lock().await.get(&terminal) {
            Some(o) => (o.activity, Some(o.state_since), o.turn_started_at),
            None => (AgentActivity::Unspecified, None, None),
        }
    }

    /// What the agent is asking, if it is.
    pub async fn blocked_question(&self, terminal: Uuid) -> Option<String> {
        self.state.lock().await.get(&terminal).and_then(|o| o.blocked_question.clone())
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
        observed.state_since = now_millis();
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
            payload: Some(farcooler_protocol::v1::event::Payload::LayoutChanged(
                wire::pane_group_list(workspace, groups),
            )),
        });
    }

    /// Tell every connected client that the set of workspaces changed.
    ///
    /// Carries nothing: reconciliation can both create and delete workspace
    /// rows in one pass, and a deletion has no resource left to describe.
    /// Sent once per reconcile pass that changed anything, not once per
    /// workspace — a client re-reads the fleet rather than applying this as
    /// a delta.
    ///
    /// Also called directly by the RPC layer after `repository.register`,
    /// `workspace.hide`, and `workspace.unhide` — mutations that change the
    /// fleet without touching git, so the mtime gate in `reconcile_worktrees`
    /// never fires for them and other connected clients would otherwise learn
    /// about them only at the next `RECONCILE_BACKSTOP` tick.
    pub fn announce_fleet_changed(&self) {
        let _ = self.events.send(Event {
            event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
            sequence: 0,
            payload: Some(farcooler_protocol::v1::event::Payload::FleetChanged(
                farcooler_protocol::v1::Empty {},
            )),
        });
    }

    /// A workspace's change set moved.
    ///
    /// Carries the workspace and a version, never the set: most clients are not
    /// looking at a diff, and a lockfile regeneration would otherwise fan
    /// thousands of file records out to every connected device.
    pub fn announce_change_set(&self, workspace_id: Uuid, version: u64) {
        let _ = self.events.send(Event {
            event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
            sequence: 0,
            payload: Some(farcooler_protocol::v1::event::Payload::ChangeSetChanged(
                farcooler_protocol::v1::ChangeSetChanged {
                    workspace_id: bytes::Bytes::copy_from_slice(workspace_id.as_bytes()),
                    version,
                },
            )),
        });
    }

    /// Reconcile repositories whose worktrees moved, or all of them if forced.
    ///
    /// Broadcasts only when something actually changed, for the same reason
    /// `sample` does: a fleet where nothing is happening produces no traffic,
    /// which is what makes a phone holding an SSH session overnight reasonable.
    async fn reconcile_worktrees(&self, force: bool) {
        let Ok(repositories) = self.service.list_repositories() else { return };

        let mut changed = false;
        for repo in repositories {
            let common = std::path::PathBuf::from(&repo.canonical_git_dir);
            let previous = self
                .worktree_marks
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&repo.id)
                .copied();

            let moved = worktrees_changed(&common, previous);
            if moved.is_none() && !force {
                continue;
            }
            if let Some(mark) = moved {
                self.worktree_marks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(repo.id, mark);
            }

            match crate::reconcile::repository(&self.service, repo.id).await {
                Ok(outcome) => changed |= !outcome.is_quiet(),
                Err(e) => tracing::warn!(repository = %repo.id, error = ?e, "reconcile failed"),
            }
        }

        if changed {
            self.announce_fleet_changed();
        }
    }

    /// Run until cancelled.
    pub async fn run(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(SAMPLE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut backstop = tokio::time::interval(BACKSTOP_INTERVAL);
        backstop.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut worktrees = tokio::time::interval(RECONCILE_BACKSTOP);
        worktrees.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick of an interval completes immediately, and comparing
        // the inventory against itself before anything has had a chance to
        // diverge would only ever report a false alarm.
        backstop.tick().await;
        worktrees.tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    self.sample().await;
                    // Gated: one stat per repository, and a git process only
                    // for repositories whose worktrees actually moved.
                    self.reconcile_worktrees(false).await;
                }
                _ = backstop.tick() => self.service.backstop_reconcile().await,
                _ = worktrees.tick() => self.reconcile_worktrees(true).await,
            }
        }
    }

    async fn sample(&self) {
        // Refresh the inventory first: a terminal that died since the last tick
        // must not have its stale screen re-read and reported as working.
        self.service.inventory.refresh().await;

        let Ok(fleet) = self.service.fleet().await else { return };
        let runtime = self.service.runtime();
        let mut panes = self.service.inventory_snapshot();

        // A record is written before its pane is created, so a managed pane
        // whose terminal is absent from a successful fleet read can never be a
        // starting terminal. It is an orphan. Reap it here, where the durable
        // fleet and live inventory are already read together, then publish the
        // repaired geometry so every open client fills the recovered space.
        if panes.inventory_healthy {
            let terminals: HashSet<Uuid> = fleet
                .iter()
                .flat_map(|workspace| workspace.terminals.iter().map(|terminal| terminal.terminal.id))
                .collect();
            let daemon = self.service.tmux.daemon_id();
            let orphaned: Vec<_> = panes
                .panes
                .iter()
                .filter(|pane| orphaned_pane(pane, daemon, &terminals))
                .map(|pane| (pane.pane_id.clone(), pane.workspace_id, pane.terminal_id))
                .collect();

            let mut repaired = HashSet::new();
            for (pane, workspace, terminal) in orphaned {
                match self.service.tmux.kill_pane(&pane).await {
                    Ok(true) => {
                        repaired.insert(workspace);
                        tracing::warn!(%pane, %terminal, "reaped orphaned managed pane");
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(%pane, %terminal, ?error, "orphaned pane could not be reaped");
                    }
                }
            }

            if !repaired.is_empty() {
                panes = self.service.inventory.refresh().await;
                for workspace in repaired {
                    if let Ok(groups) = self.service.layout(workspace).await {
                        self.publish_layout(workspace, &groups);
                    }
                }
            }
        }

        // One `ps` for the whole host, not one per pane: this is the sampling
        // loop, and a fleet of thirty panes must not mean thirty processes a
        // second.
        let foreground = crate::foreground::read().await;
        // One `lsof` for the whole machine, on the same cadence and for the
        // same reason as the one `ps`.
        //
        // Off the executor, because it is a blocking `Command::output` on a
        // process that can take tens of milliseconds — `foreground::read` gets
        // the same treatment from `tokio::process` and cannot be copied here,
        // since `farcooler-core` has no async runtime and must not gain one for
        // this.
        let ports = tokio::task::spawn_blocking(farcooler_core::ports::listening_ports)
            .await
            .unwrap_or_default();
        // By GROUP, not by process. `lsof` names the process holding the socket,
        // which for every wrapped dev server — `pnpm dev`, `npm run dev`, a
        // shell script — is a child of the one the pane is showing.
        let ports = foreground.ports_by_group(&ports);
        // Once per tick, not once per pane: every pane compares against the
        // same answer, and a machine that renamed itself mid-tick would
        // otherwise name two rows by two different rules.
        let hostname = crate::hostname();
        // Bound once per tick rather than looked up per pane: the registry
        // lives on `Service` for the same reason `root` does — re-reading the
        // config file's location on every pane would let it disagree with
        // itself mid-tick if the environment changed underneath.
        let registry = self.service.registry();

        let mut live = Vec::new();
        // Changes panes whose pane has gone. See the reap below the loop.
        let mut spent = Vec::new();
        for workspace in &fleet {
            for terminal in &workspace.terminals {
                let id = terminal.terminal.id;
                // A diff that has been closed leaves nothing behind.
                //
                // Every other terminal keeps its record after the process ends,
                // because the exit code answers a question somebody asked. A
                // changes pane has no such answer: the record exists only to
                // give a rectangle a mode, and once tmux has taken the
                // rectangle away — `⌃B x`, a killed window, a quit tmux server
                // — all it can contribute is a dead row called Changes in the
                // sidebar of every worktree anyone ever opened one in.
                //
                // Only while the inventory is HEALTHY, which is the whole
                // safety of this. `derive_terminal` reports every terminal on
                // the machine as `Lost` when tmux cannot be read at all — so
                // without this gate, one failed `list-panes` would delete every
                // changes pane in the fleet and kill the panes on the way past,
                // for a machine that was fine a second later.
                if panes.inventory_healthy
                    && terminal.terminal.pane_mode == farcooler_store::models::PaneMode::Changes
                    && matches!(
                        terminal.state(),
                        // Exited is a pane that ran `exit` and was retained, or
                        // one this app stopped; Lost is `⌃B x`, which leaves no
                        // pane behind at all. Both mean the rectangle is gone.
                        TerminalState::Exited | TerminalState::Lost
                    )
                {
                    spent.push(id);
                    continue;
                }
                // What is RUNNING, not what it was launched as — and with its
                // arguments where there are any. `pane_current_command` is a
                // process NAME, so `pnpm dev` arrives as `node`; the foreground
                // process group of the pane's tty has the argv that distinguishes
                // one pane from another.
                let pane = panes.panes.iter().find(|p| p.terminal_id == id);
                let running = pane.and_then(|p| foreground.pane(p.tty.trim_start_matches("/dev/")));
                let command = running
                    .map(|r| r.command.clone())
                    .or_else(|| pane.map(|p| p.command.clone()))
                    .unwrap_or_default();
                let title = pane.map(|p| p.title.clone()).unwrap_or_default();
                // A pane's ports are its foreground process GROUP's, found
                // through the tty they share. `ps` already gave us that group on
                // this tick, so this is a map lookup rather than a second walk
                // of the process table.
                let purpose = running
                    .and_then(|r| ports.get(&r.pgid))
                    .and_then(|open| farcooler_core::ports::purpose(open));
                live.push(Sampled {
                    id,
                    command,
                    title,
                    purpose,
                    state: terminal.state(),
                    pane_mode: terminal.terminal.pane_mode,
                    preset: terminal.terminal.command_preset.clone(),
                    exit_code: terminal.terminal.exit_code,
                    exit_signal: terminal.terminal.exit_signal,
                });
            }
        }

        // Announced once for however many were reaped, because a client re-reads
        // the fleet rather than applying this as a delta — the same contract
        // `fleet_changed` already has for a reconcile pass that deleted a
        // workspace.
        if !spent.is_empty() {
            for id in spent {
                if let Err(e) = self.service.remove_terminal(id).await {
                    tracing::debug!(terminal = %id, error = ?e, "closed changes pane not reaped");
                }
            }
            self.announce_fleet_changed();
        }

        // Terminals that are gone stop being tracked, or a restarted one would
        // inherit the activity of the process it replaced.
        {
            let mut state = self.state.lock().await;
            let ids: std::collections::HashSet<Uuid> = live.iter().map(|s| s.id).collect();
            state.retain(|id, _| ids.contains(id));
        }

        for Sampled {
            id,
            command,
            title,
            purpose,
            state: terminal_state,
            pane_mode,
            preset,
            exit_code,
            exit_signal,
        } in live
        {
            // The screen is read for any live terminal, not only one whose
            // process name we recognize. That is the point: Claude Code renames
            // itself to its version, so a pane reporting `2.1.220` is an agent
            // that process matching alone would never find.
            // Only set in the screen-reading arm below: it is the only one
            // that has a screen to read a question off of.
            let mut question: Option<String> = None;
            // Folded in before anything is decided, and on every tick rather
            // than only on the ones that announce: this counts how long the
            // title has been FROZEN, which is a fact about the pane and not
            // about the row. A terminal seen for the first time has no history
            // and starts at zero, which is the trusting end of the scale.
            let title_repeats = {
                let mut state = self.state.lock().await;
                state.get_mut(&id).map(|entry| entry.saw_title(&title)).unwrap_or(0)
            };
            let (label, observed, chat_capable) = if !matches!(terminal_state, TerminalState::Running)
            {
                // A finished agent KEEPS its summary, which is the point of
                // reading titles at all: coming back to a machine, the useful
                // thing about a pane that has stopped is what it did, not that
                // it was claude.
                //
                // Safe because a dead pane cannot claim to be busy. `remain-on-exit`
                // leaves the last title in place, the spinner glyph is stripped
                // before the name is taken, this arm hardcodes `None` against a
                // non-Running state so the row's status never says otherwise,
                // and an agent that cleared its title on the way out falls
                // through to `describe` exactly as before.
                (
                    registry.describe_pane(&command, "", &title, purpose.as_deref(), &hostname),
                    AgentActivity::None,
                    false,
                )
            } else if pane_mode == farcooler_store::models::PaneMode::Changes {
                // No screen read, because there is nothing on it to read. A
                // changes pane runs a process that prints one line and waits;
                // classifying it would spend a `capture-pane` per sample to
                // learn, every time, that a diff is not an agent.
                ("changes".to_string(), AgentActivity::None, false)
            } else if pane_mode == farcooler_store::models::PaneMode::Agent {
                // The protocol, not the screen. An agent-mode pane shows the
                // shim's status log, which matches no agent signature, so the
                // classifier would report `None` and this row would never say
                // it needs you — the one failure the whole feature exists to
                // prevent. Taken as a raw observation, `Done` and all: see
                // `agent_observation` for why folding it twice was a row that
                // could never be dismissed.
                // Named after the agent it is hosting. Calling every agent
                // pane "agent" is the one label they all share, so it
                // distinguishes nothing — and `set_pane_mode` recorded which
                // harness this is at the moment the pane could still say.
                (
                    if registry.chat_capable(&preset) {
                        preset.clone()
                    } else {
                        "agent".to_string()
                    },
                    agent_observation(self.service.agents().activity(id)),
                    // The SAME question the label just asked, not a hardcoded
                    // yes. `set_pane_mode` writes the mode and the harness in
                    // two statements, so a sample landing between them — or any
                    // row predating the capability check — is in agent mode
                    // with a preset that cannot actually be hosted, and saying
                    // otherwise offers a switch that would refuse.
                    registry.chat_capable(&preset),
                )
            } else {
                // The screen is read for any live terminal, not only one whose
                // process name we recognize: Claude Code renames itself to its
                // version, so a pane reporting `2.1.220` is an agent that
                // process matching alone would never find.
                match runtime.screen(id).await {
                    Ok((screen, _, _)) => {
                        // The title is the agent's own account of itself, so it
                        // resolves a screen that had no opinion — while the
                        // title is still alive. See `promoted_by_title`.
                        let activity = promoted_by_title(
                            registry.classify(&command, &screen),
                            &title,
                            &command,
                            &hostname,
                            title_repeats,
                        );
                        // Only while blocked, and derived here rather than on a
                        // client for the same reason activity is: a phone has
                        // no screen to read.
                        question = registry.blocked_question(&command, &screen);
                        (
                            registry.describe_pane(&command, &screen, &title, purpose.as_deref(), &hostname),
                            activity,
                            // Recognized AND hostable. Codex is recognized here
                            // and has no adapter, so offering it a chat would
                            // hand the user a Claude session in its place.
                            registry
                                .identify(&command, &screen)
                                .is_some_and(|rules| registry.chat_capable(&rules.preset)),
                        )
                    }
                    Err(_) => (
                        registry.describe_pane(&command, "", &title, purpose.as_deref(), &hostname),
                        AgentActivity::Unspecified,
                        false,
                    ),
                }
            };
            // Resolved here, and sent resolved. A client has no screen to
            // inspect, so working out what an agent is called has to happen on
            // the host — the same reason its activity does.
            let command = label;
            let blocked_question = question;

            let mut state = self.state.lock().await;
            let now = now_millis();
            // Read before `entry()` inserts one: a terminal seen for the first
            // time has no real history, and `Observed::begin` gives it `state:
            // Running` regardless of what was actually sampled — so without
            // this, a daemon restarted onto an already-dead terminal would read
            // as a Running-to-Exited transition on the very first tick.
            let just_appeared = !state.contains_key(&id);
            let entry = state.entry(id).or_insert_with(|| Observed::begin(observed, now));
            let previous_state = entry.state;

            let activity_moved = entry.observe(observed, now);
            let changed = entry.should_announce(
                activity_moved.is_some(),
                terminal_state,
                &command,
                &blocked_question,
            );
            if !changed {
                continue;
            }

            entry.state = terminal_state;
            entry.command = command.clone();
            entry.chat_capable = chat_capable;
            entry.blocked_question = blocked_question;
            let record = entry.clone();
            drop(state);

            // Worth telling the owner about, and only on the transition.
            //
            // No second check needed here: `observe` returns `Some` only when
            // it just published a real activity change, so `Some` already
            // means the activity moved — a terminal whose command or question
            // changed reports `None` and never reaches this arm, which is what
            // keeps a `cd` from buzzing a phone.
            if let Some(next) = activity_moved {
                // From `record`, not from the local `blocked_question`, which
                // was moved into the entry above. Same value, and taking it off
                // the record is what keeps the card and the row quoting one
                // question rather than two reads of it.
                self.push_if_paired(id, next, &command, record.blocked_question.as_deref());
            }
            // A command failing is a STATE transition, not an activity one —
            // `activity_moved` never fires for it, so without this a `cargo
            // build` that exited 101 overnight reached the sidebar and never
            // the phone. See `exited_into_failure` for the exact rule.
            if exited_into_failure(just_appeared, previous_state, terminal_state, exit_code, exit_signal)
            {
                self.push_failed_exit(id, &command, exit_code, exit_signal);
            }

            tracing::info!(
                terminal = %id,
                state = ?terminal_state,
                activity = ?record.activity,
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
            // `terminal_with_agent_state`, not `terminal`.
            //
            // The plain converter knows nothing about the ACP session, so an
            // event-driven update carried the STORED title — and the stored
            // title is "Terminal 19". A pane named after its conversation on
            // load therefore reverted to the harness name the first time
            // anything happened in it, which is to say immediately and forever
            // after. The full-list path had this right; this one did not, and
            // the two disagreeing about the same terminal is the disagreement
            // every part of this design exists to prevent.
            // Persisted as it is learned, not only reported.
            //
            // The title comes from the supervisor's memory; writing it through
            // to the record here is what makes it survive a daemon restart. See
            // `Service::remember_agent_title`.
            if let Some(title) = self.service.agents().title(terminal) {
                self.service.remember_agent_title(terminal, &title);
            }

            let mut message = wire::terminal_with_agent_state(view, self.service.agents());
            message.activity = observed.activity as i32;
            message.activity_changed_at = Some(wire::timestamp(observed.state_since));
            message.turn_started_at = observed.turn_started_at.map(wire::timestamp);
            message.blocked_question = observed.blocked_question.clone();
            message.current_command = observed.command.clone();
            message.chat_capable = observed.chat_capable;

            // A send with no subscribers is not a failure: it is the ordinary
            // case of a host nobody is watching, which still has to keep
            // deriving so the first client to connect sees the truth.
            let _ = self.events.send(Event {
                event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
                sequence: 0,
                payload: Some(farcooler_protocol::v1::event::Payload::TerminalChanged(message)),
            });
            return;
        }
    }
}

/// Whether a repository's worktrees have changed since we last looked.
///
/// `git worktree add` creates a directory under `$GIT_COMMON_DIR/worktrees` and
/// `git worktree remove` deletes one; either moves that directory's mtime. One
/// `stat` per repository per tick is nothing, where one `git worktree list` per
/// repository per tick is a process spawn per repository per second, almost
/// always to learn that nothing happened.
///
/// Returns the new mtime when it moved, `None` when it did not. A repository
/// with no linked worktrees has no such directory and reports no change, which
/// is correct: the backstop pass in `run` still covers anything this misses.
pub fn worktrees_changed(
    git_common_dir: &std::path::Path,
    since: Option<std::time::SystemTime>,
) -> Option<std::time::SystemTime> {
    let modified = std::fs::metadata(git_common_dir.join("worktrees")).ok()?.modified().ok()?;
    match since {
        Some(previous) if previous >= modified => None,
        _ => Some(modified),
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The clock a person actually reads.
    ///
    /// Approving a permission prompt walks Working -> Blocked -> Working. The turn
    /// clock must not notice: the task has been running the whole time, and
    /// restarting it at every approval is what made "working 12m" read as
    /// "working 2s" for a job that had been going for a quarter of an hour.
    #[test]
    fn the_turn_clock_survives_a_permission_prompt() {
        let mut o = Observed::begin(AgentActivity::Working, 1_000);
        assert_eq!(o.turn_started_at, Some(1_000));

        o = o.advance_to(AgentActivity::Blocked, 5_000);
        assert_eq!(o.turn_started_at, Some(1_000), "the turn did not restart");
        assert_eq!(o.state_since, 5_000, "but the state clock did");

        o = o.advance_to(AgentActivity::Working, 9_000);
        assert_eq!(o.turn_started_at, Some(1_000), "still the same turn");
        assert_eq!(o.state_since, 9_000);
    }

    #[test]
    fn the_turn_clock_clears_when_the_work_ends() {
        let o = Observed::begin(AgentActivity::Working, 1_000).advance_to(AgentActivity::Done, 7_000);
        assert_eq!(o.turn_started_at, None, "a finished turn has no running clock");
        assert_eq!(o.state_since, 7_000);
    }

    #[test]
    fn a_new_turn_starts_a_new_clock() {
        let o = Observed::begin(AgentActivity::Working, 1_000)
            .advance_to(AgentActivity::Done, 7_000)
            .advance_to(AgentActivity::Working, 9_000);
        assert_eq!(o.turn_started_at, Some(9_000));
    }

    /// One bad sample between two good ones publishes nothing.
    ///
    /// A capture taken mid-redraw can miss a footer that is being rewritten. Before
    /// hysteresis that was enough to fire a spurious Done, with the notification
    /// that goes with it.
    #[test]
    fn a_single_odd_sample_does_not_move_the_state() {
        let mut o = Observed::begin(AgentActivity::Working, 1_000);
        let first = o.observe(AgentActivity::Idle, 2_000);
        assert!(first.is_none(), "one sighting is not a state change");
        assert_eq!(o.activity, AgentActivity::Working);

        let second = o.observe(AgentActivity::Working, 3_000);
        assert!(second.is_none(), "and the anomaly is forgotten");
        assert_eq!(o.activity, AgentActivity::Working);
    }

    #[test]
    fn two_agreeing_samples_do_move_it() {
        let mut o = Observed::begin(AgentActivity::Working, 1_000);
        assert!(o.observe(AgentActivity::Idle, 2_000).is_none());
        let moved = o.observe(AgentActivity::Idle, 3_000).expect("two agreeing samples publish");
        assert_eq!(moved, AgentActivity::Done, "Working -> Idle is what Done is made of");
    }

    /// Blocked is exempt, and must be.
    ///
    /// A question that arrives a second late is fine. A question that never
    /// arrives is the failure that makes the whole feature pointless, so a first
    /// sighting of Blocked publishes immediately.
    #[test]
    fn a_question_is_never_made_to_wait() {
        let mut o = Observed::begin(AgentActivity::Working, 1_000);
        let moved = o.observe(AgentActivity::Blocked, 2_000).expect("blocked publishes at once");
        assert_eq!(moved, AgentActivity::Blocked);
    }

    /// Calls the exact function `sample()` calls, term by term.
    ///
    /// Not a copy of the expression: `should_announce` is the same function
    /// the loop uses to decide, so deleting a term from it breaks this test
    /// rather than leaving it vacuously green. That distinction matters
    /// because it is exactly what went wrong the first time — a hand-copied
    /// closure asserted against the fourth term without ever calling the real
    /// gate, and a review had to point out that the closure would keep
    /// passing even with the term deleted from production.
    ///
    /// The fourth term is the one worth a name: an agent can replace its
    /// question — including replacing it with silence — without activity,
    /// pane state, or command moving at all. Two permission prompts back to
    /// back, faster than the 1 Hz sample, are exactly this case. A row that
    /// keeps answering the PREVIOUS question is worse than one that only says
    /// "Needs you" — this string is headed for a lock screen.
    #[test]
    fn each_of_the_four_terms_alone_opens_the_announce_gate() {
        let mut entry = Observed::begin(AgentActivity::Blocked, 1_000);
        entry.state = TerminalState::Running;
        entry.command = "claude".to_string();
        entry.blocked_question = Some("proceed?".to_string());

        let state = entry.state;
        let command = entry.command.clone();
        let question = entry.blocked_question.clone();

        assert!(
            !entry.should_announce(false, state, &command, &question),
            "nothing changed, so there is nothing to announce"
        );
        assert!(entry.should_announce(true, state, &command, &question), "activity moved");
        assert!(
            entry.should_announce(false, TerminalState::Exited, &command, &question),
            "pane state changed"
        );
        assert!(entry.should_announce(false, state, "bash", &question), "command changed");
        assert!(
            entry.should_announce(false, state, &command, &Some("a different question?".to_string())),
            "the question changed to another question — the regression this test exists to catch"
        );
        assert!(
            entry.should_announce(false, state, &command, &None),
            "the question vanished — that is news too, not just a new one arriving"
        );
    }

    /// A spinner that is still spinning is believed.
    ///
    /// Not a copy of the loop's condition: `promoted_by_title` is the function
    /// `sample()` calls, for the same reason `should_announce` is.
    #[test]
    fn a_live_spinner_resolves_a_screen_that_had_no_opinion() {
        // Codex advances its braille frame several times a second, so a sample
        // a second later reads a different title and the repeat count stays 0.
        let mut entry = Observed::begin(AgentActivity::Idle, 1_000);
        for frame in ["⠋ bare", "⠙ bare", "⠹ bare", "⠸ bare", "⠼ bare", "⠴ bare", "⠦ bare"] {
            let repeats = entry.saw_title(frame);
            assert_eq!(repeats, 0, "an animating title is never stale: {frame}");
            assert_eq!(
                promoted_by_title(AgentActivity::Idle, frame, "codex", "Mac", repeats),
                AgentActivity::Working,
                "{frame}"
            );
        }
    }

    /// A spinner that has stopped is not.
    ///
    /// The failure this guards is silent and permanent: a title frozen on a
    /// spinner frame — the agent crashed, or the user turned `terminal_title`
    /// off mid-session — would hold the folded activity at Working forever,
    /// `advance` would never produce Done, and that pane would never notify
    /// anyone again.
    #[test]
    fn a_frozen_spinner_stops_being_believed() {
        let mut entry = Observed::begin(AgentActivity::Idle, 1_000);
        let frozen = "◐ Write tmux haiku";
        let mut promoted = Vec::new();
        for _ in 0..10 {
            let repeats = entry.saw_title(frozen);
            promoted.push(promoted_by_title(AgentActivity::Idle, frozen, "claude", "Mac", repeats));
        }

        assert_eq!(
            promoted.iter().filter(|a| **a == AgentActivity::Working).count(),
            STALE_TITLE_SAMPLES as usize,
            "believed for the bound and not one sample longer: {promoted:?}"
        );
        assert_eq!(
            promoted.last(),
            Some(&AgentActivity::Idle),
            "and the screen has the last word once the title has stopped moving"
        );

        // A title that moves again is alive again — a resumed agent is not
        // punished for the pause.
        let repeats = entry.saw_title("◑ Write tmux haiku");
        assert_eq!(repeats, 0);
        assert_eq!(
            promoted_by_title(AgentActivity::Idle, "◑ Write tmux haiku", "claude", "Mac", repeats),
            AgentActivity::Working
        );
    }

    /// The promotion runs one way only.
    #[test]
    fn a_title_never_overrules_a_screen_that_had_an_opinion() {
        // A resting glyph against a screen that read the footer: the footer
        // wins. Getting this backwards is the flapping the whole change removes.
        for screen_said in [
            AgentActivity::Working,
            AgentActivity::Blocked,
            AgentActivity::Done,
            AgentActivity::None,
            AgentActivity::Unspecified,
        ] {
            for title in ["✳ Claude Code", "◐ Write tmux haiku", "[ ! ] Action Required | bare", ""] {
                assert_eq!(
                    promoted_by_title(screen_said, title, "claude", "Mac", 0),
                    screen_said,
                    "{screen_said:?} / {title}"
                );
            }
        }
    }

    /// Idle plus a title with nothing to say stays Idle.
    #[test]
    fn only_a_spinner_promotes() {
        for title in [
            // A resting glyph: the agent says it is not working.
            "✳ Write tmux haiku",
            // Codex admitting it is blocked, which is not working either.
            "[ ! ] Action Required | bare",
            // No glyph at all.
            "Echo Banana",
            // Furniture.
            "Mac",
            "",
        ] {
            assert_eq!(
                promoted_by_title(AgentActivity::Idle, title, "claude", "Mac", 0),
                AgentActivity::Idle,
                "{title}"
            );
        }
    }

    fn tagged_pane(daemon: Uuid, terminal: Uuid) -> farcooler_core::inventory::TaggedPane {
        farcooler_core::inventory::TaggedPane {
            daemon_id: daemon,
            workspace_id: Uuid::from_u128(2),
            terminal_id: terminal,
            schema_version: 1,
            pane_id: "%1".into(),
            window_id: "@1".into(),
            columns: 80,
            rows: 24,
            left: 0,
            top: 0,
            window_active: true,
            pane_active: true,
            zoomed: false,
            tty: "/dev/ttys001".into(),
            dead: false,
            dead_status: None,
            command: "fish".into(),
            title: String::new(),
        }
    }

    #[test]
    fn a_managed_pane_without_a_terminal_record_is_an_orphan() {
        let daemon = Uuid::from_u128(1);
        let terminal = Uuid::from_u128(3);
        let pane = tagged_pane(daemon, terminal);

        assert!(orphaned_pane(&pane, daemon, &HashSet::new()));
        assert!(!orphaned_pane(&pane, daemon, &HashSet::from([terminal])));
    }

    #[test]
    fn another_daemons_pane_is_never_reaped() {
        let pane = tagged_pane(Uuid::from_u128(9), Uuid::from_u128(3));
        assert!(!orphaned_pane(&pane, Uuid::from_u128(1), &HashSet::new()));
    }

    #[test]
    fn only_a_blocked_or_finished_agent_is_worth_a_phone() {
        // Working is the NORMAL case. Something that buzzes for the normal case
        // is something people turn off, after which it cannot tell them the
        // thing that mattered — so this is the rule the whole feature rests on.
        let asking = Some("Do you want to create haiku.txt?");
        assert!(notification(AgentActivity::Working, "claude", None).is_none());
        assert!(notification(AgentActivity::Idle, "claude", None).is_none());
        assert!(notification(AgentActivity::Unspecified, "claude", None).is_none());
        // Not even with a question in hand: a question is what a card SAYS, not
        // what decides there is one.
        assert!(notification(AgentActivity::Working, "claude", asking).is_none());
    }

    #[test]
    fn a_blocked_agent_says_what_it_wants() {
        let notice = notification(AgentActivity::Blocked, "claude", None).expect("blocked");
        assert_eq!(notice.title, "claude needs you");
        assert_eq!(notice.subtitle, "Waiting for your answer");
        // What raises the live card. The relay is deliberately not in the
        // business of reading "needs you" out of the title, so if this word
        // ever changes shape the lock screen stops working and nothing else
        // does — which is why it is asserted rather than left to the copy.
        assert_eq!(notice.status, "blocked");
    }

    /// The question is the payload, and the lock screen is where it matters.
    ///
    /// Derived on the host, redacted, and carried on four paths — and until it
    /// reached this subtitle, a phone could only say that SOMETHING was being
    /// asked. Answering from the lock screen needs the question on it.
    #[test]
    fn a_blocked_agent_puts_its_question_on_the_card() {
        let notice =
            notification(AgentActivity::Blocked, "claude", Some("Do you want to create haiku.txt?"))
                .expect("blocked");
        assert_eq!(notice.title, "claude needs you");
        assert_eq!(notice.subtitle, "Do you want to create haiku.txt?");
        assert_eq!(notice.status, "blocked");

        // A screen too garbled to yield one falls back rather than showing a
        // blank second line — see `registry.blocked_question`, which returns
        // None for a trust gate whose '?' is mid-line.
        for absent in [None, Some(""), Some("   ")] {
            let notice = notification(AgentActivity::Blocked, "claude", absent).expect("blocked");
            assert_eq!(notice.subtitle, "Waiting for your answer", "{absent:?}");
        }
    }

    #[test]
    fn a_finished_agent_needs_no_second_line() {
        let notice = notification(AgentActivity::Done, "claude", None).expect("done");
        assert_eq!(notice.title, "claude finished");
        assert!(notice.subtitle.is_empty());
        // And what takes the card back down. An empty subtitle is fine; an
        // empty status would leave a live card up on the lock screen until the
        // system expired it hours later.
        assert_eq!(notice.status, "done");

        // A question left over from the prompt this agent has just finished
        // answering must not ride along: "claude finished / Do you want to
        // create haiku.txt?" reads as a question still open.
        let notice =
            notification(AgentActivity::Done, "claude", Some("Do you want to create haiku.txt?"))
                .expect("done");
        assert!(notice.subtitle.is_empty(), "{}", notice.subtitle);
    }

    #[test]
    fn a_failed_exit_names_the_code() {
        let notice = exit_notice("cargo build", Some(101), None);
        assert_eq!(notice.title, "cargo build failed");
        assert_eq!(notice.subtitle, "Exit code 101");
        // Not "blocked": nothing is waiting on an answer, so there is no live
        // card to raise. Reusing "done" is what keeps the relay's vocabulary at
        // two words instead of a third it was never taught.
        assert_eq!(notice.status, "done");
    }

    #[test]
    fn a_signal_names_the_signal_not_the_code() {
        // A process killed by a signal has no exit code at all, so the
        // sentence has to come from the signal or say nothing useful.
        let notice = exit_notice("cargo build", None, Some(9));
        assert_eq!(notice.subtitle, "Stopped by signal 9");
    }

    /// The rule that decides whether a state transition reaches a phone.
    ///
    /// Five cases, matching exactly what the brief asked this to cover: a
    /// failure pushes, a signal pushes, a clean exit does not, a repeat tick on
    /// an already-exited terminal does not, and neither does the first sighting
    /// of a terminal that was already dead when the daemon found it.
    #[test]
    fn a_command_failing_pushes_exactly_once() {
        use TerminalState::{Exited, Running};

        // A non-zero exit, crossing into Exited for the first time: pushes.
        assert!(exited_into_failure(false, Running, Exited, Some(101), None));
        // A signal, same transition: pushes.
        assert!(exited_into_failure(false, Running, Exited, None, Some(9)));
        // A clean exit must never push — this is the rule that keeps the
        // feature switched on rather than muted for closing an ordinary shell.
        assert!(!exited_into_failure(false, Running, Exited, Some(0), None));
        // Already Exited last tick: this is not the transition, it is the
        // pane having been dead for a while, and must not push again.
        assert!(!exited_into_failure(false, Exited, Exited, Some(101), None));
        // A terminal that was already dead the first time this daemon ever saw
        // it: restarting the daemon must not buzz for every failed build in
        // the fleet's history.
        assert!(!exited_into_failure(true, Running, Exited, Some(101), None));
    }

    /// The gate that keeps this off the hot path.
    ///
    /// Both `worktree add` and `worktree remove` create or delete a directory
    /// under `$GIT_COMMON_DIR/worktrees`, which moves its mtime. Scanning every
    /// repository every second would spawn a git process per repository per
    /// second to learn nothing.
    #[test]
    fn the_gate_opens_only_when_the_worktrees_directory_moves() {
        let dir = tempfile::tempdir().unwrap();
        let common = dir.path().join(".git");
        let worktrees = common.join("worktrees");
        std::fs::create_dir_all(&worktrees).unwrap();

        let first = worktrees_changed(&common, None).expect("no baseline means changed");
        assert_eq!(worktrees_changed(&common, Some(first)), None, "unchanged stays shut");

        // Coarse filesystem timestamps: without this the write can land inside
        // the same tick as the read and the mtime genuinely does not move.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::create_dir(worktrees.join("side")).unwrap();

        assert!(worktrees_changed(&common, Some(first)).is_some(), "adding a worktree opens it");
    }

    /// A repository with no linked worktrees has no such directory, and that is
    /// not a change — it is the normal state of most repositories.
    #[test]
    fn a_repository_with_no_linked_worktrees_does_not_thrash_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        let common = dir.path().join(".git");
        std::fs::create_dir_all(&common).unwrap();

        assert_eq!(worktrees_changed(&common, None), None, "nothing there, nothing to scan");
    }

    #[test]
    fn an_agent_pane_still_announces_finishing_exactly_once() {
        // The transition the whole feature exists for. The supervisor folds its
        // own events to `Done`; the watcher must read that as "the agent is
        // sitting there" and reach `Done` through its OWN fold, off the
        // `Working` it last reported.
        let done = activity::advance(AgentActivity::Working, agent_observation(AgentActivity::Done));
        assert_eq!(done, AgentActivity::Done);

        // And it stays announced while nobody has looked, sample after sample.
        let still = activity::advance(done, agent_observation(AgentActivity::Done));
        assert_eq!(still, AgentActivity::Done);
    }

    #[test]
    fn opening_an_agent_pane_ends_done_and_it_stays_ended() {
        // The bug this guards: the supervisor goes on reporting `Done` forever
        // — nothing about a user opening a terminal reaches it — so folding its
        // activity in as an observation re-derived `Done` on the very next
        // sample, one second after `mark_seen` had cleared it. The row lit up
        // again and notified again, every single time it was read.
        let cleared = activity::seen(AgentActivity::Done);
        assert_eq!(cleared, AgentActivity::Idle);

        let next = activity::advance(cleared, agent_observation(AgentActivity::Done));
        assert_eq!(next, AgentActivity::Idle, "seeing a finished agent must stick");
    }

    #[test]
    fn an_agent_that_asks_a_question_after_being_seen_still_says_so() {
        // Dropping `Done` must drop only `Done`. Blocked is not idle-and-unseen
        // — it is the agent genuinely waiting on an answer — and looking at it
        // does not answer it.
        let next = activity::advance(AgentActivity::Idle, agent_observation(AgentActivity::Blocked));
        assert_eq!(next, AgentActivity::Blocked);
    }
}
