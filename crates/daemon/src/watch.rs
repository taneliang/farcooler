//! Watching what the agents are doing, and telling everyone.
//!
//! Two jobs that belong together because they share one loop:
//!
//! 1. **Derive each agent's activity** by reading its screen. This has to be
//!    the daemon's job. A phone has no screen to inspect, and a Mac that
//!    inspected one would be a second authority disagreeing with the runner about
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
use farcooler_core::session_log::tail::Tail;
use farcooler_core::session_log::{TaskStatus, TurnEvent, TurnOutcome};
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

/// How long one repository goes unreconciled with no event to prompt it.
///
/// The backstop half of a gate-plus-backstop pair, in the shape
/// `LOG_JOIN_BACKSTOP_MS` has rather than the whole-fleet forced tick this
/// replaces: measured per repository from its own last pass, and OR'd into the
/// same boolean the event gate answers.
///
/// It is a **watcher-failure** backstop and nothing else. Staleness is covered
/// — `fs_watch` is told when `<common>/worktrees` moves — and what is not
/// covered is a watcher that was never registered, or one that stopped
/// delivering without saying so, which is exactly the class of defect this
/// project keeps finding. Five minutes rather than thirty seconds because it
/// no longer has to be fast enough to be the primary signal: it only has to be
/// fast enough that a runner whose watcher died is not wrong all night. The
/// cost when it fires is one `git worktree list` per repository, which is what
/// makes five minutes affordable and what made thirty seconds affordable
/// before it.
const RECONCILE_BACKSTOP_MS: i64 = 300_000;

/// How long one worktree goes unprobed with no event to prompt it.
///
/// The same shape and the same reason, over the expensive pass rather than the
/// cheap one: three git processes per worktree, so the number has to be much
/// larger. Ten minutes on an idle fleet of twenty worktrees is sixty git
/// processes every ten minutes — against the zero it cost before, and against
/// a three-second poll spending the same sixty every three seconds if the
/// gates ever stopped answering.
///
/// This is the one place in this module that spends git on a clock, and it is
/// spent buying back the guarantee the watcher cannot give about itself: that
/// it is still working. A daemon that silently stopped noticing edits is worse
/// than a daemon that pays a little to find out.
const CHANGE_SET_BACKSTOP_MS: i64 = 600_000;

/// How often the sidebar's `+N -M` is re-derived for a worktree that moved.
///
/// Three seconds, chosen against the clients rather than against this loop. The
/// Mac coalesces its inbox reads to one every three seconds
/// (`DaemonClient.changesInboxFloor`) and the phone polls on the same three, so
/// a number computed faster than this is a `git diff` nobody will ever draw.
/// Slower, and the count would visibly lag the pane beside it, which moves every
/// second — a row whose agent is live while its numbers are minutes old is the
/// whole complaint these counts exist to answer.
const CHANGE_SET_INTERVAL: Duration = Duration::from_secs(3);

/// How long a client's word that it is looking at a pane stays good for.
///
/// The freshness bound on `terminal.watching`, and the whole reason a claim of
/// attention is safe to act on at all. Suppression is the one thing in this
/// module that makes a notification NOT happen, so the failure that matters is
/// not a stale claim being ignored — it is a stale claim being believed: a
/// phone that crashed, a Mac that was suspended with the lid closed, an SSH
/// link that died mid-poll, any of which leaves a terminal that will never
/// interrupt anyone again. Nothing has to be cleaned up by the client that went
/// away, because the claim expires whether or not anybody notices it did.
///
/// Ten seconds, which is three of the phone's three-second polls and three of
/// the Mac's beats on the same cadence — see `Connection.reportWatching` and
/// `DaemonClient.reportWatching`. Two consecutive round trips can be lost to a
/// slow link without a person who is plainly sitting there watching being
/// buzzed about the pane in front of them, and a client that genuinely stops
/// costs at most ten seconds of silence on the one terminal it named. Shorter
/// would make a hiccup on a hotel network read as absence; longer would spend
/// the window in the direction where the mistake is a missed notification
/// rather than a redundant one.
///
/// The ten seconds is a ceiling and rarely the actual cost: both apps send an
/// empty claim as they go to the background or lose focus, which releases every
/// terminal they named at once. See `Watcher::report_watching`.
const WATCHED_TTL_MS: i64 = 10_000;

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
    /// `"working"`, `"blocked"` or `"done"`, and nothing else — an empty or
    /// invented status tells the relay it is talking to a daemon it should stop
    /// trusting. It is `&'static str` so there is nowhere for a computed one to
    /// come from.
    ///
    /// The relay reads this to decide which of two very different things a
    /// notice becomes: `blocked` and `done` raise a banner, `working` only ever
    /// moves the card — starting it silently when there is none, and updating it
    /// in place when there is. That split is what lets a working agent be
    /// reported at all without buzzing anyone: what separates the tiers is the
    /// alert, never whether a push goes out.
    status: &'static str,
    /// Whether this `done` is a turn that DIED.
    ///
    /// Beside `status` rather than folded into it, because the two answer
    /// different questions and only one of them decides anything about the lock
    /// screen. `status` is "raise, move or dismiss the card"; a failed turn is
    /// still over, so it is still `"done"` for that purpose — see `exit_notice`.
    /// What this carries is which MARK the agent gets, and that cannot be
    /// recovered downstream: `title` says "claude failed" in a human sentence,
    /// and a relay or an extension reading a verb out of one would be this
    /// module's rule about deciding-in-one-place broken in a second language.
    ///
    /// The phone is where it matters most. The notification service extension
    /// draws `feed::glyph`'s marks from `status` alone, and
    /// `accessoryCircular` draws only the glyph — so without this field an
    /// agent whose turn died wears `✓` on the lock screen until the app next
    /// polls. Always `false` for `blocked` and `working`, neither of which has
    /// ended at all.
    failed: bool,
    /// When the turn this notice is about began, in Unix milliseconds, or
    /// `None` when no turn is running.
    ///
    /// Carried on the notice rather than read at the send site because it has
    /// to come off the SAME record the title and the question came off. Read
    /// separately, a card could quote one turn's question over another turn's
    /// clock — which is not a wrong pixel, it is a wrong duration next to a
    /// right question.
    ///
    /// `None` is a real answer and never a zero: the relay omits an absent
    /// clock, and a card given zero would count from 1970. See
    /// `crate::push::Notification::started_at` for the rest of that contract.
    started_at: Option<i64>,
}

/// What, if anything, is worth waking a phone for.
///
/// Split out from the sending so it can be tested at all: the rule it encodes —
/// which states reach a phone at all, and as what — is the difference between a
/// product people keep notifications on for and one they mute, and the sending
/// half is a spawn and a filesystem read that no test can reach through.
///
/// Three states out of five produce a notice, but only two of them BUZZ. The
/// relay is what separates them: it builds a banner for `blocked` and `done`,
/// and for `working` it moves the card and nothing else — including raising one
/// silently when the run has none yet, which is what makes the card follow a
/// whole run rather than appear only once something has gone wrong. So the
/// working arm below is not a relaxation of the rule that a busy agent must not
/// interrupt anyone — that rule is about alerts and it is untouched.
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
///
/// `failed` is how the turn ENDED, and it changes the wording of a `Done` and
/// nothing else. The phone is where the sidebar's `✗` would otherwise be lost
/// entirely, and being told an agent "finished" when its turn died is the same
/// lie in the place it costs most. The relay's status stays `"done"` for the
/// reason `exit_notice` gives below: a failed turn is a turn that is OVER,
/// just over badly, and there is no live card to hold open for it.
///
/// `started_at` is `Observed::turn_started_at` — when the user's request began,
/// held across Blocked so that approving a tool call does not restart the clock.
/// It is passed through unread: nothing here decides anything by it, and the
/// only shape it can take is the one the caller was already holding. Its whole
/// job is to reach the attributes of the card the relay starts, which is where
/// a lock screen gets a timer that ticks with no push behind it.
///
/// Every tier carries it, including the two the relay does not start a card
/// from today. A `Done` turn has no clock to run and arrives as `None` on its
/// own, so there is nothing to special-case — and a tier that dropped the field
/// would be a card started from it with the timer silently missing.
/// What a notice QUOTES beyond naming a state, taken off the record that
/// produced it.
///
/// A struct rather than three more parameters because all three travel
/// together from `push_if_paired` down, and the two functions they cross were
/// already at clippy's argument limit — but mostly because they are one idea:
/// the second line of a notification, which is which pane this is about
/// followed by whatever that pane currently has to say for itself.
///
/// `workspace` is the newest of the three and the one the product owner asked
/// for outright. Every push said `codex finished` and nothing more, and they
/// run several codexes at once, so the notification identified nothing — the
/// agent's name is what a pane has in COMMON with the others, not what tells
/// it apart. Both local notifiers had already answered this the same way,
/// with a body of `workspace — said`; see `Quoted::body`, which is that shape
/// written once so the phone's push and the Mac's banner about one pane say
/// one thing.
#[derive(Debug, Clone, Copy, Default)]
struct Quoted<'a> {
    /// Which worktree this pane belongs to, by the name a client shows for it:
    /// `ws.name()`, the same string `wire::workspace` sends as `task_name`.
    ///
    /// Not an `Option`, because every terminal belongs to a workspace and the
    /// sampling loop is already iterating them to reach this pane at all. It
    /// can still arrive EMPTY — `Quoted::default()`, and a name derived from a
    /// path that has none — and `body` treats that as "nothing to say about
    /// which pane" rather than drawing a leading dash.
    workspace: &'a str,
    /// What the agent is asking, while it is asking it. Read off the screen.
    question: Option<&'a str>,
    /// The last thing the agent said, from its OPENING. Read out of its own
    /// log — see `farcooler_core::feed::Feed::said`, which is also where the
    /// "from its opening" is decided and why it is not the feed's last line.
    said: Option<&'a str>,
}

impl Quoted<'_> {
    /// The text, or nothing when it is absent or blank. Both fields arrive
    /// from places that can legitimately hand over an empty string, and an
    /// empty subtitle is a blank second line on a lock screen.
    fn worth_saying(text: Option<&str>) -> Option<&str> {
        text.map(str::trim).filter(|t| !t.is_empty())
    }

    /// One notification body: which pane this is about, then what there is to
    /// say about it.
    ///
    /// `add-auth — Both tests pass.` The workspace goes IN FRONT rather than
    /// into the title, because the title is already `codex finished` and a
    /// title is what a notification is sorted and stacked by — and because
    /// this is the shape both local notifiers have written since they gained a
    /// body at all, and the owner reads a Mac banner and a phone push about
    /// the same pane.
    ///
    /// Either half can be missing and the result still reads as a sentence: a
    /// turn can be all tool calls and say nothing, and a workspace name can be
    /// empty. Neither produces a stray dash — `— Both tests pass.` looks like
    /// a rendering bug, and a lock screen is not where to debug one.
    fn body(&self, text: Option<&str>) -> String {
        match (Self::worth_saying(Some(self.workspace)), Self::worth_saying(text)) {
            (Some(workspace), Some(text)) => format!("{workspace} — {text}"),
            (Some(workspace), None) => workspace.to_string(),
            (None, Some(text)) => text.to_string(),
            (None, None) => String::new(),
        }
    }
}

fn notification(
    activity: AgentActivity,
    label: &str,
    quoted: Quoted<'_>,
    failed: bool,
    started_at: Option<i64>,
) -> Option<Notice> {
    match activity {
        AgentActivity::Blocked => Some(Notice {
            title: format!("{label} needs you"),
            // The question is what a person answers from the lock screen, and
            // the workspace is what tells them which of three blocked codexes
            // is asking it. Both, in that order, for the same reason `Done`
            // carries both: `codex needs you / Do you want to create
            // haiku.txt?` is answerable and still not attributable.
            subtitle: quoted
                .body(Some(Quoted::worth_saying(quoted.question).unwrap_or("Waiting for your answer"))),
            status: "blocked",
            failed: false,
            started_at,
        }),
        AgentActivity::Done if failed => Some(Notice {
            title: format!("{label} failed"),
            subtitle: quoted.body(Some("Its last turn didn’t finish")),
            status: "done",
            failed: true,
            started_at,
        }),
        // The last thing the agent SAID, from the start of it, under the name
        // of the worktree it said it in.
        //
        // This subtitle was empty once, so the whole notification was "claude
        // finished": a sentence about Far Cooler, arriving on a lock screen,
        // saying nothing about the work it is reporting on. Then it was the
        // agent's words alone, which is a sentence about the work and still
        // not about WHICH work — the owner runs several codexes at once, and
        // `codex finished` names the one thing they all have in common.
        //
        // What is quoted is `Feed::said`, not the feed's last line: a feed
        // entry is a wrapped row, so its last one is the last forty characters
        // of the window, which is how a turn that ended "More shit. An
        // industrial quantity of shit, shipped in carefully authorized batches
        // to avoid N+1 shits." reached a phone as "batches to avoid N+1
        // shits." That is a fact about where the string was cut and not about
        // how the turn was parsed, so it is fixed where the cut is made.
        //
        // The workspace alone when the agent said nothing this turn, which is
        // a real case — a turn can be all tool calls — and reads as a row that
        // has nothing to add rather than as a blank line pretending to be
        // content.
        AgentActivity::Done => Some(Notice {
            title: format!("{label} finished"),
            subtitle: quoted.body(quoted.said),
            status: "done",
            failed: false,
            started_at,
        }),
        // A card, not a banner. `push_notice`'s caller decides which of the two
        // this becomes — the relay sends an alert for `blocked` and `done` and
        // only ever moves the card for `working`, silently, whether that means
        // starting one or updating it — so producing a notice here does NOT
        // reintroduce buzzing for the normal case. What it buys is a Dynamic
        // Island that says something during the only period there is anything
        // to watch.
        //
        // `title` is the label alone. The relay never builds an alert from a
        // working notice, so there is no sentence for it to be the subject of.
        //
        // `question` here is the composed signal line, not a question: a
        // working agent has nothing to ask. The parameter is "whatever the card
        // should say under the label", and for `Blocked` that happens to be
        // what the agent asked.
        //
        // And the one tier with no workspace in front of it, which is the
        // difference between a card and a banner. A banner arrives once, into a
        // stack of other banners, and has to say which pane it is about or it
        // says nothing. A card is a rectangle the relay updates IN PLACE for
        // the length of one run — it is already identified by being the card it
        // is — and its second line is `feed::signal`'s composed rung, a
        // forty-character budget already spent saying where the agent is.
        // Spending a quarter of it restating the worktree would cost the task
        // position that is the only thing on there worth watching.
        AgentActivity::Working => Some(Notice {
            title: label.to_string(),
            subtitle: quoted.question.unwrap_or("").to_string(),
            status: "working",
            failed: false,
            started_at,
        }),
        _ => None,
    }
}

/// How often a live card may be refreshed while an agent stays in one tier.
///
/// A signal line can change several times a second, and pushing each one spends
/// the app's Live Activity budget on text nobody can read at that rate. Ten
/// seconds is roughly the fastest a changing line is still worth reading rather
/// than watching flicker, and it caps a six-agent fleet at 36 pushes a minute.
///
/// A starting number, not a measured one — see the design's risk 2.
const CARD_REFRESH_MS: i64 = 10_000;

/// Whether a within-tier card refresh is due.
///
/// Only ever asked about `working`. A tier change — working to blocked to done
/// — does not come through here at all: those are the pushes this whole feature
/// exists to deliver, and withholding one to save a byte of budget would be
/// throttling the alarm to keep the clock quiet.
fn should_refresh_card(last_push_ms: Option<i64>, now_ms: i64) -> bool {
    match last_push_ms {
        None => true,
        Some(at) => now_ms.saturating_sub(at) >= CARD_REFRESH_MS,
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
///
/// `workspace` for the same reason an agent's notice carries one, and it is
/// the same complaint one step over: `cargo build failed / Exit code 101` at
/// three in the morning names neither the worktree it broke in nor anything
/// else that would tell it from the other two builds running. The Mac's local
/// `reportFailedExit` has written `workspace — exit code 101` all along, so
/// this is also the push catching up with the banner beside it.
fn exit_notice(
    label: &str,
    workspace: &str,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
) -> Notice {
    let outcome = match (exit_code, exit_signal) {
        (_, Some(signal)) => format!("Stopped by signal {signal}"),
        (Some(code), _) => format!("Exit code {code}"),
        // Reached only if this is ever called outside `exited_into_failure`'s
        // guard, which already requires one of the two above.
        (None, None) => String::new(),
    };
    // Through the same join every other notice's body goes through, so an
    // absent workspace cannot leave a leading dash here either.
    let subtitle = Quoted { workspace, ..Quoted::default() }.body(Some(&outcome));
    // No clock. This is a command that exited, not an agent turn — there is no
    // `turn_started_at` at this call site to be right about, and the relay ends
    // a card on `done` rather than starting one, so a timer would have nowhere
    // to appear even if one could be invented here.
    Notice {
        title: format!("{label} failed"),
        subtitle,
        status: "done",
        failed: true,
        started_at: None,
    }
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
/// What the last probe of one workspace left behind for the trace.
///
/// Two running positions rather than two counts: the trace records CHANGE, and
/// both sources here report a total. `lines` is `insertions + deletions` for
/// the whole diff against the base, so its growth is the code written since the
/// last look; `reflog` is a byte offset into `<git dir>/logs/HEAD`, so
/// everything past it is HEAD moves nobody has counted yet.
#[derive(Debug, Clone, Copy, Default)]
struct TraceLevel {
    /// `None` until this workspace has been probed once.
    ///
    /// The distinction is the whole correctness of the upper half: a worktree
    /// already carrying a 4,000-line diff when the daemon starts has not just
    /// gained 4,000 lines, and recording the first reading as growth would draw
    /// a bar on every workspace on the runner at every restart.
    lines: Option<u32>,
    /// How far into `<git dir>/logs/HEAD` the commit marks have been read.
    ///
    /// `None` on the first pass, and that one deliberately reads the file whole
    /// — unlike `lines` above, a reflog is a list of dated events rather than a
    /// level, so the day already in it belongs in the buckets it happened in.
    /// A daemon that has just started therefore has real commit marks
    /// immediately rather than a blank axis until the next commit.
    reflog: Option<u64>,
}

/// What each pane observed working gets out of a worktree's diff growing.
///
/// Pulled out of `record_workspace_trace` so the arithmetic can be argued with
/// directly, because every one of its four answers is a decision:
///
/// - **No previous reading gives nothing.** A worktree already carrying a
///   4,000-line diff when the daemon starts has not just gained 4,000 lines.
/// - **A shrinking diff gives nothing, never a negative.** A commit does not
///   shrink it — the counts are base-to-worktree — but a rebase, a reset or a
///   base change does, and none of those is work being undone in the last three
///   seconds.
/// - **No observed worker gives nothing.** A person editing in their own editor
///   moves the diff with no pane to put it on, and it is dropped rather than
///   shared out among whoever happens to have a terminal in that worktree.
/// - **Several workers split it evenly.** Neither git nor this daemon can say
///   which pane wrote a line, and several terminals routinely share a worktree.
///   An even split keeps the fleet sum exact and makes each row an
///   approximation, which is the direction that loses least; the alternative,
///   giving the whole to each, would multiply the fleet's upper half by however
///   many panes happened to be open.
fn code_share(previous: Option<u32>, total: u32, workers: usize) -> u32 {
    if workers == 0 {
        return 0;
    }
    let Some(previous) = previous else { return 0 };
    total.saturating_sub(previous) / workers as u32
}

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
    ///
    /// The FALLBACK gate now, not the primary one — `fs_watcher` is told when
    /// that directory moves. Kept live rather than deleted, and consulted only
    /// for a repository the watcher does not cover: a registration that failed
    /// must leave the daemon where it was, not somewhere worse.
    worktree_marks: std::sync::Mutex<HashMap<Uuid, std::time::SystemTime>>,
    /// When each repository was last reconciled.
    ///
    /// A std mutex on the same terms. Absent for a repository nothing has
    /// reconciled, which is what makes the first pass after a daemon start
    /// reach all of them — see `RECONCILE_BACKSTOP_MS`.
    worktree_reconciles: std::sync::Mutex<HashMap<Uuid, i64>>,
    /// When each workspace's `+N -M` was last computed.
    ///
    /// A std mutex on the same terms as `worktree_marks`. Absent for a worktree
    /// nothing has probed, which is what makes the first pass after a daemon
    /// start reach all of them.
    change_set_probes: std::sync::Mutex<HashMap<Uuid, i64>>,
    /// When an agent was last seen working in each workspace.
    ///
    /// The signal that makes live counts possible at all. An agent edits files
    /// with its own tools in its own pane — not through this daemon — so
    /// `review::cheap_gate` sits still through an entire turn and
    /// `ReviewCache::touched_at` never hears about it. What this process DOES
    /// know, because it is already reading every pane once a second for the
    /// fleet rows, is which panes have an agent working in them. Stamped in
    /// `sample()` where that is decided, and spent in `probe_change_sets`.
    ///
    /// A std mutex, and written under the `state` lock without ever being held
    /// across an await — a map insert of two words.
    change_set_activity: std::sync::Mutex<HashMap<Uuid, i64>>,
    /// Which terminals were seen working in each workspace since the last
    /// change-set probe.
    ///
    /// `change_set_activity` above answers "did anyone work here", which is all
    /// its gate needs. The activity trace needs "who", because the upper half
    /// of a row's trace is that row's code — and a worktree's diff growing is a
    /// fact about the WORKSPACE. Several terminals routinely share one, and git
    /// records no author per pane, so the growth is split evenly among the panes
    /// this loop actually observed working while it happened. See
    /// `probe_change_sets`, which is the only reader, and the note on
    /// `Terminal.activity_trace` in the proto, which states the approximation
    /// to clients.
    ///
    /// A std mutex on the same terms as the map above: written under the
    /// `state` lock, never held across an await.
    trace_workers: std::sync::Mutex<HashMap<Uuid, HashSet<Uuid>>>,
    /// Each terminal's thirteen buckets of code, output and commits.
    ///
    /// The daemon's copy is a ring of 312 five-minute slots — 3.7KB a terminal —
    /// and the wire carries the 66-byte rendering of thirteen of them. See
    /// `farcooler_core::trace`, and in particular why the bucket edges are
    /// absolute wall-clock seconds and must stay that way.
    ///
    /// A std mutex: every use is a map lookup and an array add, never an await.
    traces: std::sync::Mutex<HashMap<Uuid, farcooler_core::trace::Trace>>,
    /// The shape of each terminal's screen as of the previous tick.
    ///
    /// One `u64` per visible row, which is what `trace::lines_produced` compares
    /// to tell how far a pane scrolled. Nothing else survives a tick — a pane's
    /// worth of text held per terminal for a fleet of thirty is exactly the kind
    /// of quiet cost this loop is careful about.
    trace_shapes: std::sync::Mutex<HashMap<Uuid, farcooler_core::trace::ScreenShape>>,
    /// The last `insertions + deletions` seen for each workspace, and how far
    /// into its reflog the commit marks have been read.
    ///
    /// Levels, not events: `ReviewCache::counts` reports a worktree's whole
    /// diff against its base, so what the trace wants is its GROWTH between two
    /// probes. Absent for a workspace nothing has probed, which is what stops
    /// the first sighting of an existing diff from being drawn as if the agent
    /// had written all of it in the last five minutes.
    trace_levels: std::sync::Mutex<HashMap<Uuid, TraceLevel>>,
    /// Commit marks, per WORKSPACE rather than per terminal.
    ///
    /// Kept apart from `traces` because a commit is not a pane's. Several
    /// terminals share a worktree and git records nothing about which one a
    /// commit was typed in, so the honest statement is "a commit landed in this
    /// worktree" — which is what the axis mark means, and which is the same
    /// true statement on every row of that worktree.
    ///
    /// It also keeps the fleet trace right. Terminal rings are summed for the
    /// fleet, so a commit copied onto three panes would be three marks; held
    /// here it is added exactly once.
    commit_traces: std::sync::Mutex<HashMap<Uuid, farcooler_core::trace::Trace>>,
    /// Which workspace each terminal's trace should take its commit marks from.
    ///
    /// The poll and push finishers are handed a terminal id and nothing else,
    /// and both have to reach the same ring. Maintained by `tick_traces` off
    /// the same sample that maintains the rings themselves, so it cannot name a
    /// workspace the fleet no longer has.
    trace_workspaces: std::sync::Mutex<HashMap<Uuid, Uuid>>,
    /// Whether a change-set pass is still running.
    ///
    /// See `spawn_change_set_probe`, which is the only thing that reads or
    /// writes it.
    change_set_probing: std::sync::atomic::AtomicBool,
    /// Each pane's attachment to its own session log.
    ///
    /// A std mutex for the same reason `worktree_marks` is one, and with the
    /// same discipline: the entry is REMOVED from the map before any file is
    /// touched and put back after, so the lock is never held across the
    /// blocking read — let alone across an await. Separate from `state` because
    /// nothing outside the sampling loop has any business reading a file offset.
    logs: std::sync::Mutex<HashMap<Uuid, PaneLog>>,
    /// The filesystem telling us something under the three log roots moved.
    ///
    /// Drained once per tick, and used for exactly one thing: gating the JOIN,
    /// which is the expensive half (an `lsof` spawn for codex, a directory
    /// listing and a file read for the other two). A session file cannot have
    /// appeared under a root that has not changed, so on a quiet machine this
    /// skips the lookup entirely.
    ///
    /// Never a gate on READING an attached log, and never the only thing that
    /// can trigger a join. `LogWatcher::start` degrades to watching nothing at
    /// all when the platform backend fails to build — it warns and carries on,
    /// which is right for it — and a feature whose only source of turn
    /// boundaries silently switched itself off is exactly the class of defect
    /// this project keeps finding. So `LOG_JOIN_BACKSTOP_MS` tries anyway on its
    /// own clock, the same gate-plus-backstop shape `reconcile_worktrees`
    /// already has for git.
    log_watcher: crate::log_watch::LogWatcher,
    /// The filesystem telling us a worktree, or its `.git`, moved.
    ///
    /// The sibling of `log_watcher` one layer down, and what retired the two
    /// polls this module used to run on every tick: `review::cheap_gate`'s two
    /// `stat`s per worktree every three seconds, and `worktrees_changed`'s one
    /// `stat` per repository every second. See `fs_watch`, which explains why
    /// registration is gitignore-aware and why the registration strategy
    /// differs by platform.
    ///
    /// Not the only thing that can trigger either pass, for the same reason
    /// `log_watcher` is not: it reports whether it covers a subject, the
    /// gates it replaced stay live for the subjects it does not, and
    /// `CHANGE_SET_BACKSTOP_MS` and `RECONCILE_BACKSTOP_MS` try anyway on
    /// their own slow clocks. An `Arc` because the registration walk is done
    /// from a spawned pass rather than from `new`.
    fs_watcher: Arc<crate::fs_watch::TreeWatcher>,
    /// What each connected client says it is currently showing a person.
    ///
    /// A std mutex, on the same terms as `worktree_marks` above: every access is
    /// a map lookup over a handful of entries and none of them is held across an
    /// await. It is deliberately NOT part of `state` — `state` is what the
    /// runner has DERIVED about a pane, and this is what a client has ASSERTED
    /// about a person, which is a different kind of fact with a different
    /// lifetime. Folding the two together would also mean an entry could only
    /// exist for a terminal the sampler has already met, and a claim about a
    /// pane arrives whenever the client happens to poll.
    ///
    /// Keyed by client rather than by terminal, and that is what makes a claim
    /// withdrawable. A terminal-keyed map can only ever be added to — nothing in
    /// it records WHO said so, so a client that navigates to another pane has no
    /// way to say it stopped looking at the first, and the only release left is
    /// waiting out `WATCHED_TTL_MS`. Keyed this way, a client's next call
    /// REPLACES its whole set, so switching panes releases the old one in the
    /// same round trip that claims the new one. See `TerminalsWatched` in the
    /// proto for why the wire carries the whole set rather than a watch/unwatch
    /// pair.
    ///
    /// The key is `Peer::client_id`, or `"-"` for a caller that named no device
    /// — every local socket client, which on this runner means the Mac app
    /// itself. Two local clients therefore share one slot, and that is accepted
    /// rather than worked around: a runner is one Unix user and one person, the
    /// Mac app is the only local caller that sends this, and inventing a
    /// per-connection identity for it would buy multi-user semantics for a
    /// product that has one user.
    watched: std::sync::Mutex<HashMap<String, Watched>>,
}

/// One client's claim about what it is showing, and when it said so.
///
/// A `Vec` rather than a `HashSet`: it holds a Mac window's tiled layout, which
/// is single digits, and a linear scan of four ids is faster than hashing one.
struct Watched {
    /// When this claim arrived, in Unix milliseconds. What `WATCHED_TTL_MS` is
    /// measured against, and the reason a client that goes away silently stops
    /// silencing anything.
    at: i64,
    /// The terminals this client said were in front of a person. Empty is a
    /// real and useful claim — "I am looking at nothing" — which is what an app
    /// sends as it backgrounds.
    terminals: Vec<Uuid>,
}

/// One terminal as this tick found it, before anything is made of it.
///
/// The host-wide reads — `ps`, `lsof`, the fleet, the pane inventory — happen
/// once and are joined here, so the classification pass below can run per
/// terminal without going back to the runner for anything but that terminal's
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
    /// The pane's foreground process, which is the only handle codex's session
    /// log can be found by — it holds its rollout file open, so `lsof -p` names
    /// it. `None` for a pane `ps` had nothing to say about.
    pid: Option<i32>,
    /// Where the pane's work lives, which is what claude's and cursor's project
    /// directories are named after.
    ///
    /// The workspace's worktree path, not a live reading of the process's own
    /// cwd. A pane that has been `cd`-ed somewhere else fails the join and falls
    /// back to the title and the screen, which is the right failure: attaching a
    /// pane to the wrong conversation is worse than attaching it to none.
    cwd: String,
    /// Which worktree this pane belongs to, by the name a client shows for it.
    ///
    /// Carried on the sample rather than looked up at the push site, exactly
    /// as `exit_code` above is and for the same reason: the outer loop is
    /// already iterating workspaces to reach this terminal, so the name is in
    /// hand here, and a second read taken later in the tick could name a
    /// workspace this pass is not otherwise describing.
    ///
    /// The name and not the path. A notification is read on a lock screen and
    /// a path is `host_admin` only — see this module's rule about which fields
    /// carry one — and `add-auth` is the string a person recognizes anyway.
    workspace: String,
    /// The same worktree, by id, for the things that key off one rather than
    /// name one. Carried here for the same reason the name above is: the outer
    /// loop is already holding the workspace row this terminal was reached
    /// through, and a lookup taken later in the tick could answer about a
    /// different fleet than the one this pass is describing.
    workspace_id: Uuid,
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
    /// When a live card was last refreshed for this terminal, in Unix
    /// milliseconds. `None` until the first one.
    ///
    /// Only within-tier refreshes consult it; see `should_refresh_card`. A tier
    /// change clears it back to `None` rather than stamping it, so the first
    /// card of a new turn goes out on the tick the turn starts instead of up to
    /// `CARD_REFRESH_MS` after it.
    last_card_push: Option<i64>,
    /// Whether the turn that just ended, ended badly.
    ///
    /// Beside `activity` rather than inside it: `Done` is created and
    /// destroyed by `activity::advance` alone, and a fourth agent state would
    /// have to be folded, acknowledged and notified about by that same fold.
    /// This is a reading of the SAME `Done` — the ladder narrows it to `✗` and
    /// "failed" (see `wire::rung_subject`), and everything that keys off
    /// `AgentActivity` carries on unchanged.
    ///
    /// Re-derived from the log on every tick rather than latched here, so it
    /// cannot outlive the turn verdict it describes.
    turn_failed: bool,
    /// The last few things the agent SAID, from its own session log.
    ///
    /// NEVER cleared, by anything, including the turn ending — see
    /// `farcooler_core::feed`. Coming back to a machine, what a pane did while
    /// you were away is the most useful thing about it, and a row that emptied
    /// itself the moment the work finished would throw that away at exactly the
    /// moment it started being worth reading.
    feed: farcooler_core::feed::Feed,
    /// Where the agent is: its task list, the agents it spawned, and the last
    /// thing it did. Unlike the feed, cleared at a turn boundary — see
    /// `Signals`.
    signals: Signals,
    /// A candidate state and how many times running it has been seen.
    ///
    /// Hysteresis. A `capture-pane` taken while the footer is being rewritten
    /// can miss it, and one such sample used to be enough to fire a Done and
    /// the notification behind it.
    pending: Option<(AgentActivity, u8)>,
    /// Whether this daemon has yet said anything about the live card the relay
    /// may be holding for this terminal.
    ///
    /// False for every terminal on the first tick after the daemon starts, and
    /// that is the whole reason it exists. A card outlives the process that
    /// raised it — the relay keeps it, and tmux keeps the agent — so a runner
    /// coming back up inherits cards it has no memory of, and a first sighting
    /// is not a transition: the agent that was blocked before the restart is
    /// simply the agent it first sees, and nothing ever pushes the `done` that
    /// would take the card down. Every runner update orphaned every card that
    /// was up, which is a thing about to happen far more often than it does now.
    ///
    /// It settles on the first tick that can answer for the terminal ONE WAY OR
    /// THE OTHER — see `card_still_earned`, whose `None` is "could not tell" and
    /// settles nothing. Not simply the first tick: a daemon that starts while
    /// the tmux server is stalled reads every pane as unknown, and a sweep that
    /// counted that as having looked would skip the entire fleet exactly once,
    /// permanently, on the run where it mattered.
    card_settled: bool,
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

/// How long a log that says a turn is STILL RUNNING is believed after its last
/// event.
///
/// Only that half. A log that says the turn ENDED is not bounded by this at
/// all — see `resolved_activity`, where the asymmetry and the reason for it
/// live.
///
/// Thirty seconds, and the number is chosen by which way the risk runs rather
/// than by how often a log is written. Falling out of trust costs nothing worse
/// than stage 1: the title and then the screen answer instead, and both are
/// usually right — a pane running a five-minute build writes nothing to its log
/// for five minutes while its footer says `esc to interrupt` the whole time.
/// Staying in trust too long costs the failure this whole product exists to
/// avoid: `docs/agent-session-logs.md` records five codex sessions out of 183
/// that end on a `task_started` with no completion ever written, and a reader
/// that waits for that completion holds the row on Working forever, never folds
/// to `Done`, and never notifies about that pane again. That is the exact trap
/// `STALE_TITLE_SAMPLES` closed for the title's spinner, arriving through
/// another door.
const STALE_LOG_MS: i64 = 30_000;

/// How often a pane with no session log yet is looked up again.
///
/// The join is not free — an `lsof` spawn for codex, a directory listing and a
/// file read for the other two — and a pane that has no log usually has none
/// for a reason that will not change this second: codex opens its rollout only
/// once the FIRST TURN IS SUBMITTED, so a freshly started pane genuinely has
/// nothing to find until the user types something. Five seconds is far below
/// how long anyone waits between starting an agent and looking at it, and far
/// above what one spawn per pane per second would cost.
///
/// The same floor covers the other reason a pane is looked up — a join that has
/// stopped being true, see `join_looks_dead` — because it is the same lookup at
/// the same price. What a found log buys is not exemption from this gate but
/// the evidence to stay off it: a file that is being written is never
/// questioned.
const LOG_JOIN_INTERVAL_MS: i64 = 5_000;

/// How long an unresolved pane waits before being looked up with no filesystem
/// event to prompt it.
///
/// The backstop half of the pair — see the `log_watcher` field. A session file
/// cannot appear under a root that has not changed, so ordinarily the watcher
/// says when it is worth looking; this is what keeps the feature working on a
/// machine where the watcher never started, at the cost of one lookup per
/// unresolved pane per thirty seconds.
const LOG_JOIN_BACKSTOP_MS: i64 = 30_000;

/// What an agent's own log says about its turn.
///
/// Deliberately not an `AgentActivity`. A log can say a turn is running, or
/// over, and how it ended — and NOTHING else. No agent writes down that it is
/// waiting for a human (`docs/agent-session-logs.md`, "The one thing none of
/// them do", verified across 276 claude files, 183 codex, and every cursor
/// transcript that exists). Giving this type an `AgentActivity` would make
/// `Blocked` expressible here, and the first thing anyone would do with an
/// expressible state is set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LogTurn {
    /// Whether the last turn boundary seen was a start rather than an end.
    running: bool,
    /// Whether the turn that ended, ended BADLY.
    ///
    /// Only ever true alongside `running: false` — it is a reading of an end,
    /// not a state — and it lives here rather than beside it on `PaneLog` so
    /// that everything which drops a turn's verdict drops this with it. A
    /// pane re-joined to a new session file (`/clear`, a restarted agent)
    /// clears `turn`, and inheriting the previous session's failure would
    /// mark a pane that is working perfectly well.
    failed: bool,
    /// When the DAEMON last read an event out of this log, on the daemon's
    /// clock.
    ///
    /// Not the event's own timestamp, which none of the three formats can be
    /// trusted for: claude's `timestamp` runs backwards between adjacent lines
    /// in 235 of 276 files, codex mixes seconds and milliseconds inside one
    /// payload, and cursor writes a localized human string. The question this
    /// answers is "is this file still being written", which is about when we
    /// read it and not about what the writer believed the time was.
    last_event_at: i64,
}

/// A question the agent asked and is holding for, as its log stated it.
///
/// Kept beside `LogTurn` rather than on it, and that separation is the point.
/// `LogTurn` is a reading of a TURN — running, ended, ended badly — and its
/// doc explains why `Blocked` must stay inexpressible there: a quiet log is not
/// a waiting agent, and the first thing anyone would do with a settable
/// `Blocked` is set it from silence.
///
/// This is not read off silence. It is a `tool_use` the agent wrote down, whose
/// result can come from nowhere but a person, still outstanding — so it says
/// what no amount of quiet ever could. See `session_log::TurnEvent::Asked` for
/// the corpus behind that claim.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ask {
    /// The `tool_use` id, so the answer to THIS question is what clears it. A
    /// second question asked after the first was answered gets its own id, and
    /// an `Answered` naming the old one leaves it standing.
    id: String,
    /// The question in the agent's own words. Empty when the call stated none,
    /// which the notification path renders as its own sentence rather than a
    /// blank line.
    question: String,
}

/// Which of the three formats a pane's log is written in.
///
/// A pane's preset names the agent; the parser is chosen once, when the file is
/// found, rather than re-derived per line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogFormat {
    Claude,
    Codex,
    Cursor,
}

impl LogFormat {
    /// By prefix, the same way `log_join::find_session_log` reads a preset — a
    /// preset string can carry a suffix. `None` for an agent with no parser
    /// (opencode has none), which leaves that pane exactly as stage 1 left it
    /// rather than guessing at a format.
    fn of(preset: &str) -> Option<LogFormat> {
        if preset.starts_with("claude") {
            Some(LogFormat::Claude)
        } else if preset.starts_with("codex") {
            Some(LogFormat::Codex)
        } else if preset.starts_with("cursor") {
            Some(LogFormat::Cursor)
        } else {
            None
        }
    }

    fn parse_line(self, line: &str) -> Vec<TurnEvent> {
        use farcooler_core::session_log::{claude, codex, cursor};
        match self {
            LogFormat::Claude => claude::parse_line(line),
            LogFormat::Codex => codex::parse_line(line),
            LogFormat::Cursor => cursor::parse_line(line),
        }
    }
}

/// One pane's attachment to its own session log.
///
/// Not part of `Observed`, which is `Clone` and is cloned on every announce: a
/// `Tail` is a file offset that must have exactly one owner, and a cloned one
/// would re-read lines the original had already consumed.
struct PaneLog {
    /// The file and how to read it, once found. `None` while the pane has no
    /// log — a plain shell, an agent with no parser, or a codex that has not
    /// been given a first turn yet.
    tail: Option<(Tail, LogFormat)>,
    /// When the join was last attempted, so no pane is looked up once a second
    /// forever — neither one that has never found a log nor one questioning the
    /// log it has. See `LOG_JOIN_INTERVAL_MS`.
    attempted_at: i64,
    /// `None` until the log has actually said something. A pane whose log has
    /// been found but has not been appended to since is not evidence about the
    /// turn either way, and must fall through rather than assert idleness.
    turn: Option<LogTurn>,
    /// The question this pane is holding for, if its log says it asked one and
    /// has not yet recorded an answer.
    ///
    /// Survives ticks, like `turn` and for the same reason: the line that asked
    /// was read once, minutes or hours before the tick that has to say the row
    /// is blocked, and a batch of events is only ever what arrived SINCE the
    /// last read.
    asked: Option<Ask>,
}

/// Everything it takes to find one pane's session log.
///
/// Owned, and a struct rather than four parameters, because it crosses a
/// `spawn_blocking` boundary — borrowed strings cannot make that hop, and four
/// `Option`s and `&str`s in a row is a signature where transposing `cwd` and
/// `title` still compiles and only shows up as a pane quietly attached to
/// nothing.
struct PaneJoin {
    /// Which agent this is, as the registry identified it from the process and
    /// the screen. `None` for a pane that is not a recognized agent at all.
    preset: Option<String>,
    pid: Option<i32>,
    cwd: String,
    title: String,
}

impl PaneLog {
    fn new() -> PaneLog {
        // Zero, not `now`: a pane seen for the first time should be looked up
        // on this tick and not in five seconds.
        PaneLog { tail: None, attempted_at: 0, turn: None, asked: None }
    }

    /// Take what the join just answered, keeping the current file when the
    /// answer is the file already open.
    ///
    /// The identity check is not an optimization, it is the correctness of
    /// re-joining at all: a `Tail` starts at the END of its file, so replacing
    /// one with an identical one would silently swallow whatever the agent
    /// wrote between the two — every re-verification would cost a hole in the
    /// feed, and re-verifying is meant to be cheap enough to do often.
    ///
    /// The same file keeps its verdict, too, including a turn it said was over.
    /// A screen that says `esc to interrupt` above a file that says the turn
    /// ended is either a footer nobody redrew or a turn start this build failed
    /// to parse, and nothing here can tell those apart — so the agent's own
    /// account wins, which is the whole premise of the layer.
    ///
    /// Anything else — a different file, or no answer at all — also drops what
    /// the old file said about the turn. That verdict was an account of a
    /// session this pane is no longer in, and `resolved_activity` believes an
    /// ended turn indefinitely; carrying it across would pin the row on Idle
    /// through the whole of the next session. `None` leaves the pane exactly as
    /// one that never joined: stage 1, and looked up again on the ordinary gate.
    fn adopt(&mut self, found: Option<(std::path::PathBuf, LogFormat)>) {
        if let (Some((path, _)), Some((tail, _))) = (&found, &self.tail) {
            if tail.path() == path {
                return;
            }
        }
        self.tail = found.map(|(path, format)| (Tail::new(path), format));
        self.turn = None;
        // With the turn, and for a sharper version of the same reason: an
        // outstanding question belongs to the session that asked it. A pane
        // re-joined to a new file — `/clear`, a restarted agent — has nothing
        // waiting on it, and carrying one across would pin the row on "needs
        // you" for a question nobody can answer any more.
        self.asked = None;
    }
}

/// Fold a batch of newly appended log events into what the log says.
///
/// Every event refreshes the clock, not only the turn boundaries: an assistant
/// step or a title is the file being written, which is the only thing
/// `last_event_at` is asked about. Only `Started` and `Ended` move `running`,
/// because only those two are the agent stating a turn boundary — a step
/// arriving while no turn is known to be open says the log was attached
/// mid-turn, not that a turn began.
///
/// An empty batch returns what was already believed, unchanged and un-refreshed,
/// which is what lets `STALE_LOG_MS` eventually fire for a log that stopped.
///
/// How the turn ENDED travels with the boundary that ended it, and only three
/// things can set it: an end that failed sets it, an end that did not clears
/// it, and a start clears it because the failure being reported is always the
/// most recent turn's. `TurnOutcome::Aborted` — the user pressed escape —
/// counts as not failed on purpose: a cancelled turn is somebody's decision
/// carried out, and a product that raises an alarm for a decision you made
/// yourself is a product whose alarms get ignored.
fn fold_log_events(turn: Option<LogTurn>, events: &[TurnEvent], now: i64) -> Option<LogTurn> {
    let mut folded = turn;
    for event in events {
        let (running, failed) = match event {
            TurnEvent::Started { .. } => (true, false),
            TurnEvent::Ended { outcome, .. } => (false, *outcome == TurnOutcome::Failed),
            // Not a boundary. Keeps whatever was believed, and if nothing was
            // believed yet it stays unbelieved: a `Step` alone cannot tell a
            // turn that is running from one whose start we simply never saw.
            _ => match folded {
                Some(t) => (t.running, t.failed),
                None => continue,
            },
        };
        folded = Some(LogTurn { running, failed, last_event_at: now });
    }
    folded
}

/// Fold a batch of newly appended log events into the question a pane is held
/// on, if any.
///
/// Three things clear one, and each is the agent saying so rather than this
/// giving up on it:
///
/// - the matching `Answered`, which is the question being answered;
/// - a turn `Ended`, because a turn cannot both be over and be waiting. An
///   abandoned question — the user pressed escape, or answered in a way that
///   ended the turn — closes here even if its own result never arrives;
/// - a turn `Started`, which is a question from a previous turn that outlived
///   its answer in this fold rather than in the file.
///
/// An `Answered` naming a DIFFERENT id changes nothing. That is the whole
/// reason the id is carried: an agent that asks, is answered, and then runs
/// nine more tools would otherwise be cleared nine times over and, worse, a
/// question asked while an older result was still in flight would be cleared by
/// it on arrival.
fn fold_asks(asked: Option<Ask>, events: &[TurnEvent]) -> Option<Ask> {
    let mut held = asked;
    for event in events {
        match event {
            TurnEvent::Asked { id, question } => {
                held = Some(Ask { id: id.clone(), question: question.clone() });
            }
            TurnEvent::Answered { id } => {
                if held.as_ref().is_some_and(|ask| &ask.id == id) {
                    held = None;
                }
            }
            TurnEvent::Started { .. } | TurnEvent::Ended { .. } => held = None,
            _ => {}
        }
    }
    held
}

/// The one decision this stage exists for: who gets to say what an agent is
/// doing, and in what order.
///
/// Highest first, and each layer wins only where what it knows still holds:
///
/// 1. **The screen keeps `Blocked`, outright.** NO AGENT WRITES DOWN THAT IT IS
///    WAITING ON A PERMISSION DECISION — claude records the OUTCOME of one and
///    never the pending state, codex has nothing approval-shaped in 183 files,
///    cursor shows none in the four that exist
///    (`docs/agent-session-logs.md`). So a log saying the turn is still running
///    is not disagreeing with a footer that has a question on it; it is saying
///    the only thing it CAN say about a turn that is, in fact, still open. If
///    this order is ever inverted a permission prompt stops being reported, and
///    "this agent needs you" is the one sentence this product exists to say.
/// 2. **A question the log says is outstanding, also `Blocked`.** The narrow
///    exception to the rung above, and it is not a weakening of it. An
///    `AskUserQuestion` whose result has not arrived is the agent naming, in
///    its own file, a tool that no machine can answer — see `Ask`. It sits
///    below the screen because the two never disagree in practice and the
///    screen carries the better sentence when it has one, and ABOVE the turn
///    rung because that is the bug this closes: the log says the turn is still
///    running, which was read as `Working`, so a claude sitting on a question
///    for 21 hours reported "working" and then "finished" — never once "needs
///    you". A row on this rung is blocked no matter how long the file has been
///    quiet, because quiet is exactly what waiting looks like.
/// 3. **The log.** It is the agent's own account of its own turn, written by
///    the agent for itself, and it does not care whether the footer was
///    mid-redraw when `capture-pane` ran. `STALE_LOG_MS` bounds one half of
///    what it can say and not the other — see below.
/// 4. **The title**, then the **screen**, exactly as stage 1 left them — see
///    `promoted_by_title`. A pane with no log reaches this having been touched
///    by nothing above.
///
/// The staleness bound applies to a log that says the turn is still RUNNING,
/// and to nothing else. The two halves are not symmetric, because silence is
/// not symmetric evidence: a running agent keeps writing — a tool call, a
/// result, a step — so a file that has gone quiet is a running turn we have
/// stopped being able to see. Silence after the agent wrote down that the turn
/// was OVER is just silence, and the turn is still over. A stale "the turn
/// ended" never becomes less true.
///
/// Bounding both halves is what put the original bug back: thirty seconds after
/// a `turn_duration` folded a stale `esc to interrupt` into `Done`, the log fell
/// out of trust, that same unredrawn footer had the last word again, and
/// `advance(Done, Working)` flipped the row back to Working with a fresh turn
/// clock — where it stayed, forever. "Stuck on Working" and a timer reading
/// hours, arriving through the door this stage was built to close.
///
/// What replaces the bound for an ended turn is `join_looks_dead`: the way a
/// finished turn legitimately stops being the truth is that a NEW session
/// started, and the answer to that is to re-join to the file the agent is
/// actually writing now — not to disbelieve the file we have and guess from a
/// screen that has already been shown to be stale.
///
/// `Idle` rather than `Done` for a finished turn, because `Done` is not a thing
/// anything observes: it is what `activity::advance` makes of a `Working` that
/// went `Idle`, and it is created and destroyed by that fold alone. Returning it
/// from here would fold it twice — see `agent_observation` for what that costs.
///
/// Eight arguments, which is one past clippy's taste and is the shape of the
/// thing: each one is a rung, the doc above is about the ORDER they are
/// consulted in, and bundling four of them into a struct would hide exactly
/// what a reader comes here to check.
#[allow(clippy::too_many_arguments)]
fn resolved_activity(
    screen: AgentActivity,
    log: Option<LogTurn>,
    asked: bool,
    now: i64,
    title: &str,
    command: &str,
    hostname: &str,
    title_repeats: u8,
) -> AgentActivity {
    if screen == AgentActivity::Blocked {
        return AgentActivity::Blocked;
    }
    // No staleness bound, deliberately, and it is the one rung that must not
    // have one. `STALE_LOG_MS` exists because a running agent keeps writing, so
    // a quiet file is a turn we have stopped being able to see — but an agent
    // holding for an answer writes NOTHING until it gets one, by construction.
    // A bound here would expire the state at the exact moment it became true,
    // and the median wait is 101 seconds against a 30-second bound.
    if asked {
        return AgentActivity::Blocked;
    }
    if let Some(turn) = log {
        if !turn.running {
            // Only while the screen still shows an agent at all.
            //
            // A finished turn stays finished, which is why this half carries no
            // staleness bound — but "the turn ended" is a claim about an agent,
            // and it cannot be the thing that makes a pane one. After someone
            // quits claude the pane is a shell: the log still holds
            // `running: false` and the tail is never dropped (`join_looks_dead`
            // returns false the moment `working` is false), so this returned
            // `Idle` forever. `Idle` is not `None` downstream — `rung_subject`
            // only falls back to the command for `None`, and every client reads
            // `activity != none` as "this is an agent" — so a plain shell went
            // on being counted, summarized and rendered as a live agent until
            // the terminal was removed.
            //
            // Deliberately not applied to the RUNNING half above. There the
            // asymmetry runs the other way: a log saying a turn is open is
            // evidence about a pane whose screen may simply not have redrawn,
            // and refusing it would put back the stuck-on-Working bug this
            // stage exists to close.
            if screen != AgentActivity::None {
                return AgentActivity::Idle;
            }
        } else if now.saturating_sub(turn.last_event_at) < STALE_LOG_MS {
            return AgentActivity::Working;
        }
    }
    promoted_by_title(screen, title, command, hostname, title_repeats)
}

/// Whether an unresolved pane is worth looking a session log up for on this
/// tick.
///
/// Two conditions, and they are not the same one written twice. The interval is
/// a floor on cost — no pane is ever looked up more than once every
/// `LOG_JOIN_INTERVAL_MS` no matter how busy the filesystem is. Above that
/// floor, `churn` is the ordinary trigger and the backstop is what happens when
/// there is no watcher to trigger it. Pulled out for the same reason
/// `should_announce` and `promoted_by_title` are.
fn worth_a_join(attempted_at: i64, now: i64, churn: bool) -> bool {
    let since = now.saturating_sub(attempted_at);
    since >= LOG_JOIN_INTERVAL_MS && (churn || since >= LOG_JOIN_BACKSTOP_MS)
}

/// Whether the file a pane is ALREADY tailing has stopped being the one its
/// agent writes to.
///
/// The join is made once and cached because it is expensive — a `ps` and an
/// `lsof` for codex, a directory listing and a file read for the other two —
/// but a cached answer to "which file" is a standing claim about a target that
/// moves. `/clear` starts a new session uuid and therefore a NEW file; so does
/// restarting the agent in the same pane, or the harness itself. Every one of
/// them leaves the old path readable, unchanged, forever, and a pane still
/// pointed at it shows the previous session's last three steps and never learns
/// another thing.
///
/// The evidence is a CONTRADICTION rather than a timer: the pane's own screen
/// says this agent is working, and the file it is supposed to be writing has
/// said nothing for longer than a log is believed at all. A working agent
/// writes — a turn start, a tool call, a result — so silence from the file while
/// `esc to interrupt` is on the footer means the writing is going somewhere we
/// are not looking. `attempted_at` stands in for a log that has never spoken at
/// all, which is the same claim about the same file with less to go on.
///
/// Both halves are load-bearing, and each rules out a pane that must NOT be
/// looked up again:
///
/// - a healthy agent idle overnight is silent for hours, and its screen agrees;
///   nothing is contradicted, so it is never re-joined. That is the case this
///   would otherwise burn an `lsof` per pane per five seconds on, all night;
/// - `Blocked` deliberately does not count as working. No agent records that it
///   is waiting for a human, so a permission prompt left up over lunch is a pane
///   whose log is legitimately silent for exactly as long as the human is away.
///
/// The false positive it does accept is a turn spent inside one long tool call:
/// five minutes of `cargo build` writes nothing while the footer says otherwise
/// (`STALE_LOG_MS` names the same case). That costs one repeat lookup per
/// `LOG_JOIN_INTERVAL_MS` while the build runs, and the lookup returns the file
/// already open, which `PaneLog::adopt` keeps offset and all. That is the right
/// side to be wrong on: the other way round is a pane that is silently dead
/// until someone deletes the terminal.
fn join_looks_dead(turn: Option<LogTurn>, attempted_at: i64, now: i64, working: bool) -> bool {
    if !working {
        return false;
    }
    let quiet_since = turn.map_or(attempted_at, |turn| turn.last_event_at);
    now.saturating_sub(quiet_since) >= STALE_LOG_MS
}

/// Attach to a pane's session log if it has one, read whatever it appended, and
/// let go of it once it stops being the file the agent writes.
///
/// Returns the events read on THIS pass alongside the log, rather than folding
/// them in here: what they fold into — the transcript, the task list, the
/// spawned agents — lives on `Observed`, behind the state mutex, and this runs
/// on a blocking thread with no access to it.
///
/// Blocking from top to bottom — `lsof`, directory listings, file reads — so it
/// is called from `spawn_blocking` and takes and returns the `PaneLog` by value
/// rather than borrowing it across the hop.
///
/// `working` is what stage 1 alone makes of the pane, and it is here for
/// `join_looks_dead` rather than for anything this reads. `find` is the join
/// itself, taken as an argument for the same reason `log_join` keeps a
/// `find_session_log_under`: the real one answers out of `$HOME`, and a test
/// that has to move a live session between files cannot have one.
fn advance_log(
    mut log: PaneLog,
    pane: &PaneJoin,
    now: i64,
    churn: bool,
    working: bool,
    find: impl Fn(&PaneJoin) -> Option<std::path::PathBuf>,
) -> (PaneLog, Vec<TurnEvent>) {
    // Every `None` on the way down is an ordinary "not an agent, or not yet":
    // a plain shell has no preset, opencode has no parser, and a codex that has
    // not been given its first turn has not opened its rollout file. None of
    // them is a failure, and all of them leave the pane exactly as stage 1 left
    // it — but they are answered as one `Option` and handed to `adopt` whole,
    // so a pane that HAD a log and can no longer be joined lets go of it
    // instead of tailing a dead file forever.
    let joined = || {
        pane.preset
            .as_deref()
            .and_then(LogFormat::of)
            .and_then(|format| Some((find(pane)?, format)))
    };

    if log.tail.is_none() {
        if !worth_a_join(log.attempted_at, now, churn) {
            return (log, Vec::new());
        }
        log.attempted_at = now;
        log.adopt(joined());
    }

    let Some((tail, format)) = log.tail.as_mut() else { return (log, Vec::new()) };
    let format = *format;
    let events: Vec<TurnEvent> =
        tail.read_new_lines().iter().flat_map(|line| format.parse_line(line)).collect();
    log.turn = fold_log_events(log.turn, &events, now);
    log.asked = fold_asks(log.asked.take(), &events);

    // Asked AFTER the read, and never instead of it. A line arriving on this
    // tick is the strongest evidence there is that the file is still the pane's
    // own, so re-deriving the join first would let a pane be detached over a
    // silence that had already ended — and would spend the lookup to find that
    // out. An attached pane therefore always reads; what this can change is
    // only which file it reads NEXT tick.
    if join_looks_dead(log.turn, log.attempted_at, now, working)
        && worth_a_join(log.attempted_at, now, churn)
    {
        log.attempted_at = now;
        log.adopt(joined());
    }
    (log, events)
}

/// One task, as the lines that mentioned it left it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Task {
    /// The id claude refers to it by: `"1"`, `"8"`. `None` between the create
    /// that made the task and the result that numbers it — a gap of one line,
    /// during which the task still counts toward the total. See `Signals::task`.
    id: Option<String>,
    /// The agent's own present-tense phrase for it, when the create that
    /// carried it could be joined to this id.
    active_form: Option<String>,
    status: TaskStatus,
}

/// One agent this pane's agent spawned.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Spawn {
    /// The `tool_use` id of the `Agent` call. The spawn and its result name the
    /// same one, which is how two lines a minute apart are known to be about
    /// one subagent.
    id: String,
    description: String,
    running: bool,
}

/// Where the agent is: its own task list, the agents it spawned, and the last
/// thing it did.
///
/// Folded here rather than in the parser because no single line can state any
/// of it. `TaskCreate` carries a phrase and no id, `TaskUpdate` carries an id
/// and a status and no totals, and `parse_line` is one line at a time by
/// contract — so the tally is assembled across ticks, on this side of the
/// mutex, beside the turn state the watcher already folds.
///
/// The plan and the spawns are cleared at a turn boundary, both ends of it. A
/// plan belongs to the turn that wrote it: carrying one into the next turn
/// would report a row as `3/7` through work that has nothing to do with those
/// seven tasks. The action is cleared at the START of a turn only, and the
/// transcript at neither — see `saw` for the one, and `Observed::feed` for the
/// other.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Signals {
    tasks: Vec<Task>,
    /// The id of the task the agent last said it was ON. Not derived from
    /// `tasks` at read time: two tasks can sit at `in_progress` at once, and
    /// the one it moved to most recently is the one it is doing.
    active: Option<String>,
    spawns: Vec<Spawn>,
    /// The last tool call of THIS turn, as the verb and object it arrived
    /// as: `("bash", "cd acme && cargo test")`. The signal line's fallback
    /// for an agent with no task list, which is codex, cursor, and most
    /// claude sessions.
    ///
    /// Kept raw rather than as a rendered `Phrase`, because how it should
    /// read depends on something that is not known when it is folded: a turn
    /// still open says `Running cd …`, and the same call once the turn is
    /// over says `Ran cd …`. See `line`.
    action: Option<(String, String)>,
}

impl Signals {
    /// Fold one event in.
    fn saw(&mut self, event: &TurnEvent) {
        match event {
            TurnEvent::Did { verb, object } => {
                self.action = Some((verb.clone(), object.clone()));
            }
            TurnEvent::TaskState { id, active_form, status } => {
                self.task(id.as_deref(), active_form.as_deref(), *status);
            }
            TurnEvent::Subagent { id, description, running } => {
                self.spawn(id, description, *running);
            }
            // Both ends of a turn, not only the end. A turn that begins while a
            // previous plan is still on the row would otherwise show the old
            // count until the new list's first create.
            //
            // The action is cleared at the START only, and the asymmetry is
            // the point. Driven live, a new turn wore the PREVIOUS turn's
            // action — `Running test "$(< fruit.txt)" = banana` — for the
            // first nine seconds of its life, until its own first tool call
            // replaced it; nine seconds is most of a short turn, and a row
            // confidently naming work that finished before the question was
            // asked is worse than a row saying nothing. But a turn ENDING does
            // not make that line untrue: a plan is where the agent IS, and a
            // finished turn has no position, while `Writing haiku.txt` is what
            // it DID — which is the one thing a finished row is read for. It
            // stays for the same reason the transcript stays.
            TurnEvent::Started { .. } => {
                self.forget_the_turn();
                self.action = None;
            }
            TurnEvent::Ended { .. } => self.forget_the_turn(),
            // Folded by `fold_asks`, into the pane's log rather than into its
            // signals: what a question changes is whether the row is BLOCKED,
            // which is decided before this fold runs and on the other side of
            // the state mutex. The action line for a question is already
            // carried by the `Did` that arrived with it — `Asking · End state`.
            TurnEvent::Asked { .. }
            | TurnEvent::Answered { .. }
            | TurnEvent::Said { .. }
            | TurnEvent::Title(_)
            | TurnEvent::BackgroundAgents(_) => {}
        }
    }

    /// Forget where the agent was, which a turn boundary makes untrue at
    /// either end of it. What it last DID is not in here — see `saw`.
    fn forget_the_turn(&mut self) {
        self.tasks.clear();
        self.active = None;
        self.spawns.clear();
    }

    /// One task fact, joined to the task it is about.
    ///
    /// Three shapes arrive and each joins differently — see `TurnEvent::
    /// TaskState`, whose table this implements:
    ///
    /// * a **create** carries the phrase and no id. It becomes a task
    ///   immediately, unnumbered: it is already part of the total, and a row
    ///   reading `0/6` for the second between a create and its result would be
    ///   a progress indicator that stutters every time a list is written.
    /// * a **create's result** carries the id and nothing else, one line later,
    ///   and numbers the oldest task still waiting for one.
    /// * an **update** (or a listed task) carries an id and a status, and finds
    ///   the task that id belongs to.
    ///
    /// The plan proposed joining a create to an id by CREATION ORDER instead —
    /// the k-th create is id `"k"` — and that is wrong in a way a corpus count
    /// could not show: ids are numbered per SESSION, so the second turn's first
    /// create is id `"8"`. Driven live, a seven-task list read `3/11` with no
    /// phrase at all. See `claude::created_task_id`.
    ///
    /// What is left to go wrong is an update naming an id nothing here has,
    /// which means a create or a result was never seen — a pane that attached
    /// to its log mid-turn. Then the ORDER that would say which task it is, is
    /// exactly what went missing, so an unnumbered task is claimed and its
    /// phrase DROPPED rather than guessed: the count stays right, and a row
    /// reading `3/7` with no phrase is most of the value. A phrase on the wrong
    /// task would be a row confidently saying the wrong thing.
    fn task(&mut self, id: Option<&str>, active_form: Option<&str>, status: Option<TaskStatus>) {
        let Some(id) = id else {
            self.tasks.push(Task {
                id: None,
                active_form: active_form.map(str::to_string),
                status: TaskStatus::Pending,
            });
            return;
        };
        let at = match self.tasks.iter().position(|task| task.id.as_deref() == Some(id)) {
            Some(at) => at,
            None => match self.tasks.iter().position(|task| task.id.is_none()) {
                Some(at) => {
                    self.tasks[at].id = Some(id.to_string());
                    // A create's result numbers the create that is waiting, and
                    // keeps its phrase. Anything else reaching an unnumbered
                    // task is the join having broken — drop the phrase.
                    if status.is_some() {
                        self.tasks[at].active_form = None;
                    }
                    at
                }
                None => {
                    self.tasks.push(Task {
                        id: Some(id.to_string()),
                        active_form: None,
                        status: TaskStatus::Pending,
                    });
                    self.tasks.len() - 1
                }
            },
        };
        // A line that named no status it recognized leaves the task where it
        // was: a progress count that moved for a reason nobody can name is
        // worse than one that held still.
        let Some(status) = status else { return };
        self.tasks[at].status = status;
        match status {
            TaskStatus::InProgress => self.active = Some(id.to_string()),
            // The task the agent was on has stopped being the task it is on,
            // and nothing has said what replaced it.
            _ if self.active.as_deref() == Some(id) => self.active = None,
            _ => {}
        }
    }

    /// One subagent fact, joined to the spawn it is about.
    fn spawn(&mut self, id: &str, description: &str, running: bool) {
        if let Some(spawn) = self.spawns.iter_mut().find(|spawn| spawn.id == id) {
            // Only ever an upgrade. A `completed` result carries no
            // description, and overwriting the name from the spawn with an
            // empty string would un-name an agent for the sake of a field that
            // was never there.
            if !description.is_empty() {
                spawn.description = description.to_string();
            }
            spawn.running = running;
            return;
        }
        self.spawns.push(Spawn { id: id.to_string(), description: description.to_string(), running });
    }

    /// Where the agent is in its list, or `None` when it has not written one.
    ///
    /// `deleted` tasks count for neither half. `TaskUpdate` has that status and
    /// uses it, so a total that only ever grew would report `7/6` the first
    /// time an agent dropped a task it had decided against.
    fn plan(&self) -> Option<farcooler_core::feed::Plan> {
        let live = || self.tasks.iter().filter(|task| task.status != TaskStatus::Deleted);
        let total = live().count() as u32;
        if total == 0 {
            return None;
        }
        let done = live().filter(|task| task.status == TaskStatus::Completed).count() as u32;
        let active_form = self
            .active
            .as_ref()
            .and_then(|id| self.tasks.iter().find(|task| task.id.as_ref() == Some(id)))
            .and_then(|task| task.active_form.as_deref())
            .map(farcooler_core::feed::Phrase::new);
        Some(farcooler_core::feed::Plan { done, total, active_form })
    }

    /// That same position as the two NUMBERS it is, for the wire.
    ///
    /// `line` renders this into `3/7 · Designing test matrix · 2 agents` and
    /// then it is prose, so every client that wanted the fraction back would
    /// have to parse a composed string — and get it wrong the moment the agent
    /// is blocked, because then the line is the question and the numbers are
    /// not in it at all. The ladder is decided once here precisely so clients
    /// cannot disagree; a client re-deriving `3/7` out of the string this
    /// module composed would be a second derivation of one fact.
    ///
    /// Off the SAME `plan()` the line is composed from, deliberately. Two
    /// folds of `self.tasks` would be two answers waiting to differ; one call
    /// each from `Watcher::signal` and `Watcher::plan` is already two reads of
    /// the state mutex a tick apart, which is why `wire::apply_rungs` takes the
    /// string and the numbers together.
    ///
    /// The phrase is dropped, and only the phrase. It is already on `line`,
    /// where it has a row to sit in; these two are for a surface with room for
    /// a bar and a fraction and nothing else.
    fn plan_counts(&self) -> Option<(u32, u32)> {
        self.plan().map(|plan| (plan.done, plan.total))
    }

    /// The agents still running, named — one line each on a surface with room.
    ///
    /// Only `completed` ends one. A third of the spawns on this machine come
    /// back within seconds carrying `async_launched` while the agent runs for
    /// another hour, and reading those as finished would empty this list the
    /// instant it filled — see `claude::finished_subagent`.
    ///
    /// An unnamed running agent is counted by `running` and skipped here: a
    /// blank line under a row says nothing and costs a line.
    fn subagents(&self) -> Vec<String> {
        self.spawns
            .iter()
            .filter(|spawn| spawn.running && !spawn.description.is_empty())
            .map(|spawn| farcooler_core::feed::Phrase::new(&spawn.description).as_str().to_string())
            .collect()
    }

    /// How many are still running, named or not.
    fn running(&self) -> usize {
        self.spawns.iter().filter(|spawn| spawn.running).count()
    }

    /// The signal line: where this agent is, in one line.
    ///
    /// Composed by `farcooler_core::feed::signal` rather than here, so the
    /// daemon that folds the facts and the rung that renders them cannot come
    /// to two different answers about one pane.
    /// `mid_turn` decides the tense of the action, and comes from the pane's
    /// RESOLVED activity rather than from anything folded in here. A turn
    /// boundary seen in the log is not the same question: a tail that attached
    /// after the turn had already started never saw its `Started`, and a
    /// daemon restarted mid-turn never saw one either — both would render a
    /// working agent in the past tense. What the pane is doing now is decided
    /// from three sources in `resolved_activity`, and this is that answer.
    fn line(&self, mid_turn: bool) -> Option<String> {
        // Rendered here rather than at fold time, because the tense is not
        // knowable when the tool call arrives. See `action`.
        let action = self.action.as_ref().map(|(verb, object)| {
            if mid_turn {
                farcooler_core::feed::Phrase::action(verb, object)
            } else {
                farcooler_core::feed::Phrase::action_done(verb, object)
            }
        });
        farcooler_core::feed::signal(self.plan().as_ref(), self.running(), action.as_ref())
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
            last_card_push: None,
            turn_failed: false,
            feed: farcooler_core::feed::Feed::default(),
            signals: Signals::default(),
            pending: None,
            // Nothing has been said about this terminal's card yet, whether it
            // is a pane that opened a second ago or one this daemon has just
            // inherited from its own previous life. See `card_settled`.
            card_settled: false,
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
    /// Five things can make a terminal newsworthy, and the last two are the
    /// ones easy to miss. An agent can replace its question without any of the
    /// first three moving — activity stays Blocked, the pane state stays
    /// Running, the command is unchanged — and a row that keeps answering the
    /// PREVIOUS question is worse than one that says only "Needs you". A step
    /// is the same shape of news one layer down: an agent that has been Working
    /// for ten minutes moves nothing else at all, and the whole point of the
    /// transcript is that such a row stops refusing to say what it is doing —
    /// and the signal line is the same news again: a task completing moves
    /// `3/7` to `4/7` while every other fact about the pane holds still.
    /// Pulled out of `sample()`'s loop body so a unit test can call the exact
    /// function the loop calls, rather than a copy of it that can drift out of
    /// sync with what the loop actually does.
    fn should_announce(
        &self,
        activity_moved: bool,
        state: TerminalState,
        command: &str,
        blocked_question: &Option<String>,
        signals_moved: bool,
    ) -> bool {
        activity_moved
            || self.state != state
            || self.command != command
            || self.blocked_question != *blocked_question
            || signals_moved
    }

    /// Fold this tick's log events into what a client would see, reporting
    /// whether any of it moved.
    ///
    /// Compared rather than counted, which the feed's own `!steps.is_empty()`
    /// was not: a batch can carry ten events and move nothing a person would
    /// notice — a `TaskList` result restating five statuses that were already
    /// right, a subagent coming back with the description it was spawned with —
    /// and an announce per log line wakes every connected client for a row that
    /// reads identically. The comparison is over exactly the strings that go on
    /// the wire, so "moved" means the row changed and nothing weaker.
    fn saw_events(&mut self, events: &[TurnEvent]) -> bool {
        if events.is_empty() {
            return false;
        }
        let before = self.rendered();
        for event in events {
            // Only prose reaches the transcript. Everything else — a tool call,
            // a task moving, an agent spawning — is where the agent IS, which
            // is the signal line's business and not history's.
            //
            // A conclusion enters the window from its start and narration from
            // its end; see `farcooler_core::feed::Feed::conclude` for why the
            // two are read differently.
            if let TurnEvent::Said { text, conclusion } = event {
                if *conclusion {
                    self.feed.conclude(text);
                } else {
                    self.feed.push(text);
                }
            }
            self.signals.saw(event);
        }
        self.rendered() != before
    }

    /// Whether this pane is inside a turn right now, which is what decides
    /// the tense of its action line. `Blocked` counts: an agent waiting on a
    /// permission prompt has not finished what it was doing, it has stopped
    /// partway through it.
    fn mid_turn(&self) -> bool {
        matches!(self.activity, AgentActivity::Working | AgentActivity::Blocked)
    }

    /// Everything a client renders out of this pane's session log.
    fn rendered(&self) -> (Vec<String>, Option<String>, Vec<String>) {
        (self.feed.lines(), self.signals.line(self.mid_turn()), self.signals.subagents())
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

/// Whether a live card on this terminal still has a run behind it.
///
/// `Some(false)` is the whole point: it is the daemon saying "whatever the relay
/// is showing for this terminal, there is nothing here for it to be about". Only
/// the relay knows which cards exist, so this is the half of the answer the
/// runner can supply — see `crate::push::retire`.
///
/// `None` is "could not tell", and it is not a weaker "no". A failed
/// `capture-pane` reads as `Unspecified`, and retiring a blocked agent's card on
/// one would take the single notification this product exists for off the lock
/// screen — and nothing would put it back, because `Blocked` to `Blocked` is not
/// a transition and pushes nothing. The same distinction `advance_to` already
/// makes when it refuses to hold a turn clock open across these.
///
/// The pane's own state is read FIRST and outranks the screen, because it is
/// evidence of a different kind. `Exited` and `Lost` are findings — the live
/// inventory was read and this terminal is not in it, or its pane is a retained
/// corpse — where the activity beside them is whatever a failed screen read made
/// of the same pane, which is nothing. `Unknown` is the case that used to be
/// reported as `Lost`: nobody got to ask, usually because one pane's blocked
/// write has the single-threaded tmux server stalled, and answering "no card" to
/// that would retire every card on the runner in one tick. See
/// `farcooler_core::derive::derive_terminal`, which is where that distinction is
/// made and why it exists.
fn card_still_earned(state: TerminalState, activity: AgentActivity) -> Option<bool> {
    match state {
        // The pane is gone or dead, and no reading of its screen can outvote
        // that. `Error` is a terminal whose runtime was never established at
        // all, which is the same answer arrived at earlier.
        TerminalState::Exited | TerminalState::Lost | TerminalState::Error => Some(false),
        // "We could not look, so we are not saying."
        TerminalState::Unknown => None,
        _ => match activity {
            // A turn is in progress. Blocked is part of one, not the end of it,
            // and its card is the one thing here that must never be taken down
            // on a guess.
            AgentActivity::Working | AgentActivity::Blocked => Some(true),
            // Idle is a pane between turns, `Done` is a turn that has ended, and
            // `None` is a pane that is not an agent at all — somebody quit
            // claude and got their shell back, which leaves a card reading
            // "Working" with no process left to finish. None of the three has a
            // run behind it for a card to be about.
            AgentActivity::Idle | AgentActivity::Done | AgentActivity::None => Some(false),
            // `Unspecified` is a screen that could not be read and `Unknown` is
            // one that matched nothing recognized. Neither is evidence.
            _ => None,
        },
    }
}

/// Whether THIS tick is the moment a live card became an orphan.
///
/// Split out of `sample`'s loop for the reason `exited_into_failure` and
/// `should_announce` are: the condition has to be the exact thing a test
/// exercises rather than a copy of it that can drift, and this one decides
/// whether a card comes off somebody's lock screen.
///
/// Three moments, and they are three because a card can be orphaned by three
/// different things going away. `!settled` is the daemon's own restart: it has
/// just met a terminal the relay may already be holding a card for, and being
/// met is not a transition anything can fire on. `state_moved` is the pane
/// dying. `activity_moved` is the agent inside a living pane going away —
/// somebody quits claude, and `Working` becomes `None` with no notice to send.
///
/// `Done` is excepted, and it is the one exception. It carries its own notice,
/// that notice is what ends the card, and it deliberately leaves a minute of
/// "Finished" up for whoever picks the phone up because of the alert beside it —
/// see `DISMISSAL_DELAY_S` in the relay. Retiring here as well would race that
/// end and win, and the person who came to look would find nothing there.
fn card_orphaned(
    settled: bool,
    state_moved: bool,
    activity_moved: Option<AgentActivity>,
    earned: Option<bool>,
) -> bool {
    if earned != Some(false) {
        return false;
    }
    if activity_moved == Some(AgentActivity::Done) {
        return false;
    }
    !settled || state_moved || activity_moved.is_some()
}

/// Whether a claim of attention made at `at` is still worth believing at `now`.
///
/// Its own function, tiny as it is, for the reason `should_refresh_card` is one:
/// this is the freshness bound, it is the only thing standing between a client
/// that crashed and a terminal that never notifies again, and a bound that is
/// spelled inline at its one call site is a bound nothing can test. See
/// `WATCHED_TTL_MS`.
///
/// A claim dated in the FUTURE is expired, not believed. `now_millis` reads the
/// wall clock, so an NTP correction or a laptop resuming can move it backwards
/// under a claim already recorded — and a bare `age < TTL` reads a negative age
/// as freshly watched, which is a terminal silenced for as long as the jump was
/// large. Erring towards expiry is erring towards sending the notification,
/// which is the side of this to be wrong on: the mistake is then one buzz too
/// many, and the other way it is every buzz missing.
fn watch_is_live(at: i64, now: i64) -> bool {
    let age = now.saturating_sub(at);
    (0..WATCHED_TTL_MS).contains(&age)
}

/// Write down what one client says it is showing, replacing what it said last.
///
/// Free rather than a method for the reason `card_orphaned` is free: the two
/// properties that matter here — a client's later word REPLACES its earlier one,
/// and a client that stops speaking stops counting — are the properties the
/// whole feature's safety rests on, and neither is testable through a `Watcher`
/// without standing up a `Service`, a SQLite handle and a tmux server to ask a
/// question about a `HashMap`.
///
/// Expired rows are dropped on the way past. There is no timer here to sweep on
/// and no need for one: the map is bounded by the number of devices that have
/// ever connected, `anyone_watching` checks freshness itself so a lingering row
/// silences nothing, and clearing them here means a device revoked months ago
/// does not keep a slot forever.
fn record_watch(
    watched: &mut HashMap<String, Watched>,
    client: &str,
    terminals: Vec<Uuid>,
    now: i64,
) {
    watched.retain(|_, w| watch_is_live(w.at, now));
    watched.insert(client.to_string(), Watched { at: now, terminals });
}

/// Whether ANY client's live claim covers this terminal.
///
/// Any, because there is one account and one person: a Mac with the pane on
/// screen speaks for the watch on their wrist as much as for itself, and that
/// is the complaint being fixed rather than a corner cut. A claim from a client
/// that has gone quiet is skipped rather than removed, so reading this can stay
/// on a shared reference and cannot be the thing that decides a device is gone.
fn anyone_watching(watched: &HashMap<String, Watched>, terminal: Uuid, now: i64) -> bool {
    watched.values().any(|w| watch_is_live(w.at, now) && w.terminals.contains(&terminal))
}

/// What a transition earns once it is known whether anyone is looking at it.
///
/// Three outcomes rather than a bool, because "do not alert" and "do nothing"
/// are not the same instruction on a phone. A live card is a rectangle the relay
/// keeps up until something ends it, and the ONLY thing that has ever ended one
/// for a finished agent is the `done` push — so suppressing that push without
/// saying anything else would leave a card reading "Working" over an agent that
/// stopped ten minutes ago. That is the exact bug `5398dba` was written to
/// close, reintroduced through a different door.
enum Attention {
    /// Send the notice its tier earns, alert and all. What every transition did
    /// before this existed, and what all but two of them still do.
    Announce,
    /// Say nothing, and take the live card down SILENTLY — `crate::push::retire`
    /// rather than `crate::push::notify`. No banner, no sound, no buzz on a
    /// wrist, and no card left over claiming a run that has ended.
    Retire,
    /// Say nothing at all, and leave the card exactly where it is.
    Hold,
}

/// Whether to interrupt the owner about a transition, and what to do instead.
///
/// The rule the whole feature reduces to: **you do not get told about an agent
/// you are demonstrably watching right now**. `watched` is the daemon's answer
/// to that — a client with a pane on screen and its app frontmost, saying so
/// within `WATCHED_TTL_MS` — and it is per TERMINAL and across every device,
/// because it is one person: a Mac focused on the pane silences the wrist beside
/// it, which is precisely the complaint. Everything else is unchanged, and
/// "everything else" is deliberately most of it: another pane finishing, the
/// same pane finishing while the app is in the background, the window sitting
/// behind a browser, a phone in a pocket, a client that stopped saying anything
/// ten seconds ago.
///
/// `Done` retires rather than announces. The turn IS over, so the card has
/// nothing left behind it and must come down; what it must not do is buzz, since
/// the reply is already on the screen being read. `terminal.seen` will move the
/// row off `Done` on the client's next poll, as it always has — this changes
/// what the owner is TOLD, never what the runner believes.
///
/// "The card" is one card per app install rather than one per terminal, and this
/// instruction did not have to change for that. The runner names a terminal and
/// the relay answers about the card it holds: if this agent is the one the card
/// is LEADING with, the card comes down silently, exactly as intended; if it is
/// not, the card is about somebody else who is still running and is left alone,
/// which is also exactly as intended. Either way no `done` push goes out, so
/// nothing buzzes and nothing is left claiming a run that has ended.
///
/// `Blocked` raises nothing, and takes nothing down. Nothing, because you are
/// looking at the question — that is the whole request. And nothing DOWN because
/// a blocked agent is mid-turn: `card_still_earned` says a run is still behind
/// that card, and retiring it would take the one notification this product
/// exists for off a lock screen with nothing able to put it back, since `Blocked
/// -> Blocked` is not a transition and pushes nothing. The card holds its last
/// working line until the turn moves again — stale by a line, and honest about
/// there being a run in progress.
///
/// The cost of `Hold`, stated because it is real: look at a question, walk away
/// without answering it, and no alert will ever arrive for it. That is accepted
/// rather than overlooked. The alternative — remembering the withheld notice and
/// firing it once the claim expires — buzzes about a question you have already
/// read every time you switch to a browser for eleven seconds, which is a worse
/// version of the bug being fixed here.
///
/// `Working` never reaches this. It is pushed from `announce` rather than from
/// the transition site, it is silent by construction (the relay refuses to alert
/// on it), and a live card that goes on updating for a watched pane is right:
/// the phone in your pocket is not the screen you are looking at.
fn attention(next: AgentActivity, watched: bool) -> Attention {
    if !watched {
        return Attention::Announce;
    }
    match next {
        AgentActivity::Done => Attention::Retire,
        AgentActivity::Blocked => Attention::Hold,
        // Nothing else produces a notice at all — `notification` answers `None`
        // for every remaining tier — so this arm is reached only if that ever
        // changes. `Announce` is the safe default of the two: a redundant
        // notification is a nuisance, and a swallowed one is the product not
        // working.
        _ => Attention::Announce,
    }
}

impl Watcher {
    pub fn new(service: Arc<Service>) -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_BACKLOG);
        Arc::new(Self {
            service,
            events,
            state: tokio::sync::Mutex::new(HashMap::new()),
            worktree_marks: std::sync::Mutex::new(HashMap::new()),
            worktree_reconciles: std::sync::Mutex::new(HashMap::new()),
            change_set_probes: std::sync::Mutex::new(HashMap::new()),
            change_set_activity: std::sync::Mutex::new(HashMap::new()),
            trace_workers: std::sync::Mutex::new(HashMap::new()),
            traces: std::sync::Mutex::new(HashMap::new()),
            trace_shapes: std::sync::Mutex::new(HashMap::new()),
            trace_levels: std::sync::Mutex::new(HashMap::new()),
            commit_traces: std::sync::Mutex::new(HashMap::new()),
            trace_workspaces: std::sync::Mutex::new(HashMap::new()),
            change_set_probing: std::sync::atomic::AtomicBool::new(false),
            logs: std::sync::Mutex::new(HashMap::new()),
            log_watcher: crate::log_watch::LogWatcher::start(crate::log_watch::roots()),
            // Registers nothing here on purpose: the fleet is not known yet,
            // and on a per-tree backend every registration restarts the event
            // stream. The two passes that already enumerate the fleet do the
            // registering, off the daemon's startup path.
            fs_watcher: Arc::new(crate::fs_watch::TreeWatcher::start()),
            watched: std::sync::Mutex::new(HashMap::new()),
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
    /// Blocked and done BUZZ, and nothing else does. A working agent is the
    /// normal case, and something that buzzes for the normal case is something
    /// people turn off — after which it cannot tell them the thing that
    /// mattered. Working reaches here too now, from `refresh_card`, but only as
    /// a silent card update: the relay is what holds that line, and it will not
    /// build an alert out of a `working` notice. See `push_notice` for why
    /// sending is detached rather than awaited here.
    fn push_if_paired(
        &self,
        terminal: Uuid,
        activity: AgentActivity,
        label: &str,
        quoted: Quoted<'_>,
        turn_failed: bool,
        started_at: Option<i64>,
    ) {
        let Some(notice) = notification(activity, label, quoted, turn_failed, started_at) else {
            return;
        };
        self.push_notice(terminal, label, notice);
    }

    /// Refresh the live card of an agent that is still working, at most once
    /// every `CARD_REFRESH_MS`.
    ///
    /// The counterpart to `push_if_paired`, for the tier that does not change.
    /// A working agent moves its signal line several times a second while
    /// everything else about it holds still, and one push per movement is a
    /// budget spent on text that is gone before it can be read.
    ///
    /// Takes the state lock a second time rather than deciding under the
    /// sampling loop's, because the timestamp records a push that ACTUALLY went
    /// out: the line has to survive `apply_rungs` and the terminal has to still
    /// be in the fleet, neither of which is known where that lock is held.
    /// Stamping it earlier would silently swallow the next ten seconds of
    /// refreshes to pay for a card that was never sent.
    async fn refresh_card(&self, terminal: Uuid, label: &str, line: &str) {
        let now = now_millis();
        // Taken from the entry this call just found live, not from the record
        // `announce` is holding: it is the same lock that decided the push goes
        // out at all, so the clock on the card cannot be one turn older than the
        // decision to send it.
        let started_at;
        {
            let mut state = self.state.lock().await;
            let Some(observed) = state.get_mut(&terminal) else { return };
            if !should_refresh_card(observed.last_card_push, now) {
                return;
            }
            observed.last_card_push = Some(now);
            started_at = observed.turn_started_at;
        }
        // Outside the lock on principle rather than necessity — this spawns
        // rather than awaits — so that nothing about the push path can ever
        // become something the sampling loop waits on. See `push_notice`.
        self.push_if_paired(
            terminal,
            AgentActivity::Working,
            label,
            // No workspace and nothing said. A working notice is a card the
            // relay moves in place, whose one line is the composed signal rung
            // — see the `Working` arm of `notification` for why neither of the
            // other two belongs on it.
            Quoted { workspace: "", question: Some(line), said: None },
            false,
            started_at,
        );
    }

    /// Push a failed exit to the owner's devices, if this runner is paired.
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
        workspace: &str,
        exit_code: Option<i32>,
        exit_signal: Option<i32>,
    ) {
        self.push_notice(terminal, label, exit_notice(label, workspace, exit_code, exit_signal));
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
                crate::push::Outgoing {
                    title: &notice.title,
                    subtitle: &notice.subtitle,
                    status: notice.status,
                    failed: notice.failed,
                    label: &label,
                    terminal: &terminal.to_string(),
                    started_at: notice.started_at,
                },
            )
            .await;
        });
    }

    /// Ask the relay to take down cards this runner can no longer account for,
    /// detached from the sampling loop.
    ///
    /// Not async, and spawned, for the identical reason `push_notice` is not:
    /// this is called from the sampling loop, an HTTP round trip is not
    /// something that loop can afford to wait on, and a relay that is slow or
    /// unreachable must cost a card its retirement rather than cost the whole
    /// fleet its next sample. Reading the pairing file happens inside the
    /// spawned task, being blocking I/O.
    ///
    /// Silent on the phone, unlike everything else this module sends. A card
    /// coming down is not news — the person closed the pane themselves, or
    /// updated their runner — and the relay is what enforces that: see
    /// `/v1/notify/retire`, which pushes no alert and wakes nobody.
    fn retire_cards(&self, terminals: Vec<Uuid>) {
        if terminals.is_empty() {
            return;
        }
        let terminals: Vec<String> = terminals.iter().map(Uuid::to_string).collect();
        // Cheap: a `reqwest::Client` is a handle to a shared pool. Same clone,
        // same pool, same reason as `push_notice`.
        let client = self.push.clone();
        tokio::spawn(async move {
            let Some(pairing) = crate::push::Pairing::load() else { return };
            crate::push::retire(&client, &pairing, &terminals).await;
        });
    }

    /// Record what one client is showing a person right now.
    ///
    /// The whole set, replacing whatever this client last said, so a client that
    /// navigated away releases the pane it left in the same call that claims the
    /// one it arrived at — and an empty set, which is what both apps send as they
    /// go to the background, releases everything they held at once. See
    /// `Watcher::watched` for why this is keyed by client and not by terminal.
    ///
    /// Synchronous and cheap on purpose. This is called several times a minute
    /// by every connected client with a pane on screen, from a dispatcher that
    /// has a person waiting on the round trip; it takes a std mutex over a map
    /// with one entry per device and does nothing else.
    ///
    /// Stale entries are dropped on every call rather than swept on a timer.
    /// There is no timer to hang a sweep on that would not also be a timer this
    /// module does not otherwise need, and the map is bounded either way by the
    /// number of enrolled devices — but leaving expired rows in it would mean a
    /// device revoked six months ago still occupying a slot, and the entry that
    /// is expensive to keep is the one nobody will ever look at again.
    /// Expiry-on-write cannot resurrect anything: `is_watched` checks freshness
    /// itself, so a row that outlives its TTL between two calls silences
    /// nothing.
    pub fn report_watching(&self, client: &str, terminals: Vec<Uuid>) {
        record_watch(&mut self.watched.lock().unwrap(), client, terminals, now_millis());
    }

    /// Whether anybody is looking at this terminal, as of `now`.
    ///
    /// Any client's live claim is enough, and no client's identity is consulted
    /// beyond having one: there is one account and one person here, so a Mac
    /// with the pane on screen speaks for the wrist as much as for itself. That
    /// is the point rather than a simplification — the complaint being fixed is
    /// a notification arriving on a watch about a pane being read on a Mac.
    ///
    /// `now` is passed in rather than read here so the decision about a terminal
    /// uses the same clock as everything else the sampling loop decided about it
    /// in that tick, and so the rule is testable without waiting out ten real
    /// seconds.
    fn is_watched(&self, terminal: Uuid, now: i64) -> bool {
        anyone_watching(&self.watched.lock().unwrap(), terminal, now)
    }

    /// Subscribe a connection to the push stream.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// What the watcher last decided, with both clocks.
    /// Add to one terminal's trace, in the bucket `now` belongs to.
    ///
    /// `now` is seconds, not the milliseconds everything else in this loop
    /// carries: bucket edges are wall-clock seconds and the conversion belongs
    /// at the one boundary rather than inside `farcooler_core::trace`.
    pub fn record_trace(
        &self,
        terminal: Uuid,
        now_secs: i64,
        sample: farcooler_core::trace::Sample,
    ) {
        self.traces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(terminal)
            .or_default()
            .record(now_secs, sample);
    }

    /// Move every live terminal's ring forward and drop the ones that are gone.
    ///
    /// Advancing matters even for a terminal that did nothing: a ring that is
    /// not advanced has not cleared the slots a gap skipped, and the next read
    /// would find yesterday's counts sitting in today's buckets. See
    /// `Trace::advance_to`.
    pub fn tick_traces(&self, live: &HashMap<Uuid, Uuid>, now_secs: i64) {
        let mut traces = self.traces.lock().unwrap_or_else(|e| e.into_inner());
        traces.retain(|id, _| live.contains_key(id));
        for id in live.keys() {
            traces.entry(*id).or_default().tick(now_secs);
        }
        drop(traces);

        let workspaces: HashSet<Uuid> = live.values().copied().collect();
        let mut commits = self.commit_traces.lock().unwrap_or_else(|e| e.into_inner());
        commits.retain(|id, _| workspaces.contains(id));
        for id in &workspaces {
            commits.entry(*id).or_default().tick(now_secs);
        }
        drop(commits);

        self.trace_shapes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|id, _| live.contains_key(id));
        *self.trace_workspaces.lock().unwrap_or_else(|e| e.into_inner()) = live.clone();
    }

    /// One terminal's trace, encoded as the wire carries it.
    ///
    /// Its own ring for the two bars, plus its worktree's ring for the commit
    /// marks on the axis. Read by BOTH finishers — `rpc::with_activity` on the
    /// poll path and `announce` on the push path — so a client that polls and a
    /// client that is pushed to never hold different histories for the same
    /// terminal. That is the rule every other field in `wire::terminal`
    /// follows, and the reason this is a method here rather than a computation
    /// at either call site.
    pub fn trace(&self, terminal: Uuid) -> Vec<u8> {
        let now = now_millis() / 1000;
        let Some(mut trace) = self
            .traces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&terminal)
            .cloned()
        else {
            return Vec::new();
        };
        let workspace =
            self.trace_workspaces.lock().unwrap_or_else(|e| e.into_inner()).get(&terminal).copied();
        if let Some(workspace) = workspace {
            if let Some(marks) =
                self.commit_traces.lock().unwrap_or_else(|e| e.into_inner()).get(&workspace)
            {
                trace.absorb(marks, now);
            }
        }
        trace.encode(now)
    }

    /// Every terminal's trace added together, at one width for the whole fleet.
    ///
    /// Summed here rather than on the phone because the rows do not share a
    /// width — each snaps to the shortest window holding its own activity — so
    /// adding bucket 4 of a five-minute row to bucket 4 of a two-hour row would
    /// add two different spans of time. The daemon holds every ring and can
    /// pick one width across all of them; a client holding only the rendered
    /// rows cannot.
    ///
    /// Commits come off the workspace rings and so are added once each, not
    /// once per pane sharing the worktree. See `commit_traces`.
    pub fn fleet_trace(&self) -> Vec<u8> {
        let now = now_millis() / 1000;
        let mut fleet = farcooler_core::trace::Trace::new();
        let mut any = false;
        for one in self.traces.lock().unwrap_or_else(|e| e.into_inner()).values() {
            fleet.absorb(one, now);
            any = true;
        }
        for marks in self.commit_traces.lock().unwrap_or_else(|e| e.into_inner()).values() {
            fleet.absorb(marks, now);
            any = true;
        }
        if !any {
            return Vec::new();
        }
        fleet.encode(now)
    }

    /// The upper half and the axis, for one workspace that was just probed.
    ///
    /// Called from `probe_change_sets`, which is already behind the gate that
    /// decides a worktree is worth looking at, and which has just refreshed the
    /// counts this reads. **Nothing here runs git.** `ReviewCache::counts` is a
    /// map lookup by contract, and the commit marks are a read of a file git
    /// appends to — so the trace costs one `stat`, and one short read on the
    /// passes where HEAD actually moved.
    ///
    /// # What the upper half really is, said plainly
    ///
    /// It is the GROWTH of `insertions + deletions` for the whole worktree
    /// against its base, split evenly among the panes this loop saw working
    /// while it grew. Three things follow, and none of them is a bug:
    ///
    /// - **Attribution is an even split, not a measurement.** Several terminals
    ///   routinely share a worktree — the same objection `FleetSnapshot.swift`
    ///   already records for `reviewsWaiting` — and neither git nor the daemon
    ///   can say which pane wrote a line. Where one pane was working it is
    ///   exact; where three were, each row gets a third of the truth.
    /// - **Churn is lost.** A line written and then written back shows as
    ///   growth of zero, because the diff against the base ends where it
    ///   started. The trace under-reports a busy agent that rewrites its own
    ///   work, and never over-reports.
    /// - **Nothing is attributed when nobody was seen working.** A person
    ///   editing in their own editor moves the diff with no pane to put it on,
    ///   and it is dropped rather than shared out among whoever happens to be
    ///   in that worktree.
    fn record_workspace_trace(&self, workspace: Uuid, worktree: &std::path::Path, now_secs: i64) {
        let workers: Vec<Uuid> = self
            .trace_workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&workspace)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();

        let mut levels = self.trace_levels.lock().unwrap_or_else(|e| e.into_inner());
        let level = levels.entry(workspace).or_default();

        if let crate::review::Counts::Known(_, insertions, deletions) =
            self.service.review_cache.counts(workspace)
        {
            let total = insertions.saturating_add(deletions);
            let each = code_share(level.lines, total, workers.len());
            level.lines = Some(total);
            if each > 0 {
                for terminal in &workers {
                    self.traces
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .entry(*terminal)
                        .or_default()
                        .record(now_secs, farcooler_core::trace::Sample::code(each));
                }
            }
        }

        // The commit marks. One `stat`, and a read only of what is new.
        let log = crate::review::git_dir(worktree).join("logs/HEAD");
        let Ok(meta) = std::fs::metadata(&log) else {
            // No reflog: either the file has never been written or this
            // repository has `core.logAllRefUpdates` off. Either way git kept
            // no record, and no mark is the honest reading of that.
            return;
        };
        let from = level.reflog.unwrap_or(0);
        if level.reflog.is_some() && meta.len() <= from {
            // Nothing appended. A file that SHRANK has been expired or
            // rewritten by `git reflog expire` or a gc; re-reading it whole
            // would count every surviving commit a second time, so the offset
            // is simply moved and this pass records nothing.
            level.reflog = Some(meta.len());
            return;
        }

        // The lock is held across this read, and the read is why. The offset
        // stored below has to be the length of the text that was actually
        // parsed, not the length `stat` reported a moment earlier — git may
        // have appended between the two, and an offset short of what was
        // counted makes the next pass count those commits a second time. A
        // reflog is a few tens of kilobytes and nothing else touches this map,
        // so holding it is cheaper than the correction would be.
        let Ok(text) = std::fs::read_to_string(&log) else { return };
        level.reflog = Some(text.len() as u64);
        // On the first pass `from` is 0 and the whole file is read, which is
        // the backfill described on `TraceLevel::reflog`. After that only the
        // tail is parsed, because everything before the offset has been
        // counted once already.
        // `get` rather than an index, and that is not defensiveness. The offset
        // is a byte count, a commit message is UTF-8, and slicing a string
        // anywhere but a character boundary PANICS. The file is append-only so
        // a stored offset stays a boundary — until the day something rewrites
        // it, and a panic in the sampling loop takes the whole daemon's fleet
        // with it. Falling back to the whole file counts a few marks twice,
        // once, which is a bar one pixel taller than it should be.
        let fresh = text.get(from as usize..).unwrap_or(text.as_str());
        let commits = crate::change_set::reflog_commits(fresh);
        drop(levels);
        if commits.is_empty() {
            return;
        }
        let mut traces = self.commit_traces.lock().unwrap_or_else(|e| e.into_inner());
        let ring = traces.entry(workspace).or_default();
        for at in commits {
            // Recorded at the second the commit was authored, not at the second
            // this pass noticed it. That is what makes the backfill land in the
            // buckets the work actually happened in.
            ring.record(at, farcooler_core::trace::Sample::commits(1));
        }
        // And forward to now, so a backfill of yesterday's reflog does not
        // leave the ring believing yesterday is the present.
        ring.tick(now_secs);
    }

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

    /// The last few things this terminal's agent did, as rows.
    ///
    /// Empty for a pane with no session log, which is an honest answer rather
    /// than a missing one: a plain shell has nothing to report.
    pub async fn feed(&self, terminal: Uuid) -> Vec<String> {
        self.state.lock().await.get(&terminal).map(|o| o.feed.lines()).unwrap_or_default()
    }

    /// The last thing this terminal's agent said, from its OPENING.
    ///
    /// What a notification quotes, and deliberately NOT the last entry of
    /// `feed` above: those are wrapped rows, so the last of them is the end of
    /// the window rather than the start of a sentence. See
    /// `farcooler_core::feed::Feed::said`.
    ///
    /// `None` for a pane with no session log and for an agent that has said
    /// nothing, which are both honest answers rather than missing ones — and
    /// distinct from `Some("")`, which this can never return.
    pub async fn said(&self, terminal: Uuid) -> Option<String> {
        self.state.lock().await.get(&terminal).and_then(|o| o.feed.said().map(str::to_string))
    }

    /// Where this terminal's agent is, in one line: its position in its own
    /// task list, or what it is doing right now.
    ///
    /// `None` for a pane with no session log and for one whose agent has done
    /// nothing yet — the ladder falls back to the headline rather than to a
    /// blank row. The question a blocked agent is asking is NOT here: it
    /// outranks this, and `farcooler_core::feed::line` is where that priority
    /// is applied, because only the rungs know the agent's state.
    pub async fn signal(&self, terminal: Uuid) -> Option<String> {
        self.state.lock().await.get(&terminal).and_then(|o| o.signals.line(o.mid_turn()))
    }

    /// How far this terminal's agent is through its own task list, as
    /// `(done, total)`.
    ///
    /// `None` for a pane with no session log, for an agent that has not written
    /// a list, and for an agent whose log format records none — which today is
    /// codex and cursor; see `Signals::plan_counts` and the fields' comments in
    /// `farcooler.proto`. All four are the same honest answer: nobody has said,
    /// so no surface draws a bar.
    ///
    /// A separate accessor rather than a second return value on `signal`
    /// because every fact the two builders of a live `Terminal` ask this type
    /// for is one accessor — `feed`, `said`, `subagents`, `turn_failed` — and
    /// one exception would be the one somebody has to remember. What must not
    /// be separate is where they are SET: `wire::apply_rungs` takes the line
    /// and these counts in one call for exactly that reason.
    pub async fn plan(&self, terminal: Uuid) -> Option<(u32, u32)> {
        self.state.lock().await.get(&terminal).and_then(|o| o.signals.plan_counts())
    }

    /// The agents this terminal's agent spawned and has not finished with,
    /// named.
    ///
    /// Empty for a pane with no session log, which is an honest answer rather
    /// than a missing one: nothing has spawned anything.
    pub async fn subagents(&self, terminal: Uuid) -> Vec<String> {
        self.state.lock().await.get(&terminal).map(|o| o.signals.subagents()).unwrap_or_default()
    }

    /// Whether the turn this terminal's agent just finished, failed.
    ///
    /// False for a pane with no session log, which is the honest answer rather
    /// than a missing one: nothing has claimed the turn went badly.
    pub async fn turn_failed(&self, terminal: Uuid) -> bool {
        self.state.lock().await.get(&terminal).is_some_and(|o| o.turn_failed)
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
    /// fleet without touching git at all, so neither the watch on
    /// `<common>/worktrees` nor the mtime gate behind it ever fires for them,
    /// and other connected clients would otherwise learn about them only at
    /// that repository's next `RECONCILE_BACKSTOP_MS` pass.
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

    /// A repository's stack was re-read.
    ///
    /// Carries the whole list, unlike `announce_change_set` above and like
    /// `publish_layout`. The reason the change set does not is that it is
    /// unbounded — a lockfile regeneration fans thousands of file records out to
    /// every connected device — and a stack is not: it is one link per branch in
    /// somebody's chain, with a PR status each. There is nothing here worth a
    /// second round trip, and the client waiting for it is holding a row it
    /// could not fill.
    ///
    /// Emitted from `review_ops::fill_prs_in_background`, which is what a
    /// `stack.get` against a repository nobody has read GitHub for kicks off.
    /// Without this the fill lands in a cache nobody re-reads, and filling on
    /// first read does nothing at all.
    pub fn announce_stack_changed(&self, list: farcooler_protocol::v1::StackLinkList) {
        let _ = self.events.send(Event {
            event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
            sequence: 0,
            payload: Some(farcooler_protocol::v1::event::Payload::StackChanged(list)),
        });
    }

    /// Start a change-set pass, unless one is still going.
    ///
    /// Detached rather than awaited inside the tick loop, because this is the
    /// one pass there that can wait on something the loop must never wait on:
    /// resolving a base falls through to `gh repo view` for a repository nobody
    /// has read a default branch for, bounded at `stack::GH_TIMEOUT` — fifteen
    /// seconds, once per repository, and every one of them on the first pass
    /// after a daemon start. Awaiting that here would hold `sample()` off for as
    /// long as it took, and a fleet whose rows stop moving is a worse failure
    /// than counts that arrive late. A worktree on a stalled mount buys the same
    /// stall out of `GIT_TIMEOUT`.
    ///
    /// Registering the worktree watches is now on the same list, and lands on
    /// the same first pass: a `git ls-files` per worktree, and a `watch` call
    /// per directory on the backends that charge per directory.
    ///
    /// A flag rather than a queue. If a pass is still running, the next tick has
    /// nothing to add: it would gate on exactly the same facts and probe exactly
    /// the same worktrees, and two of them racing would spend the git twice.
    fn spawn_change_set_probe(self: &Arc<Self>) {
        use std::sync::atomic::Ordering;
        if self.change_set_probing.swap(true, Ordering::SeqCst) {
            return;
        }
        let watcher = Arc::clone(self);
        tokio::spawn(async move {
            watcher.probe_change_sets().await;
            watcher.change_set_probing.store(false, Ordering::SeqCst);
        });
    }

    /// Re-derive the sidebar's `+N -M` for every worktree that could have moved.
    ///
    /// The expensive half of the review surface — three git processes per
    /// worktree, a `merge-base`, a `diff --shortstat` and a
    /// `status --porcelain=v2` — and what makes it affordable is everything this
    /// refuses to do before spending them. A worktree is probed only when one of
    /// these says it might have changed, and none of them costs a git process:
    ///
    /// - **The filesystem said so.** `fs_watch` was told that something under
    ///   the worktree, or under the `.git` that worktree reads `HEAD` and
    ///   `index` out of, was written. A drained set lookup. This is the primary
    ///   signal and it is the one that spends nothing at all when nothing
    ///   happens: an idle runner produces no events, so it makes no lookups
    ///   worth the name and asks the kernel for nothing.
    /// - **The cache was marked stale by something this daemon did itself** — a
    ///   base change, or a change set recomputed for a client. Not a write to
    ///   any file, so no watcher can see it. A map lookup.
    /// - **An agent has been active in one of its panes since the last probe** —
    ///   observed working, or its own session log advanced. A map lookup, off an
    ///   observation `sample()` already made this second, so it is free whether
    ///   or not anything else is watching.
    ///
    /// The first is what closes the gap this pass used to name out loud: an edit
    /// made in a worktree with NO pane open and NO diff on screen, by something
    /// this daemon never served, moved none of the other two and was not noticed
    /// until the next commit or the next time anyone worked or looked there. An
    /// agent editing a file writes only that file, so `.git` never moves; a
    /// commit writes only into `.git`, so the working tree is byte-identical
    /// either side of it. The watcher sees both, which is why it is registered
    /// on both.
    ///
    /// **And it keeps the property this module is built around, and makes it
    /// stronger.** A fleet where nothing is happening passes none of these, so
    /// it spawns no git at all however many worktrees it holds — and now it also
    /// makes no `stat` and no `git worktree list`, because the two polls that
    /// used to run every tick regardless are gone. `review::cheap_gate`'s two
    /// `stat`s per worktree are still HERE, as the fallback below, and they run
    /// only for a worktree the watcher reports it does not cover.
    ///
    /// The one clock left is `CHANGE_SET_BACKSTOP_MS`, which is not about
    /// staleness — the watcher covers that — but about the watcher having
    /// stopped without saying so.
    async fn probe_change_sets(self: &Arc<Self>) {
        let Ok(workspaces) = self.service.store.list_all_workspaces() else { return };

        let live: HashSet<Uuid> = workspaces.iter().map(|ws| ws.id).collect();
        self.service.review_cache.retain_counts(&live);
        self.change_set_probes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|id, _| live.contains(id));
        self.change_set_activity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|id, _| live.contains(id));

        // Drained once for the whole pass, before anything is decided, for the
        // reason `sample` drains the log watcher once a tick: every workspace
        // consults the same answer, and draining per workspace would give the
        // first one the news and the rest an empty set. Events that arrive
        // while this pass runs land in the next drain rather than being lost —
        // the set is what coalesces them, and it is never cleared by anything
        // but a drain.
        let moved = self.fs_watcher.drain_workspaces();

        // A directory git cares about that did not exist when the registration
        // walk ran. Its creation surfaced, because its parent is watched;
        // nothing written INSIDE it would, until it is registered too. Asked
        // here rather than in the watcher's own callback because the answer
        // costs a `stat` per unfamiliar path and the callback runs on the
        // backend's event thread.
        let rewalk: HashSet<Uuid> = moved
            .iter()
            .filter(|(id, paths)| self.fs_watcher.needs_rewalk(**id, paths))
            .map(|(id, _)| *id)
            .collect();
        let fleet: Vec<(Uuid, std::path::PathBuf)> = workspaces
            .iter()
            .map(|ws| (ws.id, std::path::PathBuf::from(&ws.worktree_path)))
            .collect();
        // A set comparison when the fleet has not changed, which is every pass
        // but the ones where a worktree appeared, went, or grew a directory.
        //
        // Awaited here rather than pushed off the executor, unlike the
        // repository half: this pass is ALREADY detached, and is detached for
        // exactly this class of reason — see `spawn_change_set_probe`. When it
        // does work it is a `git ls-files` and a run of `watch` calls, both of
        // which the tick loop would have to wait on if this ran there.
        self.fs_watcher.sync_workspaces(&fleet, &rewalk).await;

        let now = now_millis();
        for ws in workspaces {
            let worktree = std::path::PathBuf::from(&ws.worktree_path);
            // A worktree whose directory is gone is not a worktree that changed
            // nothing; it is one git will refuse, loudly, ten seconds at a time.
            if !worktree.is_dir() {
                continue;
            }

            let probed_at = self
                .change_set_probes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&ws.id)
                .copied();
            let worked = self
                .change_set_activity
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&ws.id)
                .copied();
            // Newer than the probe, not merely present: an agent that stopped an
            // hour ago has already had its last edit counted, and re-reading it
            // every three seconds for the rest of the night is the fleet-wide
            // `git` loop this is built to avoid.
            let worked_since = match (worked, probed_at) {
                (Some(w), Some(p)) => w > p,
                (Some(_), None) => true,
                (None, _) => false,
            };
            // Unprobed reads as due, which is what makes the first pass after a
            // daemon start reach every worktree — the same thing the missing
            // `counts_current` entry used to do.
            let backstop_due =
                probed_at.is_none_or(|probed| now.saturating_sub(probed) >= CHANGE_SET_BACKSTOP_MS);
            let unproven = self.service.review_cache.counts_unproven(ws.id);

            let worth_probing = if self.fs_watcher.covers_workspace(ws.id) {
                moved.contains_key(&ws.id) || worked_since || unproven || backstop_due
            } else {
                // The fallback, and the reason `cheap_gate` is still here. A
                // registration that failed — no backend, the inotify ceiling, a
                // worktree too large to watch — leaves this workspace exactly
                // where it was before any of this: two `stat`s and the activity
                // gate, which is a known and bounded gap rather than a daemon
                // that quietly stopped noticing edits. `fs_watch` says so out
                // loud when it lands here.
                worked_since || !self.service.review_cache.counts_current(ws.id, &worktree)
            };
            if !worth_probing {
                continue;
            }

            // The SAME base the change set uses, not a cheaper guess at one.
            //
            // The inbox once asked `default_base_for`, which lands on
            // `origin/HEAD`, while the change set resolves through the recorded
            // base, the PR's base ref and the repository's default branch. The
            // two disagree the moment local `main` is ahead of `origin/main`, and
            // every worktree was then charged with every unpushed commit on main:
            // a branch that added 8,821 lines was reported as 44,691, and a main
            // checkout with nothing to say at all was reported as 35,870. The
            // sidebar and the panel beside it described the same worktree with
            // different numbers.
            //
            // Resolved here rather than above the gate because it is not free:
            // for a repository whose default branch has never been read it falls
            // through to `guess_base`, which is two more git processes. Behind
            // the gate it runs only for a worktree that already moved.
            let base = crate::review_ops::resolve_base(&self.service, &ws).await.ok();
            let version = self
                .service
                .review_cache
                .recompute_counts(ws.id, &worktree, base.as_ref().map(|(b, _)| b.as_str()))
                .await;

            // Stamped whether or not the numbers moved, and whether or not the
            // computation succeeded. This records that we LOOKED, which is what
            // the activity gate above is asking about; stamping only on success
            // would put a worktree git cannot answer for into a three-second
            // retry loop for as long as an agent works in it.
            self.change_set_probes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(ws.id, now_millis());

            // The activity trace's upper half and its commit marks, off the
            // counts this pass just refreshed. Free by construction: a map
            // lookup for the diff, one `stat` for the reflog. See
            // `record_workspace_trace`, which states exactly what the upper
            // half is and what it cannot say.
            self.record_workspace_trace(ws.id, &worktree, now_millis() / 1000);

            if let Some(version) = version {
                self.announce_change_set(ws.id, version);
            }
        }
    }

    /// Reconcile repositories whose set of worktrees moved.
    ///
    /// Broadcasts only when something actually changed, for the same reason
    /// `sample` does: a fleet where nothing is happening produces no traffic,
    /// which is what makes a phone holding an SSH session overnight reasonable.
    ///
    /// The gate used to be one `stat` of `<common>/worktrees` per repository
    /// per tick — a poll, cheap but running all night on a runner where nothing
    /// was going to happen. `fs_watch` watches that directory instead, which is
    /// how a worktree appearing is learned without asking git for the list, and
    /// what retired the thirty-second forced pass that stood behind the `stat`.
    ///
    /// The `stat` is still here for a repository the watcher does not cover,
    /// and `RECONCILE_BACKSTOP_MS` still tries every repository on its own slow
    /// clock — five minutes rather than thirty seconds, because it is now
    /// insuring against a watcher that stopped rather than standing in for one
    /// that was never the primary signal.
    async fn reconcile_worktrees(&self) {
        let Ok(repositories) = self.service.list_repositories() else { return };

        // Drained before the loop and after the list, so a signal is never
        // taken out of the set for a repository this pass is not going to
        // visit.
        let awake = self.fs_watcher.drain_repositories();
        let fleet: Vec<(Uuid, std::path::PathBuf)> = repositories
            .iter()
            .map(|repo| (repo.id, std::path::PathBuf::from(&repo.canonical_git_dir)))
            .collect();
        // `awake` is passed in because `<common>/worktrees` does not exist until
        // the FIRST linked worktree creates it, and its creation is an event on
        // `<common>` — so this is the moment to look again, and the only moment
        // worth looking. See `TreeWatcher::sync_repositories`.
        //
        // Off the executor, on the same terms as `listening_ports` and
        // `foreground::read` in `sample`: registering a watch is a synchronous
        // call into the platform backend, and this one runs INSIDE the tick
        // loop rather than on a detached task. Awaiting it on the executor
        // would let a slow backend hold up the fleet's own sampling, and a
        // fleet whose rows stop moving is a worse failure than a worktree
        // registered a beat late.
        {
            let registry = Arc::clone(&self.fs_watcher);
            let (plan, waking) = (fleet.clone(), awake.clone());
            let _ = tokio::task::spawn_blocking(move || registry.sync_repositories(&plan, &waking))
                .await;
        }

        let now = now_millis();
        let mut changed = false;
        for repo in repositories {
            let common = std::path::PathBuf::from(&repo.canonical_git_dir);
            let reconciled_at = self
                .worktree_reconciles
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&repo.id)
                .copied();
            // Never reconciled reads as due, which is what makes the first pass
            // after a daemon start reach every repository — the job the
            // immediate first tick of the old forced interval used to do.
            let backstop_due = reconciled_at
                .is_none_or(|at| now.saturating_sub(at) >= RECONCILE_BACKSTOP_MS);

            let worth_reconciling = if self.fs_watcher.covers_repository(repo.id) {
                awake.contains(&repo.id) || backstop_due
            } else {
                // The fallback gate, unchanged: the mtime of the one directory
                // a worktree appears in. Only reached for a repository whose
                // registration failed, and `fs_watch` says so when it does.
                let previous = self
                    .worktree_marks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&repo.id)
                    .copied();
                match worktrees_changed(&common, previous) {
                    Some(mark) => {
                        self.worktree_marks
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(repo.id, mark);
                        true
                    }
                    None => backstop_due,
                }
            };
            if !worth_reconciling {
                continue;
            }

            // Stamped whether or not anything came of it, on the same terms as
            // `change_set_probes`: this records that we LOOKED, and stamping
            // only on success would put a repository git cannot answer for into
            // a once-a-second retry loop.
            self.worktree_reconciles
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(repo.id, now_millis());

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
        let mut change_sets = tokio::time::interval(CHANGE_SET_INTERVAL);
        change_sets.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick of an interval completes immediately, and comparing
        // the inventory against itself before anything has had a chance to
        // diverge would only ever report a false alarm.
        backstop.tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    self.sample().await;
                    // Gated: a drained set lookup per repository, and a git
                    // process only for repositories the filesystem says
                    // actually gained or lost a worktree. The separate forced
                    // pass this used to sit beside is gone — its job is now
                    // `RECONCILE_BACKSTOP_MS`, measured per repository inside.
                    self.reconcile_worktrees().await;
                }
                _ = backstop.tick() => self.service.backstop_reconcile().await,
                // Its first tick is immediate and that is wanted: every worktree
                // is unprobed at startup, so this pass is what puts a number
                // beside a row before anybody asks for one.
                _ = change_sets.tick() => self.spawn_change_set_probe(),
            }
        }
    }

    /// What this pane's own log says about its turn, after reading whatever it
    /// appended since the last tick.
    ///
    /// The entry is taken OUT of the map for the duration and put back after,
    /// so the lock is never held across the blocking read — let alone across
    /// the await. Nothing outside the sampling loop reads this map, and the loop
    /// visits each pane once per tick, so there is no second reader to see the
    /// gap.
    async fn turn_from_log(
        &self,
        id: Uuid,
        pane: PaneJoin,
        now: i64,
        churn: bool,
        working: bool,
    ) -> (Option<LogTurn>, Option<Ask>, Vec<TurnEvent>) {
        // Taken together, under one lock, and in this order: the entry for
        // THIS pane comes out first, so what is left is exactly the files
        // other panes hold. Without the removal a pane would find its own
        // tail in the set and refuse to re-adopt the file it is already
        // reading.
        //
        // A snapshot rather than a live view, because the join runs on a
        // blocking thread with no lock. It can only go stale by naming a file
        // whose pane has since gone, and one tick later that pane's entry is
        // pruned and the file is free again.
        let (log, claimed) = {
            let mut logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
            let log = logs.remove(&id).unwrap_or_else(PaneLog::new);
            let claimed = logs
                .values()
                .filter_map(|other| other.tail.as_ref())
                .map(|(tail, _)| tail.path().to_path_buf())
                .collect::<std::collections::HashSet<_>>();
            (log, claimed)
        };
        // Off the executor for the same reason `listening_ports` is: this can
        // spawn a process and read files, and the sampling loop is shared with
        // everything else the daemon is doing.
        let log = tokio::task::spawn_blocking(move || {
            advance_log(log, &pane, now, churn, working, |pane| {
                crate::log_join::find_session_log(
                    pane.preset.as_deref()?,
                    pane.pid,
                    &pane.cwd,
                    &pane.title,
                    &claimed,
                )
            })
        })
        .await;
        // A join error loses the file offset, and the next tick re-attaches at
        // the end of the file. That costs whatever was written in between,
        // which is the right trade against putting an entry back that a
        // panicking task may have left half-advanced.
        let Ok((log, steps)) = log else { return (None, None, Vec::new()) };
        let turn = log.turn;
        // Cloned rather than moved out: the pane goes on holding the question
        // until its own log says otherwise, and this call is a reading of that
        // state and not a hand-off of it.
        let asked = log.asked.clone();
        self.logs.lock().unwrap_or_else(|e| e.into_inner()).insert(id, log);
        (turn, asked, steps)
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
        // One `lsof` for the whole host, on the same cadence and for the
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
        // same answer, and a host that renamed itself mid-tick would
        // otherwise name two rows by two different rules.
        let hostname = crate::hostname();
        // Bound once per tick rather than looked up per pane: the registry
        // lives on `Service` for the same reason `root` does — re-reading the
        // config file's location on every pane would let it disagree with
        // itself mid-tick if the environment changed underneath.
        let registry = self.service.registry();
        // Drained once for the whole tick, before anything is decided: a pane
        // whose log has just appeared is worth looking up, and every pane
        // consults the same answer. Draining per pane would give the first pane
        // the news and the rest an empty set.
        let churn = !self.log_watcher.drain().is_empty();

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
                // the runner as `Lost` when tmux cannot be read at all — so
                // without this gate, one failed `list-panes` would delete every
                // changes pane in the fleet and kill the panes on the way past,
                // for a runner that was fine a second later.
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
                    pid: running.map(|r| r.pid),
                    cwd: workspace.workspace.worktree_path.clone(),
                    // The same string `wire::workspace` puts on the wire as
                    // `task_name`, off the same row, so a notification and the
                    // sidebar row it is about name the worktree identically.
                    workspace: workspace.workspace.name(),
                    workspace_id: workspace.workspace.id,
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

        // The live cards this pass has decided nothing is left to say about,
        // sent once at the end. See `card_orphaned` for the three ways a card
        // gets here and `retire_cards` for why they travel together.
        let mut orphaned: Vec<Uuid> = Vec::new();

        // Terminals that are gone stop being tracked, or a restarted one would
        // inherit the activity of the process it replaced.
        {
            let mut state = self.state.lock().await;
            let ids: std::collections::HashSet<Uuid> = live.iter().map(|s| s.id).collect();
            state.retain(|id, _| {
                if ids.contains(id) {
                    return true;
                }
                // Its card goes with it, and this is the only moment anything
                // can say so. Dropping the state was all this did, so a pane
                // closed while its agent was blocked left a card on the lock
                // screen reading "Waiting for your answer" with nothing left in
                // this process that had ever heard of the terminal — no
                // transition to observe, and so no `done` to end it with.
                //
                // Unconditionally, whatever the entry last observed. A record
                // that has been removed has no run behind it by construction,
                // and the relay does nothing at all for a terminal it holds no
                // card for — so naming one costs a `SELECT` that finds nothing,
                // where deciding here which ones "probably had a card" would be
                // this daemon guessing at a table it cannot see.
                orphaned.push(*id);
                false
            });
            // The file offset goes with it, for the same reason: a terminal
            // that came back would otherwise resume reading a log at the byte
            // the terminal it replaced had reached.
            self.logs.lock().unwrap_or_else(|e| e.into_inner()).retain(|id, _| ids.contains(id));
        }

        // Every live terminal's ring moves forward, whether or not this tick
        // had anything to put in it, and rings belonging to terminals that are
        // gone are dropped.
        //
        // Advancing a quiet ring is not bookkeeping: a slot that is not cleared
        // as it is passed still holds what was written into it 26 hours ago,
        // and the next read would draw yesterday afternoon as this one. See
        // `Trace::advance_to`.
        self.tick_traces(
            &live.iter().map(|s| (s.id, s.workspace_id)).collect(),
            now_millis() / 1000,
        );

        for Sampled {
            id,
            command,
            title,
            purpose,
            pid,
            cwd,
            workspace,
            workspace_id,
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
            // What the agent's own log recorded since the last tick. Set in
            // the same arm as `question` and for the mirror-image reason: that
            // is the only arm with a session log attached to read.
            let mut events: Vec<TurnEvent> = Vec::new();
            // Whether the agent's own log says the turn it just finished died.
            // Set in the same arm, and false everywhere else for the same
            // reason: a pane with no log to read has made no claim about how
            // its turn went, and "we could not tell" must never render as an
            // alarm.
            let mut turn_failed = false;
            // One clock for the whole of this pane's tick. The log's staleness
            // bound and the state clocks are answering questions about the same
            // instant, and reading the clock twice would let them disagree by
            // however long a `capture-pane` took.
            let now = now_millis();
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
                // reading titles at all: coming back to a runner, the useful
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
                        // How far the pane scrolled since the last tick, which
                        // is the trace's lower half.
                        //
                        // Here, and not in the pty byte stream, because the
                        // byte stream is not always there. Raw pane bytes reach
                        // a process exactly once — `fanout::serve_on` — which
                        // tmux starts when a client asks to watch a pane and
                        // which exits five seconds after the last one leaves.
                        // Nothing pipes a pane nobody is looking at, on purpose.
                        // A newline counter there would describe only the
                        // minutes somebody was watching, and the trace exists to
                        // describe the hours they were not.
                        //
                        // This capture, by contrast, already happens once a
                        // second for every live terminal whatever is connected,
                        // because the activity classifier needs it. The counter
                        // is a hash per row and a shift search over them; see
                        // `trace::lines_produced` for what it counts, what it
                        // saturates on, and why tmux's own `history_size`
                        // cannot be used instead.
                        let shape = farcooler_core::trace::ScreenShape::of(&screen);
                        let was = self
                            .trace_shapes
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(id, shape.clone());
                        // Nothing on the first sighting. A daemon that just
                        // started has no previous screen, and treating the
                        // whole of one as freshly produced would draw a bar
                        // on every terminal at every restart.
                        if let Some(was) = was {
                            let produced = farcooler_core::trace::lines_produced(&was, &shape);
                            if produced > 0 {
                                self.record_trace(
                                    id,
                                    now / 1000,
                                    farcooler_core::trace::Sample::output(produced),
                                );
                            }
                        }
                        // Which agent this is, by process where the process
                        // says and by what it drew where it does not — Claude
                        // Code renames itself to its version, so the screen is
                        // the only thing that finds it. Owned, because the
                        // borrow of `registry` cannot be held across the await
                        // below.
                        let agent =
                            registry.identify(&command, &screen).map(|rules| rules.preset.clone());
                        // Read once and used twice: what stage 1 alone makes of
                        // this pane is the log layer's own input as well as its
                        // fallback. A pane that stage 1 can see is working,
                        // whose file has gone quiet anyway, is a pane attached
                        // to the wrong file — see `join_looks_dead`.
                        let screen_says = registry.classify(&command, &screen);
                        let working = promoted_by_title(
                            screen_says,
                            &title,
                            &command,
                            &hostname,
                            title_repeats,
                        ) == AgentActivity::Working;
                        // The agent's own account of its own turn, which is
                        // what stage 2 exists to read. It cannot see a
                        // permission prompt, and `resolved_activity` is where
                        // that is enforced.
                        let (turn, held, read) = self
                            .turn_from_log(
                                id,
                                PaneJoin {
                                    preset: agent,
                                    pid,
                                    cwd: cwd.clone(),
                                    title: title.clone(),
                                },
                                now,
                                churn,
                                working,
                            )
                            .await;
                        events = read;
                        // Read off the log's standing verdict rather than off
                        // this tick's batch: the end of a turn moves the
                        // activity, and `CONFIRMATIONS` means the tick that
                        // ANNOUNCES that move is usually a later one, by which
                        // time the batch carrying the failure is long gone.
                        turn_failed = turn.is_some_and(|t| t.failed);
                        // Log, then title, then screen — except that a screen
                        // saying Blocked beats all of them. See
                        // `resolved_activity`.
                        let activity = resolved_activity(
                            screen_says,
                            turn,
                            held.is_some(),
                            now,
                            &title,
                            &command,
                            &hostname,
                            title_repeats,
                        );
                        // Only while blocked, and derived here rather than on a
                        // client for the same reason activity is: a phone has
                        // no screen to read.
                        //
                        // The screen first, where it has one: a permission
                        // prompt is only ever visible there, and its footer
                        // states the question better than any log could. The
                        // held question is what answers for the case the screen
                        // cannot see at all — an `AskUserQuestion` draws none of
                        // claude's usual furniture, so the footer match finds
                        // nothing to quote.
                        question = registry.blocked_question(&command, &screen).or_else(|| {
                            held.filter(|ask| !ask.question.trim().is_empty()).map(|ask| ask.question)
                        });
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
            // the runner — the same reason its activity does.
            let command = label;
            let blocked_question = question;

            let mut state = self.state.lock().await;
            // Read before `entry()` inserts one: a terminal seen for the first
            // time has no real history, and `Observed::begin` gives it `state:
            // Running` regardless of what was actually sampled — so without
            // this, a daemon restarted onto an already-dead terminal would read
            // as a Running-to-Exited transition on the very first tick.
            let just_appeared = !state.contains_key(&id);
            let entry = state.entry(id).or_insert_with(|| Observed::begin(observed, now));
            let previous_state = entry.state;

            let activity_moved = entry.observe(observed, now);

            // The one thing this loop knows that the review cache cannot: files
            // under this worktree are probably moving right now.
            //
            // Three readings of that, because one is not enough. The raw sample
            // is the direct one. The folded `activity` is read as well so the
            // tail of a turn still counts — it holds `Working` until the fold is
            // sure the agent has stopped, and an agent's last write of a turn
            // lands inside exactly that window. And a non-empty batch of log
            // steps says the agent's own session log advanced this tick, which
            // is the one that carries a pane whose activity this runner cannot
            // classify at all: `Unspecified` is a common honest answer, and a
            // worktree whose counts depended on a screen match would simply
            // never move.
            //
            // See `probe_change_sets`, which spends a `git diff` on this and on
            // nothing else that is not free.
            if observed == AgentActivity::Working
                || entry.activity == AgentActivity::Working
                || !events.is_empty()
            {
                self.change_set_activity
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(workspace_id, now);
                // WHICH pane, as well as whether any. The gate above only has
                // to know that this worktree is worth a `git diff`; the trace's
                // upper half has to put the result on a row, and a workspace's
                // diff belongs to whichever of its panes was working while it
                // grew. Cleared by `probe_change_sets` when it spends them.
                self.trace_workers
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(workspace_id)
                    .or_default()
                    .insert(id);
            }

            // Whether the relay is holding a card for a run that has gone.
            //
            // Before the announce gate below, and deliberately: the gate asks
            // whether a CLIENT would see anything new, and a card the daemon can
            // no longer account for has to come down whether or not a sidebar
            // row moved. A daemon that has just started is the case that makes
            // the difference — it inherits terminals whose rows read exactly as
            // they did a second ago, and every one of their cards is from a
            // previous life.
            let earned = card_still_earned(terminal_state, entry.activity);
            if card_orphaned(
                entry.card_settled,
                // `just_appeared` excluded for the reason it is excluded from
                // `exited_into_failure`: `Observed::begin` says `Running`
                // whatever was sampled, so a first sighting is not a crossing.
                // It does not need to be one — an unsettled entry is already
                // swept by the first arm of `card_orphaned`.
                !just_appeared && previous_state != terminal_state,
                activity_moved,
                earned,
            ) {
                orphaned.push(id);
            }
            // "Could not tell" settles nothing, and the sweep comes round again
            // next tick. See `card_settled`.
            if earned.is_some() {
                entry.card_settled = true;
            }

            // Folded before the gate is consulted, not after: a line that
            // arrived is what makes this tick worth announcing, and folding it
            // afterwards would hold every line back until something ELSE moved
            // — which for a long, quiet turn is nothing at all until it ends.
            //
            // Redaction and truncation happen inside `farcooler_core::feed`.
            // Nothing on this path may hand a raw tool argument to a client.
            let signals_moved = entry.saw_events(&events);
            let changed = entry.should_announce(
                activity_moved.is_some(),
                terminal_state,
                &command,
                &blocked_question,
                signals_moved,
            );
            if !changed {
                continue;
            }

            entry.state = terminal_state;
            entry.command = command.clone();
            entry.chat_capable = chat_capable;
            entry.blocked_question = blocked_question;
            entry.turn_failed = turn_failed;
            // A tier change is never withheld, and clearing the window is how
            // that is enforced for the one tier whose card is throttled.
            // Stamping `now` here instead would mean an agent that has just
            // STARTED working sits behind a card describing the turn before it
            // for up to ten seconds — the throttle silencing the transition it
            // exists to make room for.
            if activity_moved.is_some() {
                entry.last_card_push = None;
            }
            let record = entry.clone();
            drop(state);

            // Worth telling the owner about, and only on the transition.
            //
            // No second check needed here: `observe` returns `Some` only when
            // it just published a real activity change, so `Some` already
            // means the activity moved — a terminal whose command or question
            // changed reports `None` and never reaches this arm, which is what
            // keeps a `cd` from buzzing a phone.
            //
            // `Working` is the exception, and it is pushed from `announce`
            // rather than here. What a working card says is the composed signal
            // line, which does not exist until `wire::apply_rungs` has run a
            // few lines further on — pushing from here as well would spend an
            // APNs delivery on a card with a blank second line and then
            // overwrite it milliseconds later with the real one. The transition
            // is still unthrottled: `last_card_push` was just cleared above, so
            // the refresh in `announce` fires on this very tick.
            if let Some(next) = activity_moved.filter(|next| *next != AgentActivity::Working) {
                // And whether the person is sitting there watching it happen.
                //
                // Asked here, at the transition, rather than after the fact —
                // which is the whole reason `terminal.watching` exists beside
                // `terminal.seen`. `seen` is the same judgement arriving one
                // beat too late: a client reports it on its NEXT poll, up to
                // three seconds after the turn ended, by which time the push
                // has crossed the relay and buzzed a wrist. A claim of
                // attention is made in advance and is therefore already true
                // when the moment comes.
                match attention(next, self.is_watched(id, now)) {
                    // Nothing goes out, and the live card comes down silently
                    // on the same batched request the orphan sweep uses — see
                    // `retire_cards` at the end of this pass for why one
                    // request rather than one per terminal. Deliberately the
                    // same list `card_orphaned` fills: a `done` that nobody
                    // needs to be told about and a card whose run has vanished
                    // are the same instruction to the relay, and naming a
                    // terminal twice costs a `SELECT` that finds nothing the
                    // second time.
                    Attention::Retire => orphaned.push(id),
                    // A watched agent asking a question. See `attention` for
                    // why this leaves the card alone rather than retiring it.
                    Attention::Hold => {}
                    Attention::Announce => self.push_if_paired(
                        id,
                        next,
                        &command,
                        Quoted {
                            // The one fact here that is NOT off the record, because
                            // it is not a fact about the pane's state at all: a
                            // workspace's name does not change while a turn ends,
                            // and this is the sample's own copy of the row the
                            // outer loop walked to reach this terminal.
                            workspace: &workspace,
                            question: record.blocked_question.as_deref(),
                            // Off the same record as everything else here, for the
                            // reason the comment above gives: the card, the row and
                            // the banner must be quoting one read of this pane, not
                            // three.
                            //
                            // `said`, not `feed.lines().last()`. They are two cuts
                            // of one message and only one of them is a sentence's
                            // beginning — see `farcooler_core::feed::Feed::said`.
                            said: record.feed.said(),
                        },
                        record.turn_failed,
                        record.turn_started_at,
                    ),
                }
            }
            // A command failing is a STATE transition, not an activity one —
            // `activity_moved` never fires for it, so without this a `cargo
            // build` that exited 101 overnight reached the sidebar and never
            // the phone. See `exited_into_failure` for the exact rule.
            if exited_into_failure(just_appeared, previous_state, terminal_state, exit_code, exit_signal)
            {
                self.push_failed_exit(id, &command, &workspace, exit_code, exit_signal);
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

        // Last, and together. A sweep on the first tick after a restart can
        // name every pane on the runner, and one request per idle pane would be
        // a fleet's worth of round trips in the first second of every runner
        // update — on a loop that must not wait for any of them.
        self.retire_cards(orphaned);
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
            // Finished lines, already redacted and already cut to a row's
            // width. See `farcooler_core::feed` for why both happen here and
            // not on the three clients that render them.
            message.feed = observed.feed.lines();
            // The same last message the window above ends on, cut from its
            // OPENING instead — which is not recoverable from those lines, so
            // it travels as its own field rather than being re-derived by
            // three clients that would each get it wrong the same way. This is
            // what the two apps' local notifications quote, so that a banner
            // raised on this Mac and a push delivered to the phone beside it
            // are one sentence. See `farcooler_core::feed::Feed::said`.
            message.said = observed.feed.said().map(str::to_string);
            // The agents this one spawned, named, on the same terms: a
            // description is text the agent wrote and takes the same cleaning
            // every other line here does.
            message.subagents = observed.signals.subagents();
            // The thirteen buckets. Off the ring rather than off `observed`,
            // because a trace is history and an `Observed` is one moment — but
            // through the SAME method the poll path calls, so the two cannot
            // hand a client different histories of one pane.
            message.activity_trace = self.trace(terminal).into();
            // How the last turn went, which `apply_rungs` below narrows into
            // the glyph and the headline. Without it a turn that died reaches
            // every client as an ordinary `done`.
            message.turn_failed = observed.turn_failed;
            // The compact ladder, computed from everything just set above —
            // see `wire::apply_rungs` for why it has to run last, and why the
            // signal line is handed to it rather than read off the message.
            // The counts behind that line's task-list rung, off the SAME
            // `Signals` in the same expression — one fold of one `Observed`,
            // so the `3/7` in the string and the 3 and the 7 beside it cannot
            // be a tick apart. See `wire::apply_rungs`.
            wire::apply_rungs(
                &mut message,
                observed.signals.line(observed.mid_turn()).as_deref(),
                observed.signals.plan_counts(),
            );

            // The live card, from the same line the sidebar is about to draw.
            //
            // Here rather than at the push site above because `message.line` is
            // the composed rung and only exists once `apply_rungs` has run —
            // and a card built from anything else is a phone and a Mac
            // disagreeing about the same agent in the same second, which is the
            // failure this whole module is arranged to prevent.
            //
            // Not a banner: the relay refuses to ALERT on `working`. It will
            // start a card from one — silently, so the card covers the whole
            // run — and after that this only ever moves the card already up.
            // Nothing on this path can interrupt anyone.
            if observed.activity == AgentActivity::Working && !message.line.is_empty() {
                self.refresh_card(terminal, &observed.command, &message.line).await;
            }

            // A send with no subscribers is not a failure: it is the ordinary
            // case of a runner nobody is watching, which still has to keep
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
/// `stat` per repository is nothing, where one `git worktree list` per
/// repository is a process spawn almost always spent to learn that nothing
/// happened.
///
/// **The fallback gate, not the primary one.** `fs_watch` watches that same
/// directory and says when it moved, which costs nothing on a repository where
/// nothing is happening — this runs only for a repository whose registration
/// failed, and it is kept precisely so that such a repository lands where it
/// was before rather than somewhere worse.
///
/// Returns the new mtime when it moved, `None` when it did not. A repository
/// with no linked worktrees has no such directory and reports no change, which
/// is correct: `RECONCILE_BACKSTOP_MS` still covers anything this misses.
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
    /// The upper half of the trace, and every case where it must say nothing.
    ///
    /// The three zeroes matter more than the number: each of them is a
    /// worktree's diff moving for a reason that is not an agent writing code in
    /// the last few seconds, and drawing a bar for any of them puts activity on
    /// a row that had none.
    #[test]
    fn the_code_share_refuses_to_invent_work() {
        use super::code_share;
        assert_eq!(code_share(None, 4_000, 1), 0, "a first reading is not growth");
        assert_eq!(code_share(Some(4_000), 3_000, 1), 0, "a shrinking diff is not negative work");
        assert_eq!(code_share(Some(10), 200, 0), 0, "nobody was working, so nobody wrote it");
        assert_eq!(code_share(Some(10), 10, 1), 0, "a diff that did not move is not work");
    }

    /// And the case where it does say something, split among the panes that
    /// were working. The sum is exact; each row is an approximation.
    #[test]
    fn the_code_share_splits_growth_among_the_panes_that_were_working() {
        use super::code_share;
        assert_eq!(code_share(Some(100), 142, 1), 42, "one pane gets all of it");
        assert_eq!(code_share(Some(100), 142, 2), 21, "two panes split it");
        assert_eq!(code_share(Some(100), 142, 3), 14, "three panes split it");
        assert_eq!(code_share(Some(0), 1, 2), 0, "a split too small to divide is not rounded up");
    }

    /// A pane holding a question is blocked, however long its log has been
    /// quiet.
    ///
    /// The bug this closes, end to end: claude calls `AskUserQuestion` and
    /// stops writing. `STALE_LOG_MS` expires 30 seconds later, the turn rung
    /// stops answering, and the row fell through to a screen showing none of
    /// claude's furniture. The median real wait is 101 seconds and the longest
    /// measured is 21 hours.
    #[test]
    fn a_held_question_is_blocked_however_stale_the_log_is() {
        let now = 1_000_000;
        let open = log_said(true, now - STALE_LOG_MS * 100, 0);
        assert_eq!(
            resolved_activity(AgentActivity::Idle, open, true, now, "", "claude", "Mac", 0),
            AgentActivity::Blocked
        );
        // And with no log verdict at all, which is what a pane that attached
        // mid-question looks like.
        assert_eq!(
            resolved_activity(AgentActivity::Idle, None, true, now, "", "claude", "Mac", 0),
            AgentActivity::Blocked
        );
    }

    /// The screen still outranks it, because a permission prompt is visible
    /// only there and its footer states the question better.
    #[test]
    fn a_screen_that_says_blocked_still_wins() {
        let now = 1_000_000;
        assert_eq!(
            resolved_activity(AgentActivity::Blocked, None, true, now, "", "claude", "Mac", 0),
            AgentActivity::Blocked
        );
    }

    /// A question outranks the turn rung, which is the whole of the fix: the
    /// log says the turn is open, and open is exactly what a turn is while it
    /// waits for you.
    #[test]
    fn a_question_beats_a_running_turn() {
        let now = 1_000_000;
        let fresh = log_said(true, now, 0);
        assert_eq!(
            resolved_without_a_question(AgentActivity::Idle, fresh, now, "", "claude", "Mac", 0),
            AgentActivity::Working,
            "without one, a fresh running turn is working"
        );
        assert_eq!(
            resolved_activity(AgentActivity::Idle, fresh, true, now, "", "claude", "Mac", 0),
            AgentActivity::Blocked
        );
    }

    #[test]
    fn a_question_is_held_until_its_own_answer_arrives() {
        let asked = |id: &str, q: &str| TurnEvent::Asked { id: id.to_string(), question: q.to_string() };
        let answered = |id: &str| TurnEvent::Answered { id: id.to_string() };

        let held = fold_asks(None, &[asked("a", "Which base?")]);
        assert_eq!(held, Some(Ask { id: "a".to_string(), question: "Which base?".to_string() }));

        // Somebody else's result changes nothing. Every tool result emits an
        // `Answered`, so this is the common case rather than the exotic one.
        let held = fold_asks(held, &[answered("b"), answered("c")]);
        assert!(held.is_some(), "a question is not cleared by another call coming back");

        assert_eq!(fold_asks(held, &[answered("a")]), None);
    }

    #[test]
    fn a_second_question_replaces_the_first() {
        let events = vec![
            TurnEvent::Asked { id: "a".to_string(), question: "First?".to_string() },
            TurnEvent::Answered { id: "a".to_string() },
            TurnEvent::Asked { id: "b".to_string(), question: "Second?".to_string() },
        ];
        let held = fold_asks(None, &events);
        assert_eq!(held, Some(Ask { id: "b".to_string(), question: "Second?".to_string() }));
        // The first question's answer must not clear the second, which is why
        // the id is carried at all.
        assert_eq!(fold_asks(held.clone(), &[TurnEvent::Answered { id: "a".to_string() }]), held);
    }

    /// A rejected question never gets a result shaped like an answer -- 5 of
    /// the 200 on this machine came back as `"User rejected tool use"` -- and a
    /// turn boundary is what closes those.
    #[test]
    fn a_turn_boundary_clears_a_question_nobody_answered() {
        let held = Some(Ask { id: "a".to_string(), question: "Which base?".to_string() });
        assert_eq!(
            fold_asks(held.clone(), &[TurnEvent::Ended {
                at_ms: None,
                duration_ms: None,
                outcome: TurnOutcome::Aborted
            }]),
            None
        );
        assert_eq!(fold_asks(held, &[TurnEvent::Started { at_ms: None }]), None);
    }

    /// A pane re-joined to a different file has nothing waiting on it. The old
    /// question belonged to a session that is over, and keeping it would pin
    /// the row on "needs you" for a question nobody can answer.
    #[test]
    fn a_new_session_file_drops_the_question_with_the_turn() {
        let mut log = PaneLog::new();
        log.tail = Some((Tail::new(std::path::PathBuf::from("/tmp/one.jsonl")), LogFormat::Claude));
        log.asked = Some(Ask { id: "a".to_string(), question: "Which base?".to_string() });
        log.adopt(Some((std::path::PathBuf::from("/tmp/two.jsonl"), LogFormat::Claude)));
        assert_eq!(log.asked, None);
    }

    /// A finished agent says what it finished, not merely that it did.
    ///
    /// "claude finished" is a sentence about Far Cooler. The subtitle was
    /// empty, so that was the whole of what arrived on a lock screen — and the
    /// agent's own account of the turn was already sitting on the record that
    /// produced the notice.
    #[test]
    fn a_finished_agent_says_what_it_did() {
        let quoted = Quoted {
            workspace: "add-auth",
            question: None,
            said: Some("I've added the tail window and fixed the stale action"),
        };
        let notice = notification(AgentActivity::Done, "claude", quoted, false, None).expect("done");
        assert_eq!(notice.title, "claude finished");
        assert_eq!(
            notice.subtitle,
            "add-auth — I've added the tail window and fixed the stale action"
        );
    }

    /// A notification has to say WHICH pane finished, and the agent's name is
    /// the one thing that cannot.
    ///
    /// The product owner runs several codexes at once, so `codex finished`
    /// with nothing under it identified nothing at all — the title names the
    /// harness, which is exactly what every pane in the fleet has in common.
    /// The worktree is what tells them apart, and it goes in the BODY because
    /// the title is already a sentence and because both local notifiers have
    /// written `workspace — said` since they gained a body: the owner reads a
    /// Mac banner and a phone push about the same pane, and they must not be
    /// two different notifications.
    #[test]
    fn a_finished_agent_says_which_workspace_it_finished_in() {
        let quoted =
            Quoted { workspace: "add-auth", question: None, said: Some("Both tests pass.") };
        let one = notification(AgentActivity::Done, "codex", quoted, false, None).expect("done");
        let other = notification(
            AgentActivity::Done,
            "codex",
            Quoted { workspace: "fix-flaky-ci", ..quoted },
            false,
            None,
        )
        .expect("done");
        assert_eq!(one.title, other.title, "same agent, and the title cannot tell them apart");
        assert_eq!(one.subtitle, "add-auth — Both tests pass.");
        assert_eq!(other.subtitle, "fix-flaky-ci — Both tests pass.");
    }

    /// A turn can be all tool calls and no prose. That reads as the row it
    /// belongs to rather than as a blank line pretending to be content — and
    /// the workspace on its own is still an answer to "which one finished",
    /// which is the more useful half.
    #[test]
    fn a_finished_agent_that_said_nothing_still_says_where() {
        for quiet in [None, Some(""), Some("   ")] {
            let quoted = Quoted { workspace: "add-auth", question: None, said: quiet };
            let notice =
                notification(AgentActivity::Done, "claude", quoted, false, None).expect("done");
            assert_eq!(notice.subtitle, "add-auth", "{quiet:?}");
        }
    }

    /// And with neither, nothing at all — never a dangling dash.
    ///
    /// `Quoted::default()` is what the tests below hand over and what a pane
    /// whose worktree path yields no name would produce. `— Both tests pass.`
    /// reads as a rendering bug, and a lock screen is not where anybody can
    /// debug one.
    #[test]
    fn a_notice_with_nothing_to_say_says_nothing_rather_than_a_bare_dash() {
        let notice =
            notification(AgentActivity::Done, "claude", Quoted::default(), false, None).expect("done");
        assert_eq!(notice.subtitle, "");
        let unnamed = Quoted { workspace: "  ", question: None, said: Some("Both tests pass.") };
        let notice = notification(AgentActivity::Done, "claude", unnamed, false, None).expect("done");
        assert_eq!(notice.subtitle, "Both tests pass.");
    }

    /// A failed turn names its workspace too. Being told an agent died is
    /// worth very little if it does not say which one.
    #[test]
    fn a_failed_turn_says_which_workspace_died() {
        let quoted = Quoted { workspace: "add-auth", ..Quoted::default() };
        let notice = notification(AgentActivity::Done, "codex", quoted, true, None).expect("done");
        assert_eq!(notice.title, "codex failed");
        assert_eq!(notice.subtitle, "add-auth — Its last turn didn’t finish");
    }

    /// The said belongs to the turn that ended, not to the question in
    /// front of it: a blocked agent is asking, and what it said before asking
    /// is not the answer to "what does it need".
    ///
    /// The workspace rides in front of the question for the same reason it
    /// rides in front of an answer: `codex needs you / Do you want to create
    /// haiku.txt?` is answerable from a lock screen and still does not say
    /// which of three codexes is asking it.
    #[test]
    fn a_blocked_agent_still_quotes_its_question() {
        let quoted = Quoted {
            workspace: "add-auth",
            question: Some("Overwrite fruit.txt?"),
            said: Some("Almost there"),
        };
        let notice =
            notification(AgentActivity::Blocked, "claude", quoted, false, None).expect("blocked");
        assert_eq!(notice.subtitle, "add-auth — Overwrite fruit.txt?");
    }

    /// A `Quoted` carrying only a question, which is what these tests are about.
    /// The said half arrives from the agent's log and has its own tests, and
    /// the workspace half is asserted where it is the point rather than
    /// repeated at every call site that is about something else.
    fn just_asking(question: &str) -> Quoted<'_> {
        Quoted { workspace: "", question: Some(question), said: None }
    }

    /// `resolved_activity` for the tests that predate questions, which is most
    /// of them: they are about the screen, the title and the turn, and a pane
    /// holding a question is a rung above all three. Spelling `false` at every
    /// call site would say nothing at each of them and bury the handful of
    /// tests where the flag is the whole point.
    fn resolved_without_a_question(
        screen: AgentActivity,
        log: Option<LogTurn>,
        now: i64,
        title: &str,
        command: &str,
        hostname: &str,
        title_repeats: u8,
    ) -> AgentActivity {
        resolved_activity(screen, log, false, now, title, command, hostname, title_repeats)
    }

    use super::*;
    use farcooler_core::session_log::TurnOutcome;

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
    ///
    /// The fifth is the same argument for the feed: an agent ten minutes into
    /// a turn moves none of the other four, and a step is the only news there
    /// is about it.
    #[test]
    fn each_of_the_five_terms_alone_opens_the_announce_gate() {
        let mut entry = Observed::begin(AgentActivity::Blocked, 1_000);
        entry.state = TerminalState::Running;
        entry.command = "claude".to_string();
        entry.blocked_question = Some("proceed?".to_string());

        let state = entry.state;
        let command = entry.command.clone();
        let question = entry.blocked_question.clone();

        assert!(
            !entry.should_announce(false, state, &command, &question, false),
            "nothing changed, so there is nothing to announce"
        );
        assert!(entry.should_announce(true, state, &command, &question, false), "activity moved");
        assert!(
            entry.should_announce(false, TerminalState::Exited, &command, &question, false),
            "pane state changed"
        );
        assert!(entry.should_announce(false, state, "bash", &question, false), "command changed");
        assert!(
            entry.should_announce(false, state, &command, &Some("a different question?".to_string()), false),
            "the question changed to another question — the regression this test exists to catch"
        );
        assert!(
            entry.should_announce(false, state, &command, &None, false),
            "the question vanished — that is news too, not just a new one arriving"
        );
        assert!(
            entry.should_announce(false, state, &command, &question, true),
            "a step arrived — the row can say what it is doing, and nothing else moved to carry it"
        );
    }

    /// A finished agent KEEPS its transcript.
    ///
    /// The user asked for this outright, and it is the whole reason the
    /// transcript is worth building: coming back to a machine, "what did it do
    /// while I was away" is the question, and a row that emptied itself when
    /// the turn ended would have thrown the answer away at the moment it became
    /// useful. The task list is the opposite case and clears with the turn —
    /// see `a_new_turn_starts_from_no_task_list`.
    #[test]
    fn a_finished_agent_keeps_its_feed() {
        let mut entry = Observed::begin(AgentActivity::Working, 1_000);
        for said in ["Reading watch.rs.", "Editing feed.rs.", "Both tests pass."] {
            entry.feed.push(said);
        }
        let while_working = entry.feed.lines();

        let entry = entry.advance_to(AgentActivity::Done, 7_000);
        assert_eq!(entry.feed.lines(), while_working, "the turn ended; the summary did not");
        let entry = entry.advance_to(AgentActivity::Idle, 9_000);
        assert_eq!(entry.feed.lines(), while_working, "and being seen does not clear it either");
    }

    /// The value the notification's second line is read off, at the moment it
    /// is read off.
    ///
    /// `notification` is tested against a said handed to it directly, and
    /// `a_finished_agent_keeps_its_feed` proves the feed survives the
    /// transition. This is the link between them: that a turn ending with an
    /// answer leaves that answer as `feed.said()`, which is what
    /// `push_if_paired` takes off the record on exactly that tick. Without it
    /// the two halves could both pass while the sentence that reaches a lock
    /// screen was empty.
    #[test]
    fn a_finished_turn_leaves_its_answer_where_the_push_looks() {
        let mut entry = Observed::begin(AgentActivity::Working, 1_000);
        entry.saw_events(&[
            TurnEvent::Said { text: "Reading watch.rs.".to_string(), conclusion: false },
            TurnEvent::Said { text: "Both tests pass.".to_string(), conclusion: true },
            TurnEvent::Ended {
                at_ms: None,
                duration_ms: None,
                outcome: TurnOutcome::Finished,
            },
        ]);
        // Narration first, the answer last: each message replaces the one
        // before it, so the answer is what `said` hands back rather than the
        // narration that led to it.
        let entry = entry.advance_to(AgentActivity::Done, 7_000);
        assert_eq!(entry.feed.said(), Some("Both tests pass."));
    }

    /// The same link for the message that made this a bug report, end to end:
    /// a codex rollout line, through the parser, through the fold, to the
    /// string the push site reads.
    ///
    /// The owner's notification said `batches to avoid N+1 shits.` The parser
    /// was innocent — that message arrived correctly flagged as the turn's
    /// final answer, and the census in `session_log::codex` is that `phase` is
    /// present on every one of the 1,065 final answers in `~/.codex/sessions`
    /// on this machine. What went wrong was downstream of all of it: the push
    /// read the last WRAPPED ROW of a three-row window, which for a
    /// 101-character sentence is its last twenty-seven characters. So this
    /// asserts both readings of one message at once — the transcript still
    /// ends where it ended, and the notification now opens where the sentence
    /// opens.
    #[test]
    fn the_notification_quotes_the_start_of_the_answer_and_the_row_still_ends_it() {
        const ANSWER: &str = "More shit. An industrial quantity of shit, shipped in \
                              carefully authorized batches to avoid N+1 shits.";
        let mut entry = Observed::begin(AgentActivity::Working, 1_000);
        entry.saw_events(&[TurnEvent::Said { text: ANSWER.to_string(), conclusion: true }]);
        let entry = entry.advance_to(AgentActivity::Done, 7_000);

        assert_eq!(entry.feed.lines().last().map(String::as_str), Some("batches to avoid N+1 shits."));
        assert_eq!(entry.feed.said(), Some(ANSWER));

        let notice = notification(
            AgentActivity::Done,
            "codex",
            Quoted { workspace: "add-auth", question: None, said: entry.feed.said() },
            false,
            None,
        )
        .expect("done");
        assert_eq!(notice.title, "codex finished");
        assert_eq!(notice.subtitle, format!("add-auth — {ANSWER}"));
    }

    /// And the same when the agent never wrote a conclusion at all.
    ///
    /// The parser is right about this one too: narration is narration, and the
    /// window is meant to keep its tail. But a turn that stops mid-narration
    /// has still stopped — there is no answer coming — and its notification
    /// still has to open where the sentence opens. That is why the head cut
    /// lives beside the window rather than inside `conclude`.
    #[test]
    fn a_turn_that_ended_on_narration_still_gets_quoted_from_the_start() {
        const NARRATION: &str = "Reading the watcher first, then the parser it calls, and \
                                 only then changing anything at all.";
        let mut entry = Observed::begin(AgentActivity::Working, 1_000);
        entry.saw_events(&[TurnEvent::Said { text: NARRATION.to_string(), conclusion: false }]);
        let entry = entry.advance_to(AgentActivity::Done, 7_000);

        assert!(
            entry.feed.lines().last().is_some_and(|line| line.ends_with("anything at all.")),
            "the window keeps watching the tail: {:?}",
            entry.feed.lines()
        );
        assert_eq!(entry.feed.said(), Some(NARRATION));
    }

    /// The transcript is what the agent SAID, and nothing else in the batch.
    ///
    /// `fold_log_events` reads the same batch for turn boundaries; this is the
    /// other half of what one read is worth. The tool call is the case that
    /// matters: it used to land in this list prefixed with its own lowercased
    /// tool name, which is the complaint this stage exists to answer. It is now
    /// the signal line instead, and both halves are asserted on one batch so
    /// neither can quietly take the other's place.
    #[test]
    fn only_what_the_agent_said_reaches_the_transcript() {
        let mut entry = Observed::begin(AgentActivity::Working, 1_000);
        let events = vec![
            TurnEvent::Started { at_ms: None },
            TurnEvent::Did { verb: "write".to_string(), object: "haiku.txt".to_string() },
            TurnEvent::Title("Write a haiku".to_string()),
            TurnEvent::Said { text: "Written to haiku.txt.".to_string(), conclusion: true },
        ];
        assert!(entry.saw_events(&events), "the row moved");
        assert_eq!(entry.feed.lines(), vec!["Written to haiku.txt."]);
        assert_eq!(entry.signals.line(true).as_deref(), Some("Writing haiku.txt"));
    }

    /// A tool argument is where a token lives, and this is the path one would
    /// travel: session log to `Observed` to the wire. The cleaning happens in
    /// `farcooler_core::feed` — this pins that the daemon actually routes
    /// through it rather than stashing the raw string somewhere alongside.
    #[test]
    fn a_planted_credential_does_not_survive_the_trip_into_an_observed() {
        let mut entry = Observed::begin(AgentActivity::Working, 1_000);
        let events = vec![
            TurnEvent::Did {
                verb: "bash".to_string(),
                object: "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI make deploy".to_string(),
            },
            TurnEvent::Said { text: "Deployed with AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI".to_string(), conclusion: false },
            // Agent-authored text, on the two paths that are new: a task's own
            // phrase and a subagent's description travel exactly as far.
            TurnEvent::TaskState {
                id: None,
                active_form: Some("Deploying with AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI".to_string()),
                status: None,
            },
            TurnEvent::TaskState { id: Some("1".to_string()), active_form: None, status: None },
            TurnEvent::TaskState { id: Some("1".to_string()), active_form: None, status: Some(TaskStatus::InProgress) },
            TurnEvent::Subagent {
                id: "toolu_1".to_string(),
                description: "Auditing AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI".to_string(),
                running: true,
            },
        ];
        entry.saw_events(&events);

        let mut everything = entry.feed.lines();
        everything.extend(entry.signals.line(true));
        everything.extend(entry.signals.subagents());
        for line in &everything {
            assert!(!line.contains("wJalrXUtnFEMI"), "a live credential reached a client: {line}");
        }
        assert_eq!(entry.feed.lines()[0], "Deployed with AWS_SECRET_ACCESS_KEY=…");
        assert_eq!(entry.signals.subagents(), vec!["Auditing AWS_SECRET_ACCESS_KEY=…"]);
    }

    // ---------------------------------------------------------------------
    // The fold: a task list and the agents it spawned, across ticks
    // ---------------------------------------------------------------------

    /// Helpers named for the three lines claude actually writes, so a test
    /// below reads as the sequence a session really produces.
    fn created(form: &str) -> TurnEvent {
        TurnEvent::TaskState { id: None, active_form: Some(form.into()), status: None }
    }
    fn numbered(id: &str) -> TurnEvent {
        TurnEvent::TaskState { id: Some(id.into()), active_form: None, status: None }
    }
    fn moved(id: &str, status: TaskStatus) -> TurnEvent {
        TurnEvent::TaskState { id: Some(id.into()), active_form: None, status: Some(status) }
    }

    /// One task fact per line, and a tally no line ever stated.
    ///
    /// The shape the whole stage rests on: a create carries a phrase and no id,
    /// its result carries the id and no phrase, an update carries an id and a
    /// status, and the row reads `1/2 · Identifying edge cases` only because
    /// the three were joined here.
    #[test]
    fn a_task_list_folds_into_a_count_and_the_phrase_it_is_on() {
        let mut signals = Signals::default();
        for event in [
            created("Designing test matrix"),
            numbered("1"),
            created("Identifying edge cases"),
            numbered("2"),
            moved("1", TaskStatus::InProgress),
        ] {
            signals.saw(&event);
        }
        assert_eq!(signals.line(true).as_deref(), Some("0/2 · Designing test matrix"));

        // The first finishes and the second starts: both halves of the line
        // move, off two lines that each stated only one of them.
        signals.saw(&moved("1", TaskStatus::Completed));
        signals.saw(&moved("2", TaskStatus::InProgress));
        assert_eq!(signals.line(true).as_deref(), Some("1/2 · Identifying edge cases"));
    }

    /// The numbers on the wire and the numbers in the line are ONE fold.
    ///
    /// `plan_counts` exists so that no client has to read `1/2` back out of a
    /// composed string. That only holds if the two are the same fact: a fold
    /// that answered the line from one read of `tasks` and the counts from
    /// another could put `1/2` on the row and a two-thirds-full bar beside it.
    /// Asserted in one test rather than two so the pair cannot be edited apart.
    #[test]
    fn the_counts_on_the_wire_are_the_counts_in_the_line() {
        let mut signals = Signals::default();
        assert_eq!(signals.plan_counts(), None, "no list yet is not 0/0");

        for event in [
            created("Designing test matrix"),
            numbered("1"),
            created("Identifying edge cases"),
            numbered("2"),
            moved("1", TaskStatus::InProgress),
        ] {
            signals.saw(&event);
        }
        assert_eq!(signals.line(true).as_deref(), Some("0/2 · Designing test matrix"));
        // Zero done and a real total, which is exactly the pair that must not
        // be spelled "absent" anywhere on the way to a client.
        assert_eq!(signals.plan_counts(), Some((0, 2)));

        signals.saw(&moved("1", TaskStatus::Completed));
        signals.saw(&moved("2", TaskStatus::InProgress));
        assert_eq!(signals.line(true).as_deref(), Some("1/2 · Identifying edge cases"));
        assert_eq!(signals.plan_counts(), Some((1, 2)));

        // A total can GROW: an agent adds work to its own list as it finds it.
        // The counts say so honestly rather than holding the denominator still,
        // and a surface that animates them must not read this as going
        // backwards. See the fields' comments in `farcooler.proto`.
        signals.saw(&created("Auditing the rules"));
        signals.saw(&numbered("3"));
        assert_eq!(signals.plan_counts(), Some((1, 3)));

        // And it can shrink, for the reason `a_deleted_task_leaves_the_total`
        // records: a dropped task counts toward neither half.
        signals.saw(&moved("3", TaskStatus::Deleted));
        assert_eq!(signals.plan_counts(), Some((1, 2)));

        // A turn boundary forgets the list, both ends of it, and the counts go
        // with it rather than describing seven tasks the next turn never wrote.
        signals.saw(&TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Finished });
        assert_eq!(signals.plan_counts(), None);
    }

    /// The correction this stage's live run forced, as a test.
    ///
    /// Task ids are numbered per SESSION, not per turn: a second turn that
    /// creates seven tasks gets ids `"8"` through `"14"`. The join the plan
    /// proposed — the k-th create is id `"k"` — therefore matched nothing at
    /// all from a session's second turn on, and every update naming an
    /// unrecognized id added a task on top: a list of seven read `3/11`, with
    /// no phrase. Both halves of the row wrong, on the second thing anybody
    /// would try.
    #[test]
    fn a_second_turns_task_list_is_numbered_where_the_first_left_off() {
        let mut signals = Signals::default();
        signals.saw(&TurnEvent::Started { at_ms: None });
        for (form, id) in [("Designing test matrix", "1"), ("Identifying edge cases", "2")] {
            signals.saw(&created(form));
            signals.saw(&numbered(id));
        }
        signals.saw(&TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Finished });

        // A second turn, and claude carries on numbering from three.
        signals.saw(&TurnEvent::Started { at_ms: None });
        for (form, id) in [("Writing the report", "3"), ("Auditing the rules", "4")] {
            signals.saw(&created(form));
            signals.saw(&numbered(id));
        }
        signals.saw(&moved("3", TaskStatus::Completed));
        signals.saw(&moved("4", TaskStatus::InProgress));
        assert_eq!(
            signals.line(true).as_deref(),
            Some("1/2 · Auditing the rules"),
            "two tasks in this turn, not four, and the phrase joined to the right one"
        );
    }

    /// A total can SHRINK. `TaskUpdate` has a `deleted` status and agents use
    /// it, so a fold that assumed totals only grow reports `2/3` for a list of
    /// two — a progress indicator that can never reach its own end.
    #[test]
    fn a_deleted_task_leaves_the_total() {
        let mut signals = Signals::default();
        for (n, form) in ["Designing test matrix", "Identifying edge cases", "Writing the report"]
            .iter()
            .enumerate()
        {
            signals.saw(&created(form));
            signals.saw(&numbered(&(n + 1).to_string()));
        }
        signals.saw(&moved("1", TaskStatus::Completed));
        signals.saw(&moved("2", TaskStatus::Completed));
        assert_eq!(signals.line(true).as_deref(), Some("2/3"));

        signals.saw(&moved("3", TaskStatus::Deleted));
        assert_eq!(signals.line(true).as_deref(), Some("2/2"), "a dropped task leaves both halves");
    }

    /// What is left to go wrong, and the half that must survive it.
    ///
    /// A pane that attached to its log mid-turn sees updates for tasks whose
    /// creates it never read. The COUNT must survive and only the phrase go
    /// missing: a row that says `1/2` with nothing after it is most of the
    /// value, and a row that named the wrong task would be confidently wrong,
    /// which is worse than quiet.
    #[test]
    fn an_update_naming_a_task_nobody_created_still_counts() {
        let mut signals = Signals::default();
        signals.saw(&created("Designing test matrix"));
        signals.saw(&numbered("1"));
        // An id no create was ever seen for — a pane that joined mid-turn.
        signals.saw(&moved("42", TaskStatus::Completed));
        assert_eq!(signals.line(true).as_deref(), Some("1/2"), "the count survived");

        // And the phrase of a task claimed by an id it was never numbered with
        // goes rather than being guessed at.
        let mut orphaned = Signals::default();
        orphaned.saw(&created("Designing test matrix"));
        orphaned.saw(&moved("42", TaskStatus::InProgress));
        assert_eq!(orphaned.line(true).as_deref(), Some("0/1"), "no phrase rather than a guessed one");
    }

    /// A plan belongs to the turn that wrote it.
    #[test]
    fn a_new_turn_starts_from_no_task_list() {
        let mut signals = Signals::default();
        signals.saw(&created("Designing test matrix"));
        signals.saw(&numbered("1"));
        signals.saw(&moved("1", TaskStatus::InProgress));
        signals.saw(&TurnEvent::Subagent { id: "toolu_1".into(), description: "Auditing rules".into(), running: true });
        assert_eq!(signals.line(true).as_deref(), Some("0/1 · Designing test matrix · 1 agent"));

        signals.saw(&TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Finished });
        assert_eq!(signals.line(false), None, "the plan ended with the turn that wrote it");
        assert!(signals.subagents().is_empty());
    }

    /// A new turn does not wear the last turn's work.
    ///
    /// Found by driving it, not by reading it: for the first NINE SECONDS of a
    /// new turn the row still read `Running test "$(< fruit.txt)" = banana` —
    /// the command the previous turn had finished with — because the plan and
    /// the spawns were cleared at a turn boundary and the action was not. Nine
    /// seconds is most of a short turn, so a person watching a row saw it
    /// confidently report work that was over before they had asked for
    /// anything.
    ///
    /// The end of a turn is deliberately NOT the same case: see `Signals::saw`.
    #[test]
    fn a_new_turn_does_not_wear_the_last_turns_action() {
        let mut signals = Signals::default();
        signals.saw(&TurnEvent::Did {
            verb: "bash".into(),
            object: "test \"$(< fruit.txt)\" = banana".into(),
        });
        signals.saw(&TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Finished });
        assert_eq!(
            signals.line(false).as_deref(),
            Some("Ran test \"$(< fruit.txt)\" = banana"),
            "a finished row still says what it did, in the tense it did it in"
        );

        signals.saw(&TurnEvent::Started { at_ms: None });
        assert_eq!(signals.line(true), None, "and a new turn starts from nothing");
    }

    /// A background spawn's result arrives at LAUNCH, not at the end.
    ///
    /// A third of the spawns measured on this machine come back within seconds
    /// carrying `async_launched` while the agent runs for another hour. Reading
    /// that as an ending would empty the list the instant it filled.
    #[test]
    fn a_launched_agent_stays_on_the_row_until_it_completes() {
        let mut signals = Signals::default();
        signals.saw(&TurnEvent::Subagent {
            id: "toolu_1".into(),
            description: "Auditing the redaction rules".into(),
            running: true,
        });
        signals.saw(&TurnEvent::Subagent {
            id: "toolu_2".into(),
            description: "Checking the join against real logs".into(),
            running: true,
        });
        assert_eq!(
            signals.subagents(),
            vec!["Auditing the redaction rules", "Checking the join against real logs"]
        );

        // The launch coming back, description restated, still running.
        signals.saw(&TurnEvent::Subagent {
            id: "toolu_2".into(),
            description: "Checking the join against real logs".into(),
            running: true,
        });
        assert_eq!(signals.subagents().len(), 2, "a launch is not an ending");

        // Completion carries no description, and must not un-name the agent it
        // is taking off the list.
        signals.saw(&TurnEvent::Subagent { id: "toolu_1".into(), description: String::new(), running: false });
        assert_eq!(signals.subagents(), vec!["Checking the join against real logs"]);
    }

    /// A batch that says nothing new must not wake every connected client.
    #[test]
    fn a_batch_that_changes_nothing_is_not_worth_announcing() {
        let mut entry = Observed::begin(AgentActivity::Working, 1_000);
        let events = vec![TurnEvent::TaskState {
            id: Some("1".into()),
            active_form: None,
            status: Some(TaskStatus::InProgress),
        }];
        assert!(entry.saw_events(&events), "the first sighting is news");
        assert!(!entry.saw_events(&events), "the same fact restated is not");
        assert!(!entry.saw_events(&[]), "and an empty batch never is");
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

    // ------------------------------------------------------------------
    // The log layer. See `resolved_activity` for the order and why it is
    // that order.
    // ------------------------------------------------------------------

    /// A log that has just said something, `age` milliseconds ago.
    fn log_said(running: bool, now: i64, age: i64) -> Option<LogTurn> {
        Some(LogTurn { running, failed: false, last_event_at: now - age })
    }

    /// The log outranks the screen while it is fresh.
    ///
    /// A screen classified as `Idle` is a footer that matched nothing — a
    /// `capture-pane` taken mid-redraw, an agent whose footer this build does
    /// not know, a pane scrolled somewhere else. The agent's own log saying it
    /// started a turn is a better answer than any of them, and the whole stage
    /// is here to use it.
    #[test]
    fn a_log_that_says_the_turn_started_beats_a_screen_that_says_idle() {
        let now = 1_000_000;
        assert_eq!(
            resolved_without_a_question(AgentActivity::Idle, log_said(true, now, 0), now, "", "claude", "Mac", 0),
            AgentActivity::Working
        );
    }

    /// And it outranks a title that disagrees, which is the layer between.
    ///
    /// A live spinner would promote `Idle` to `Working` all on its own (see
    /// `promoted_by_title`), so a log saying the turn is OVER has to be
    /// consulted first or the title would hold the row on Working using a frame
    /// that is simply the last one the agent drew.
    #[test]
    fn a_fresh_log_outranks_the_title_as_well_as_the_screen() {
        let now = 1_000_000;
        let spinner = "◐ Write tmux haiku";
        // Stage 1, for contrast: the title alone promotes.
        assert_eq!(
            promoted_by_title(AgentActivity::Idle, spinner, "claude", "Mac", 0),
            AgentActivity::Working
        );
        assert_eq!(
            resolved_without_a_question(
                AgentActivity::Idle,
                log_said(false, now, 0),
                now,
                spinner,
                "claude",
                "Mac",
                0
            ),
            AgentActivity::Idle,
            "the agent wrote down that the turn ended; a spinner frame does not overrule that"
        );
    }

    /// A stale footer is exactly what the log is for.
    ///
    /// `esc to interrupt` left on screen after the turn ended — the redraw that
    /// never came, the pane that was scrolled back — held a row on Working with
    /// nothing to contradict it. The log's `turn_duration` record contradicts
    /// it, and `Idle` against a `Working` is what `activity::advance` makes
    /// `Done` out of. Asserted through the real fold rather than by expecting
    /// `Done` out of this function: `Done` is created by the fold and by nothing
    /// else, which is what keeps it from being folded twice.
    #[test]
    fn a_log_that_says_the_turn_ended_folds_a_stale_working_footer_into_done() {
        let now = 1_000_000;
        let observed = resolved_without_a_question(
            AgentActivity::Working,
            log_said(false, now, 0),
            now,
            "",
            "claude",
            "Mac",
            0,
        );
        assert_eq!(observed, AgentActivity::Idle, "the log's answer is Idle, never Done");
        assert_eq!(
            activity::advance(AgentActivity::Working, observed),
            AgentActivity::Done,
            "and the fold is what makes Done out of it"
        );
    }

    /// THE test this stage exists to keep passing. Do not weaken it.
    ///
    /// NO AGENT WRITES DOWN THAT IT IS WAITING FOR A HUMAN. Claude records the
    /// OUTCOME of a permission decision — `toolDenialKind` — and never the
    /// pending state, across 276 files; codex has nothing approval-shaped in
    /// 183; cursor shows none in the four transcripts that exist. So while a
    /// permission prompt is on screen the log says only what it can say, which
    /// is that the turn is still open — and it is: approving a tool call does
    /// not begin a new turn.
    ///
    /// If the log ever wins here, a permission prompt stops reaching the
    /// sidebar and stops reaching the phone, and "this agent needs you" is the
    /// one sentence this product exists to say. The log is fresh, the log is
    /// unambiguous, and it still loses.
    /// Printed as well as asserted, so `--nocapture` shows the whole chain
    /// rather than the absence of a failure. What a reader needs to see is not
    /// only that one function returned `Blocked` but that the row publishes it
    /// and the phone is told — this is the one path the product exists for.
    #[test]
    fn a_screen_that_says_blocked_beats_a_log_that_says_the_turn_is_running() {
        let now = 1_000_000;
        let log = log_said(true, now, 0);
        let screen = AgentActivity::Blocked;
        let resolved = resolved_without_a_question(screen, log, now, "◐ Write tmux haiku", "claude", "Mac", 0);
        assert_eq!(resolved, AgentActivity::Blocked);

        // Through the fold and the announce gate, from a row that was Working
        // one tick ago: this is what a permission prompt arriving mid-turn
        // actually looks like.
        let mut entry = Observed::begin(AgentActivity::Working, now);
        let published = entry.observe(resolved, now + 1_000);
        let notice =
            notification(resolved, "claude", just_asking("Do you want to create haiku.txt?"), false, None);

        println!("screen said      : {screen:?}");
        println!("log said         : {log:?}  (a turn still open, written this instant)");
        println!("resolved_activity: {resolved:?}");
        println!("published        : {published:?}");
        println!("row activity     : {:?}", entry.activity);
        println!(
            "notification     : {:?}",
            notice.as_ref().map(|n| (n.title.as_str(), n.subtitle.as_str(), n.status))
        );

        assert_eq!(published, Some(AgentActivity::Blocked), "and it publishes at once");
        assert_eq!(entry.activity, AgentActivity::Blocked);
        assert_eq!(notice.map(|n| n.status), Some("blocked"), "the phone raises a live card");

        // Every shape the log could be in, including the one that has just
        // said the turn is over: the screen owns Blocked outright, not merely
        // when the log happens to agree that something is happening.
        for log in [None, log_said(true, now, 0), log_said(false, now, 0), log_said(true, now, 999_999)] {
            assert_eq!(
                resolved_without_a_question(AgentActivity::Blocked, log, now, "", "claude", "Mac", 0),
                AgentActivity::Blocked,
                "{log:?}"
            );
        }
    }

    /// A pane with no log is stage 1, unchanged.
    ///
    /// Asserted against `promoted_by_title` itself rather than against a
    /// hand-written expectation, so the two can never drift: whatever stage 1
    /// would have decided is by construction what a pane with no log gets.
    #[test]
    fn a_pane_with_no_log_falls_through_to_the_title_and_then_the_screen() {
        let now = 1_000_000;
        for screen in [
            AgentActivity::Idle,
            AgentActivity::Working,
            AgentActivity::Done,
            AgentActivity::None,
            AgentActivity::Unspecified,
        ] {
            for title in ["◐ Write tmux haiku", "✳ Write tmux haiku", "Echo Banana", ""] {
                for repeats in [0, STALE_TITLE_SAMPLES] {
                    assert_eq!(
                        resolved_without_a_question(screen, None, now, title, "claude", "Mac", repeats),
                        promoted_by_title(screen, title, "claude", "Mac", repeats),
                        "{screen:?} / {title} / {repeats}"
                    );
                }
            }
        }
    }

    /// A turn the agent wrote down as finished does not start itself again.
    ///
    /// The bug this pins is the one the whole feature was built to remove,
    /// coming back through the feature: a pane stuck on Working forever with a
    /// turn timer reading hours. `esc to interrupt` is left on the screen after
    /// the turn ends — claude genuinely does this — so the screen says Working
    /// for the rest of the pane's life. The log's `turn_duration` folds that
    /// into `Done` correctly, and then thirty seconds of the silence a finished
    /// agent is supposed to produce used to drop the log out of trust, hand the
    /// last word back to that same unredrawn footer, and flip the row to
    /// Working with a brand new turn clock — where it stayed.
    ///
    /// Driven through `observe` rather than asserted off `resolved_activity`,
    /// because the published row and the clock behind it are the symptom; the
    /// helper returning `Idle` is only how it is avoided.
    #[test]
    fn a_finished_turn_does_not_start_itself_again_while_the_log_stays_quiet() {
        let start = 1_000_000;
        let stale_footer = AgentActivity::Working;
        let ended = Some(LogTurn { running: false, failed: false, last_event_at: start });

        let mut entry = Observed::begin(AgentActivity::Working, start);
        assert_eq!(entry.turn_started_at, Some(start), "a turn is running");

        // The log records `turn_duration`. Two agreeing samples, because only
        // Blocked publishes on sight.
        let mut published = None;
        for tick in 1..=i64::from(CONFIRMATIONS) {
            let now = start + tick * 1_000;
            let resolved =
                resolved_without_a_question(stale_footer, ended, now, "", "claude", "Mac", 0);
            published = entry.observe(resolved, now);
        }
        assert_eq!(published, Some(AgentActivity::Done), "the turn ended, and the row said so");
        assert_eq!(entry.turn_started_at, None, "and the turn clock stopped with it");

        // Then two minutes of nothing, which is exactly what a finished agent
        // writes. Every second of it is the same non-news, and none of it is a
        // reason to believe the turn resumed: the agent said it was over, and
        // no second passing makes that less true.
        for tick in 0..=120 {
            let now = start + STALE_LOG_MS + tick * 1_000;
            let resolved = resolved_without_a_question(stale_footer, ended, now, "", "claude", "Mac", 0);
            let moved = entry.observe(resolved, now);
            assert_eq!(moved, None, "nothing changed {tick}s past the staleness bound");
            assert_eq!(entry.activity, AgentActivity::Done, "at {tick}s");
            assert_eq!(entry.turn_started_at, None, "no turn clock restarted at {tick}s");
        }
    }

    /// Quitting the agent leaves a shell, and a shell is not an idle agent.
    ///
    /// The reported bug: after exiting claude, the sidebar row went on
    /// carrying the agent's summary and its last task. A finished turn is
    /// believed forever and deliberately so — `last_event_at` is never
    /// consulted for an ended turn — and `join_looks_dead` will not let go of
    /// the tail either, since it returns false the moment `working` is false.
    /// So `Some(LogTurn { running: false })` sat there returning `Idle` for a
    /// pane that had been a plain zsh prompt for an hour.
    ///
    /// `Idle` is not `None` anywhere downstream: `wire::rung_subject` only
    /// falls back to the command for `None`, and every client reads
    /// `activity != none` as "this is an agent", so the pane went on being
    /// counted in fleet summaries and written into widget snapshots.
    ///
    /// The screen is the thing that can tell: `classify` returns `None` when
    /// it cannot identify an agent on it at all, and a log cannot make a pane
    /// into an agent that the screen says is not one.
    #[test]
    fn a_finished_log_does_not_keep_a_shell_looking_like_an_agent() {
        let now = 1_000_000;
        let ended = Some(LogTurn { running: false, failed: false, last_event_at: now - 60_000 });

        // The agent is gone: nothing on the screen identifies one.
        assert_eq!(
            resolved_without_a_question(AgentActivity::None, ended, now, "", "zsh", "Mac", 0),
            AgentActivity::None,
            "a pane running a shell reported as an agent"
        );

        // Still on screen, between turns: that IS an idle agent, and the
        // finished log is exactly how it is known.
        assert_eq!(
            resolved_without_a_question(AgentActivity::Idle, ended, now, "", "claude", "Mac", 0),
            AgentActivity::Idle,
            "an agent between turns stopped reading as one"
        );

        // The running half is deliberately NOT gated the same way: a log
        // saying a turn is open is evidence about a pane whose screen may
        // simply not have redrawn yet.
        let open = Some(LogTurn { running: true, failed: false, last_event_at: now });
        assert_eq!(
            resolved_without_a_question(AgentActivity::None, open, now, "", "claude", "Mac", 0),
            AgentActivity::Working,
            "an open turn stopped being believed over an unredrawn screen"
        );
    }

    /// A log that stopped being written stops being believed.
    ///
    /// The same trap `STALE_TITLE_SAMPLES` closed for the title's spinner,
    /// arriving through another door. An agent killed mid-turn leaves a log
    /// whose last record is a turn that started and never ended; believing that
    /// forever holds the row on Working, so `activity::advance` never produces
    /// `Done` and that pane never notifies anyone again. Reopening it here
    /// would be reopening it for the layer that now outranks the title.
    #[test]
    fn a_log_that_stopped_being_written_stops_being_believed() {
        let now = 1_000_000;
        // Just inside the bound: still believed, and the screen is overruled.
        assert_eq!(
            resolved_without_a_question(
                AgentActivity::Idle,
                log_said(true, now, STALE_LOG_MS - 1),
                now,
                "",
                "claude",
                "Mac",
                0
            ),
            AgentActivity::Working
        );
        // On the bound and past it: the screen has the last word again, which
        // is what lets the row reach Idle and fold to Done.
        for age in [STALE_LOG_MS, STALE_LOG_MS + 1, STALE_LOG_MS * 60] {
            assert_eq!(
                resolved_without_a_question(AgentActivity::Idle, log_said(true, now, age), now, "", "claude", "Mac", 0),
                AgentActivity::Idle,
                "age {age}"
            );
        }
    }

    /// What a batch of appended events makes of the turn.
    #[test]
    fn the_fold_moves_the_turn_only_on_a_boundary() {
        let started = || TurnEvent::Started { at_ms: None };
        let ended =
            || TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Finished };
        let step = || TurnEvent::Did { verb: "bash".into(), object: "cargo test".into() };

        let running = fold_log_events(None, &[started()], 5_000).expect("a start is a boundary");
        assert!(running.running);
        assert_eq!(running.last_event_at, 5_000);

        let over = fold_log_events(Some(running), &[ended()], 6_000).expect("so is an end");
        assert!(!over.running);
        assert_eq!(over.last_event_at, 6_000);

        // A step is not a boundary, but it IS the file being written, which is
        // the only thing the clock is asked about — a turn running a long tool
        // call must not go stale while its own log says it is alive.
        let stepped = fold_log_events(Some(running), &[step()], 7_000).expect("still running");
        assert!(stepped.running, "a step does not end a turn");
        assert_eq!(stepped.last_event_at, 7_000, "but it does prove the log is alive");

        // Attached mid-turn: a step with no boundary ever seen cannot say
        // whether a turn is open, and inventing one would report Working for an
        // agent that had merely written something.
        assert_eq!(fold_log_events(None, &[step()], 8_000), None);

        // Nothing appended leaves the clock alone, which is what lets
        // STALE_LOG_MS eventually fire.
        assert_eq!(fold_log_events(Some(over), &[], 9_000), Some(over));

        // Both boundaries in one batch: the last one wins, in file order, which
        // is the only order these logs guarantee.
        let both = fold_log_events(None, &[started(), ended()], 10_000).expect("folded");
        assert!(!both.running);
    }

    /// The join is rate-limited, and the watcher is not the only thing that can
    /// trigger it.
    #[test]
    fn an_unresolved_pane_is_looked_up_on_a_gate_with_a_backstop() {
        // Never looked up: `PaneLog::new` starts at zero and the clock is unix
        // milliseconds, so a pane seen for the first time is looked up on that
        // tick rather than after a wait it has no reason to serve.
        assert!(worth_a_join(0, now_millis(), false));

        let last = 1_000_000;
        // Inside the interval nothing gets a lookup, however much churn there
        // is — an `lsof` per pane per second is what this floor exists to stop.
        assert!(!worth_a_join(last, last + LOG_JOIN_INTERVAL_MS - 1, true));
        // Above the floor, a changed file under a log root is the ordinary
        // trigger: a session file cannot appear under a root that did not move.
        assert!(worth_a_join(last, last + LOG_JOIN_INTERVAL_MS, true));
        // Above the floor with no event at all, nothing happens until the
        // backstop — which must still fire, because `LogWatcher::start` degrades
        // to watching nothing when the platform backend fails to build, and a
        // feature that silently switches itself off is the defect class this
        // project keeps finding.
        assert!(!worth_a_join(last, last + LOG_JOIN_INTERVAL_MS, false));
        assert!(worth_a_join(last, last + LOG_JOIN_BACKSTOP_MS, false));
    }

    /// A join that must never be asked for, for the tests that pre-attach a
    /// tail: reaching it means something re-joined a pane with a live, quiet
    /// log, which is the thrash `join_looks_dead` exists to refuse.
    fn never_joined(_: &PaneJoin) -> Option<std::path::PathBuf> {
        panic!("this pane is already attached; nothing here should look one up")
    }

    /// The whole read path over a real file, with no join involved.
    ///
    /// Tail plus parser plus fold, so a change that breaks the wiring between
    /// them fails here rather than in a live session at three in the morning.
    /// The join itself is `log_join`'s own suite; what this pins is that what
    /// the agent appends actually reaches `LogTurn`.
    #[test]
    fn appending_a_real_claude_turn_moves_the_turn() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "").unwrap();

        // Pre-attached: this exercises reading, not finding.
        let mut log = PaneLog::new();
        log.tail = Some((Tail::new(path.clone()), LogFormat::Claude));
        // Never consulted, because the log is already attached — the join is
        // `log_join`'s own suite.
        let pane = PaneJoin {
            preset: Some("claude".to_string()),
            pid: None,
            cwd: "/tmp".to_string(),
            title: String::new(),
        };

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, r#"{{"type":"user","promptSource":"typed","message":{{"role":"user"}}}}"#).unwrap();
        let (log, events) = advance_log(log, &pane, 5_000, false, false, never_joined);
        assert_eq!(log.turn, Some(LogTurn { running: true, failed: false, last_event_at: 5_000 }));
        assert!(
            matches!(events.as_slice(), [TurnEvent::Started { .. }]),
            "a turn start is a boundary and nothing the agent did: {events:?}"
        );

        // A tool result is ALSO `type: "user"` and must not read as a second
        // turn start; it carries no `promptSource`.
        writeln!(
            file,
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result"}}]}}}}"#
        )
        .unwrap();
        let (log, _) = advance_log(log, &pane, 6_000, false, false, never_joined);
        assert_eq!(
            log.turn,
            Some(LogTurn { running: true, failed: false, last_event_at: 5_000 }),
            "a tool result is not an event, so it neither ends the turn nor touches the clock"
        );

        // The other half of what one read is worth: what the agent DID, on its
        // way to the row. Appended as claude actually writes it, so the whole
        // chain — tail, parser, fold, redaction, truncation — is exercised on a
        // real line rather than a hand-built `TurnEvent`.
        writeln!(
            file,
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Write","input":{{"file_path":"/Users/example/project/haiku.txt"}}}}]}}}}"#
        )
        .unwrap();
        let (log, events) = advance_log(log, &pane, 6_500, false, false, never_joined);
        let mut entry = Observed::begin(AgentActivity::Working, 6_500);
        entry.saw_events(&events);
        assert_eq!(entry.signals.line(true).as_deref(), Some("Writing haiku.txt"));

        writeln!(file, r#"{{"type":"system","subtype":"turn_duration","durationMs":1200}}"#).unwrap();
        let (log, events) = advance_log(log, &pane, 7_000, false, false, never_joined);
        entry.saw_events(&events);
        assert_eq!(log.turn, Some(LogTurn { running: false, failed: false, last_event_at: 7_000 }));
        assert_eq!(
            entry.signals.line(false).as_deref(),
            Some("Wrote haiku.txt"),
            "the turn ended; what it last did is still what a finished row reports"
        );

        // And that end, against a screen still showing `esc to interrupt`, is
        // the Done this whole stage is for.
        assert_eq!(
            activity::advance(
                AgentActivity::Working,
                resolved_without_a_question(AgentActivity::Working, log.turn, 7_000, "", "claude", "Mac", 0)
            ),
            AgentActivity::Done
        );
    }

    /// A pane's FIRST turn is seen, which for codex it never was.
    ///
    /// The join can only succeed once codex has opened its rollout, and codex
    /// opens it when the first turn is submitted — so `task_started` is
    /// already written by the time anything can look. A tail that attached at
    /// the end therefore began after the only line saying a turn was open,
    /// `fold_log_events` dropped the steps that followed (a step with no turn
    /// believed is not evidence of one), and the row stayed on stage 1 until
    /// the SECOND turn started.
    ///
    /// Through `advance_log` rather than `Tail` directly, because the fix has
    /// to survive the join: it is `PaneLog::adopt` that builds the tail, and
    /// the whole point is where that tail begins.
    #[test]
    fn a_pane_joining_a_codex_rollout_mid_first_turn_sees_that_turn() {
        const TASK_STARTED: &str =
            r#"{"type":"event_msg","payload":{"type":"task_started","started_at":1786866892}}"#;
        const AGENT_SAID: &str = r#"{"type":"event_msg","payload":{"type":"agent_message","phase":"final_answer","message":"Writing the haiku now."}}"#;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-2026-08-16T09-14-52-abc.jsonl");
        // Everything codex writes before anyone could possibly have found the
        // file: the session opens, the turn starts, the agent gets to work.
        append(&path, TASK_STARTED);
        append(&path, AGENT_SAID);

        let pane = PaneJoin {
            preset: Some("codex".to_string()),
            pid: None,
            cwd: "/tmp".to_string(),
            title: String::new(),
        };
        let (log, events) =
            advance_log(PaneLog::new(), &pane, 5_000, true, false, |_| Some(path.clone()));

        assert_eq!(
            log.turn,
            Some(LogTurn { running: true, failed: false, last_event_at: 5_000 }),
            "the turn the pane is in the middle of"
        );
        let mut entry = Observed::begin(AgentActivity::Working, 5_000);
        entry.saw_events(&events);
        assert_eq!(entry.feed.lines(), vec!["Writing the haiku now."], "and what it has said so far");
        // Which is the whole point: the row can now say the pane is working
        // during its first turn, not only from its second.
        assert_eq!(
            resolved_without_a_question(AgentActivity::Idle, log.turn, 5_000, "", "codex", "Mac", 0),
            AgentActivity::Working
        );
    }

    /// A turn that DIED, read out of the file the agent really wrote.
    ///
    /// The finding this exists for: `TurnOutcome::Failed` was parsed correctly
    /// by all three parsers and then read by nothing, so a cursor turn that
    /// ended on `status: "error"` — the recorded one says "Named models
    /// unavailable" — folded to exactly the Idle a clean turn folds to,
    /// reached `activity::advance` as the same `Done`, and drew the same `✓`.
    /// A fleet glanced at from a lock screen therefore said everything was
    /// fine while one agent had stopped working.
    ///
    /// The real fixture, through the real tail and the real parser, so this
    /// fails if any link in that chain drops the outcome — not only if the
    /// fold does.
    #[test]
    fn a_turn_that_ended_in_an_error_is_not_reported_as_a_clean_finish() {
        const TURN_FAILED: &str =
            include_str!("../../core/fixtures/session-logs/cursor-turn-ended-error.jsonl");
        const TURN_SUCCEEDED: &str = r#"{"type":"turn_ended","status":"success"}"#;
        const TURN_ABORTED: &str = r#"{"type":"turn_ended","status":"aborted"}"#;

        let pane = PaneJoin {
            preset: Some("cursor".to_string()),
            pid: None,
            cwd: "/tmp".to_string(),
            title: String::new(),
        };

        // Each ending replayed on its own file, from the same open turn.
        let ended_with = |ending: &str| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("transcript.jsonl");
            std::fs::write(&path, "").unwrap();
            let mut log = PaneLog::new();
            log.tail = Some((Tail::new(path.clone()), LogFormat::Cursor));

            append(&path, r#"{"role":"user","message":"write the haiku"}"#);
            let (log, _) = advance_log(log, &pane, 5_000, false, false, never_joined);
            assert_eq!(log.turn.map(|t| t.running), Some(true), "the turn is open");

            append(&path, ending.trim());
            let (log, _) = advance_log(log, &pane, 6_000, false, false, never_joined);
            log.turn.expect("the ending is a boundary")
        };

        let died = ended_with(TURN_FAILED);
        assert!(!died.running, "the turn is over either way");
        assert!(died.failed, "and the log said it came back an error");

        // A clean end and a CANCELLED one are both "not failed", and for
        // different reasons: one worked, and the other is a decision somebody
        // made on purpose. Escape is not an alarm.
        assert!(!ended_with(TURN_SUCCEEDED).failed);
        assert!(!ended_with(TURN_ABORTED).failed, "the user pressed escape; nothing broke");

        // What the row ends up saying. The activity is `Done` for all three —
        // that is the fold's business and stays untouched — so the failure has
        // to travel beside it, and this is where it becomes a `✗`.
        let folded = activity::advance(
            AgentActivity::Working,
            resolved_without_a_question(AgentActivity::Working, Some(died), 6_000, "", "cursor", "Mac", 0),
        );
        assert_eq!(folded, AgentActivity::Done, "Done is still created by the fold alone");
        let mut message = farcooler_protocol::v1::Terminal {
            activity: folded as i32,
            activity_changed_at: Some(wire::timestamp(crate::review::now_millis())),
            current_command: "cursor".to_string(),
            turn_failed: died.failed,
            ..Default::default()
        };
        wire::apply_rungs(&mut message, None, None);
        assert_eq!(message.glyph, "✗", "a dead turn drew the same tick as a healthy one");
        assert_eq!(message.headline, "cursor failed");
        // And the phone is told the same thing the sidebar shows.
        assert_eq!(
            notification(folded, "cursor", Quoted::default(), died.failed, None).map(|n| n.title),
            Some("cursor failed".to_string())
        );
    }

    /// One session file, and the pane's turn as claude would write it.
    ///
    /// Shared by the re-join tests, which care about WHICH file is being read
    /// rather than about the grammar inside it — that is
    /// `appending_a_real_claude_turn_moves_the_turn`'s subject.
    fn append(path: &std::path::Path, line: &str) {
        use std::io::Write;
        let mut file =
            std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        writeln!(file, "{line}").unwrap();
    }

    const TURN_STARTED: &str = r#"{"type":"user","promptSource":"typed","message":{"role":"user"}}"#;
    const TURN_ENDED: &str = r#"{"type":"system","subtype":"turn_duration","durationMs":1200}"#;
    const WROTE_A_FILE: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/Users/example/project/haiku.txt"}}]}}"#;

    /// A pane whose agent starts a NEW session stops reading the old one.
    ///
    /// `/clear` starts a new session uuid, which is a new file; so does
    /// restarting the agent in the same pane. The old file stays on disk,
    /// perfectly readable, and never grows again — so a join made once and
    /// never questioned leaves the row frozen on the previous session's last
    /// three steps with `log.turn` stuck at `None` forever, which is stage 1
    /// with the cost of stage 2 and none of the benefit. The only way out used
    /// to be deleting the terminal.
    #[test]
    fn a_pane_that_starts_a_new_session_stops_reading_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.jsonl");
        let second = dir.path().join("second.jsonl");
        std::fs::write(&first, "").unwrap();

        let pane = PaneJoin {
            preset: Some("claude".to_string()),
            pid: None,
            cwd: "/tmp".to_string(),
            title: String::new(),
        };
        // Stands in for `find_session_log`, which answers out of a real
        // `$HOME`: whichever session file exists now, the way `/clear` puts a
        // second one in the same project directory. Counted, because "how often
        // is this asked" is half of what is under test — an answer this
        // expensive must not be asked for every tick.
        let joins = std::cell::Cell::new(0usize);
        let find = |_: &PaneJoin| {
            joins.set(joins.get() + 1);
            Some(if second.exists() { second.clone() } else { first.clone() })
        };

        // A first session: joined, and read while it is being written.
        let start = 1_000_000;
        let (log, _) = advance_log(PaneLog::new(), &pane, start, true, false, find);
        assert_eq!(joins.get(), 1, "a pane with no log is looked up at once");
        append(&first, TURN_STARTED);
        let (log, _) = advance_log(log, &pane, start + 1_000, true, true, find);
        assert_eq!(log.turn, Some(LogTurn { running: true, failed: false, last_event_at: start + 1_000 }));
        append(&first, TURN_ENDED);
        let (log, _) = advance_log(log, &pane, start + 2_000, true, true, find);
        assert_eq!(log.turn, Some(LogTurn { running: false, failed: false, last_event_at: start + 2_000 }));
        assert_eq!(joins.get(), 1, "a live log is never re-joined; that is the cache");

        // `/clear`. A new file, a new turn typed into it, and not one more byte
        // written to the old one for the rest of the pane's life.
        append(&second, TURN_STARTED);

        // The screen says this agent is working, and it is. The file we hold
        // says nothing, because it is not the file being written any more.
        // Nothing is re-joined until that silence outlives the trust a log
        // gets, so a pane between two tool calls is not looked up over and over.
        let mut log = log;
        for tick in 1..(STALE_LOG_MS / 1_000) {
            let (advanced, events) = advance_log(log, &pane, start + 2_000 + tick * 1_000, true, true, find);
            log = advanced;
            assert!(events.is_empty(), "the old file has nothing left to say");
            assert_eq!(joins.get(), 1, "still inside the log's own trust, at {tick}s");
        }

        // Past it, the contradiction stands: re-join.
        let (log, _) = advance_log(log, &pane, start + 2_000 + STALE_LOG_MS, true, true, find);
        assert_eq!(joins.get(), 2, "a visibly working pane whose file went quiet is looked up again");
        assert_eq!(log.turn, None, "and what the OLD session said about its turn goes with it");

        // And the pane is reading the new session now, which is the whole
        // point: what the agent does next reaches the feed and the turn.
        append(&second, WROTE_A_FILE);
        append(&second, TURN_ENDED);
        let (log, events) = advance_log(log, &pane, start + 40_000, true, true, find);
        let mut entry = Observed::begin(AgentActivity::Working, start + 40_000);
        entry.saw_events(&events);
        // Past tense: the appended turn ended. What matters here is that the
        // row is reading the NEW session at all.
        assert_eq!(entry.signals.line(false).as_deref(), Some("Wrote haiku.txt"), "the row unfroze");
        assert_eq!(log.turn, Some(LogTurn { running: false, failed: false, last_event_at: start + 40_000 }));
    }

    /// An agent that is simply resting is never looked up again.
    ///
    /// The other half of the same rule, and the one that keeps re-joining
    /// affordable. A healthy agent left alone overnight writes nothing for
    /// hours — and its screen says nothing either, so there is no contradiction
    /// to resolve. Looking one up anyway would spend an `lsof` and a directory
    /// of file reads per pane per five seconds, all night, to re-learn the
    /// answer already in hand.
    #[test]
    fn a_healthy_agent_left_alone_is_not_looked_up_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "").unwrap();
        append(&path, TURN_STARTED);
        append(&path, TURN_ENDED);

        let pane = PaneJoin {
            preset: Some("claude".to_string()),
            pid: None,
            cwd: "/tmp".to_string(),
            title: String::new(),
        };

        let start = 1_000_000;
        let mut log = PaneLog::new();
        log.tail = Some((Tail::new(path.clone()), LogFormat::Claude));
        log.attempted_at = start;
        log.turn = Some(LogTurn { running: false, failed: false, last_event_at: start });

        // Two hours of nothing, with the filesystem busy the whole time —
        // other panes' logs are churning, which is what would otherwise open
        // the gate. `never_joined` panics if anything asks.
        for tick in 1..=7_200 {
            let (advanced, events) = advance_log(log, &pane, start + tick * 1_000, true, false, never_joined);
            log = advanced;
            // Tick one consumes the finished session already in the file — a
            // turn start and its end, which is what the turn was set to say
            // above. Every tick after it reads an empty file.
            assert!(tick == 1 || events.is_empty(), "at {tick}s: {events:?}");
        }
        // The clock reads the FIRST tick, not the last. A file this small is
        // attached to at its start (see `Tail::new`), so tick one consumes the
        // finished session already in it — which says exactly what the turn was
        // set to say above — and the 7,199 ticks after it read nothing at all.
        assert_eq!(
            log.turn,
            Some(LogTurn { running: false, failed: false, last_event_at: start + 1_000 })
        );

        // The contradiction is the whole of the difference: the same silent
        // log, at the same hour, on a pane whose screen says it is working, IS
        // looked up again. A pane waiting on a permission prompt gets the
        // resting answer for the same reason an idle one does — `sample` asks
        // for `Working` specifically, and no agent writes down that it is
        // waiting, so a question left up over lunch is a log that is silent
        // exactly as long as the human is away.
        assert!(!join_looks_dead(log.turn, start, start + 7_200_000, false));
        assert!(join_looks_dead(log.turn, start, start + 7_200_000, true));
    }

    /// An agent Far Cooler has no parser for keeps stage 1 exactly.
    #[test]
    fn an_agent_with_no_parser_is_never_given_one() {
        assert_eq!(LogFormat::of("claude"), Some(LogFormat::Claude));
        assert_eq!(LogFormat::of("codex"), Some(LogFormat::Codex));
        assert_eq!(LogFormat::of("cursor"), Some(LogFormat::Cursor));
        // Recognized by the registry, with no session-log parser written for
        // it. Guessing at one of the other three formats would read another
        // agent's grammar into this one's file.
        assert_eq!(LogFormat::of("opencode"), None);
        assert_eq!(LogFormat::of("bash"), None);
        assert_eq!(LogFormat::of(""), None);
        // By prefix, the same way `find_session_log` reads a preset.
        assert_eq!(LogFormat::of("claude-sonnet"), Some(LogFormat::Claude));
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
    fn only_a_blocked_or_finished_agent_is_worth_a_banner() {
        // Working is the NORMAL case. Something that buzzes for the normal case
        // is something people turn off, after which it cannot tell them the
        // thing that mattered — so this is the rule the whole feature rests on.
        //
        // A working agent IS worth a phone now: it produces a notice, and the
        // relay will even start a lock screen card from it. The rule is
        // unchanged because the relay never builds a BANNER out of one — the
        // card goes up and moves silently. So what this test guards is the
        // status, which is what the relay switches on — never `blocked` or
        // `done` for an agent that is merely busy.
        let asking = just_asking("Do you want to create haiku.txt?");
        assert_eq!(
            notification(AgentActivity::Working, "claude", Quoted::default(), false, None).map(|n| n.status),
            Some("working")
        );
        assert!(notification(AgentActivity::Idle, "claude", Quoted::default(), false, None).is_none());
        assert!(notification(AgentActivity::Unspecified, "claude", Quoted::default(), false, None).is_none());
        // Not even with a question in hand: a question is what a card SAYS, not
        // what decides there is one.
        assert_eq!(
            notification(AgentActivity::Working, "claude", asking, false, None).map(|n| n.status),
            Some("working")
        );
        // And not because a turn failed, either. How the last turn ended is the
        // wording of a card, never the reason there is one — a turn that died
        // mid-flight is still a turn in flight, and a phone buzzing about an
        // agent that is busy is the noise this rule exists to refuse.
        assert_eq!(
            notification(AgentActivity::Working, "claude", Quoted::default(), true, None).map(|n| n.status),
            Some("working")
        );
        assert!(notification(AgentActivity::Idle, "claude", Quoted::default(), true, None).is_none());
    }

    /// A working agent gets a CARD, and never a banner.
    ///
    /// The rule that a working agent must not buzz is about alerts, and it
    /// stands. A live card is not an alert: it updates silently, and the whole
    /// point of it is watching a run you already know about.
    #[test]
    fn a_working_agent_gets_a_card_that_says_where_it_is() {
        let notice = notification(
            AgentActivity::Working,
            "claude",
            just_asking("3/7 · Designing test matrix"),
            false,
            None,
        )
        .expect("working now produces a notice");
        assert_eq!(notice.status, "working");
        assert_eq!(notice.subtitle, "3/7 · Designing test matrix");
        // The label alone, with no sentence built around it. The relay never
        // alerts on a working notice, so there is nothing for a sentence to be
        // the subject of — and "claude needs you" on a card for an agent that
        // needs nothing is the exact lie the status split exists to prevent.
        assert_eq!(notice.title, "claude");
    }

    /// The card counts, and this is the only clock it can count from.
    ///
    /// The phone renders `startedAt` as a native timer — no push per tick, and
    /// it keeps running with the device off the network. Nothing on the card
    /// can derive that number: the daemon is the only thing that knows when the
    /// turn began. So a notice that drops it is a card with no elapsed time on
    /// it, which is not a broken build or a failed push but a feature that
    /// quietly does nothing, in the one place where nobody is looking at logs.
    #[test]
    fn a_card_carries_the_turn_it_is_about_and_never_invents_one() {
        // A real millisecond stamp, the shape `Observed::turn_started_at`
        // holds. Seconds would be a plausible-looking number that the phone
        // reads as a different decade — see the decoder in
        // `AgentActivityAttributes`, which tells the two apart by magnitude.
        const STARTED: i64 = 1_755_000_000_000;

        let working = notification(
            AgentActivity::Working,
            "claude",
            just_asking("3/7 · Designing"),
            false,
            Some(STARTED),
        )
        .expect("working");
        assert_eq!(working.started_at, Some(STARTED), "the card counts the turn, not the push");

        // Blocked matters most: it is the only tier the relay ever STARTS a
        // card from, and attributes are fixed for an activity's life, so a
        // clock missing here can never be sent again for that card.
        let blocked = notification(AgentActivity::Blocked, "claude", Quoted::default(), false, Some(STARTED))
            .expect("blocked");
        assert_eq!(blocked.started_at, Some(STARTED));

        // And between turns, nothing — never a zero. `turn_started_at` is
        // `None` when no request is running, and zero is a decodable instant:
        // a card given one counts up from January 1970 on the lock screen,
        // which is worse than the missing timer this test exists to prevent.
        let quiet = notification(AgentActivity::Working, "claude", just_asking("waiting"), false, None)
            .expect("working");
        assert_eq!(quiet.started_at, None, "no turn is running, so there is no clock to show");
    }

    /// A tier change is never withheld. Those are the pushes the feature is for.
    #[test]
    fn a_state_change_is_never_throttled() {
        assert!(should_refresh_card(None, 1_000));
        assert!(should_refresh_card(Some(1_000), 1_000 + CARD_REFRESH_MS));
    }

    /// A line that changes three times a second is not three pushes.
    #[test]
    fn a_line_that_moves_faster_than_it_reads_is_coalesced() {
        assert!(!should_refresh_card(Some(1_000), 1_100));
        assert!(!should_refresh_card(Some(1_000), 1_000 + CARD_REFRESH_MS - 1));
    }

    #[test]
    fn a_blocked_agent_says_what_it_wants() {
        let notice =
            notification(AgentActivity::Blocked, "claude", Quoted::default(), false, None).expect("blocked");
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
    /// Derived on the runner, redacted, and carried on four paths — and until it
    /// reached this subtitle, a phone could only say that SOMETHING was being
    /// asked. Answering from the lock screen needs the question on it.
    #[test]
    fn a_blocked_agent_puts_its_question_on_the_card() {
        let notice = notification(
            AgentActivity::Blocked,
            "claude",
            just_asking("Do you want to create haiku.txt?"),
            false,
            None,
        )
        .expect("blocked");
        assert_eq!(notice.title, "claude needs you");
        assert_eq!(notice.subtitle, "Do you want to create haiku.txt?");
        assert_eq!(notice.status, "blocked");

        // A screen too garbled to yield one falls back rather than showing a
        // blank second line — see `registry.blocked_question`, which returns
        // None for a trust gate whose '?' is mid-line.
        for absent in [None, Some(""), Some("   ")] {
            let quoted = Quoted { workspace: "add-auth", question: absent, said: None };
            let notice = notification(AgentActivity::Blocked, "claude", quoted, false, None)
                .expect("blocked");
            assert_eq!(notice.subtitle, "add-auth — Waiting for your answer", "{absent:?}");
        }
    }

    /// A finished agent's second line is its own account of the turn, and
    /// nothing else — least of all the question it has just finished
    /// answering.
    ///
    /// Renamed from `a_finished_agent_needs_no_second_line`, which stopped
    /// being true when the summary arrived: it now has one whenever the agent
    /// said anything. What survived the rename is the half that still holds,
    /// and it is the more important half.
    #[test]
    fn a_finished_agent_does_not_inherit_the_question_it_answered() {
        let notice = notification(AgentActivity::Done, "claude", Quoted::default(), false, None).expect("done");
        assert_eq!(notice.title, "claude finished");
        // And what takes the card back down. An empty subtitle is fine; an
        // empty status would leave a live card up on the lock screen until the
        // system expired it hours later.
        assert_eq!(notice.status, "done");

        // A question left over from the prompt this agent has just finished
        // answering must not ride along: "claude finished / Do you want to
        // create haiku.txt?" reads as a question still open.
        let notice = notification(
            AgentActivity::Done,
            "claude",
            just_asking("Do you want to create haiku.txt?"),
            false,
            None,
        )
        .expect("done");
        assert!(notice.subtitle.is_empty(), "{}", notice.subtitle);
    }

    /// A turn that DIED must not reach the phone as "finished".
    ///
    /// The same finding as the sidebar's `✓`, one surface further out: the
    /// notification is often the only thing a person sees, and telling them an
    /// agent finished when its turn came back an error is how they learn the
    /// card cannot be trusted. The wording is the failed command's, so a
    /// person reads one vocabulary for both.
    #[test]
    fn a_turn_that_died_says_so_rather_than_saying_it_finished() {
        let notice = notification(AgentActivity::Done, "cursor", Quoted::default(), true, None).expect("done");
        assert_eq!(notice.title, "cursor failed");
        assert_eq!(notice.subtitle, "Its last turn didn’t finish");
        // Still "done": there is no live card to hold open for a turn that is
        // over, however it ended — the same reasoning `exit_notice` follows.
        assert_eq!(notice.status, "done");
        // And the fact the status cannot carry, carried beside it. The phone's
        // notification service extension has a status word and nothing else to
        // pick a mark from, so without this a dead agent wears the finished
        // one — `✓` on a lock screen widget, for an agent that died, until the
        // app next polls.
        assert!(notice.failed);
    }

    /// A turn that FINISHED must not arrive looking like one that died.
    ///
    /// The other direction of the same field, which is worth pinning
    /// separately: a `failed` stuck true would put `✗` on every completed run,
    /// and a mark that is always wrong in one direction is noticed; one that is
    /// wrong in both is simply not believed.
    #[test]
    fn a_finished_turn_is_not_reported_as_a_failed_one() {
        let done = notification(AgentActivity::Done, "claude", Quoted::default(), false, None).expect("done");
        assert!(!done.failed);
        // Neither tier has ended at all, so neither can have ended badly.
        let blocked = notification(AgentActivity::Blocked, "claude", Quoted::default(), true, None);
        assert_eq!(blocked.map(|n| n.failed), Some(false));
        let working = notification(AgentActivity::Working, "claude", just_asking("x"), true, None);
        assert_eq!(working.map(|n| n.failed), Some(false));
    }

    #[test]
    fn a_failed_exit_names_the_code_and_the_workspace() {
        let notice = exit_notice("cargo build", "add-auth", Some(101), None);
        assert_eq!(notice.title, "cargo build failed");
        // Which build broke, as well as how. `cargo build failed / Exit code
        // 101` at three in the morning is a fact about a fleet and not about
        // any pane in it — the same complaint the agent notices answer, one
        // step over, and the same shape the Mac's local banner already used.
        assert_eq!(notice.subtitle, "add-auth — Exit code 101");
        // Not "blocked": nothing is waiting on an answer, so there is no live
        // card to raise. Reusing "done" is what keeps the relay's vocabulary at
        // two words instead of a third it was never taught.
        assert_eq!(notice.status, "done");
        // Which is exactly why the failure travels beside the status: a command
        // that exited badly has to reach the phone as `✗`, and `"done"` alone
        // cannot say so.
        assert!(notice.failed);
    }

    #[test]
    fn a_signal_names_the_signal_not_the_code() {
        // A process killed by a signal has no exit code at all, so the
        // sentence has to come from the signal or say nothing useful.
        let notice = exit_notice("cargo build", "add-auth", None, Some(9));
        assert_eq!(notice.subtitle, "add-auth — Stopped by signal 9");
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

    /// What the runner is willing to say about a card it cannot see.
    ///
    /// The relay holds the cards and this holds the only half of the answer a
    /// runner has. Three outcomes, and the third is the one that matters most:
    /// "could not tell" must never be spelled as "no", because a card taken
    /// down on a failed screen read is the one notification this product exists
    /// for, deleted, with nothing able to put it back — `Blocked` to `Blocked`
    /// is not a transition and pushes nothing.
    #[test]
    fn a_card_is_only_retired_on_evidence() {
        use AgentActivity::{Blocked, Done, Idle, Unknown, Unspecified, Working};
        use TerminalState::{Error, Exited, Lost, Running, Starting};

        // A turn is in progress: the card is exactly right and must not move.
        assert_eq!(card_still_earned(Running, Working), Some(true));
        assert_eq!(card_still_earned(Running, Blocked), Some(true));

        // No turn behind it. `None` is the case that has no notice of its own:
        // somebody quit claude and got their shell back, leaving a card reading
        // "Working" for a process that no longer exists.
        assert_eq!(card_still_earned(Running, Idle), Some(false));
        assert_eq!(card_still_earned(Running, Done), Some(false));
        assert_eq!(card_still_earned(Running, AgentActivity::None), Some(false));

        // The pane's state outranks the screen, because a dead pane's screen
        // read is not a reading of anything. Blocked here is what the last live
        // sample made of it, and it does not save the card.
        assert_eq!(card_still_earned(Exited, Blocked), Some(false));
        assert_eq!(card_still_earned(Lost, Blocked), Some(false));
        assert_eq!(card_still_earned(Error, Working), Some(false));

        // And the abstentions. `TerminalState::Unknown` is tmux not answering —
        // one stalled server would otherwise retire every card on the runner in
        // a single tick — and the two unreadable activities are a failed
        // `capture-pane` and a screen matching nothing recognized.
        assert_eq!(card_still_earned(TerminalState::Unknown, Blocked), None);
        assert_eq!(card_still_earned(Running, Unspecified), None);
        assert_eq!(card_still_earned(Running, Unknown), None);
        // A terminal whose pane has not appeared yet is judged on its activity
        // like any other, which for a shell that has not started is nothing.
        assert_eq!(card_still_earned(Starting, Unspecified), None);
    }

    /// When a card becomes an orphan, which is the whole of this fix.
    ///
    /// Two of these are the bug as reported: a card that outlived the state
    /// that raised it, on a runner where nothing was stuck at all.
    #[test]
    fn an_orphaned_card_is_found_the_moment_it_is_orphaned() {
        // The daemon restarted. Every terminal is unsettled, nothing has moved
        // — the row reads exactly as it did a second ago — and the card the
        // relay is holding is from a previous life. This is the case that
        // makes every runner update orphan every card that was up.
        assert!(card_orphaned(false, false, None, Some(false)));
        // Same tick, an agent that is genuinely still blocked: tmux owns the
        // pane and the agent kept waiting across the restart. Its card is
        // correct and is left exactly as it is — not ended, and not re-raised
        // either, which would alert twice for one question.
        assert!(!card_orphaned(false, false, None, Some(true)));
        // Same tick again, a pane nobody could read. Nothing is claimed, and
        // the sweep comes round on the next tick — see `card_settled`.
        assert!(!card_orphaned(false, false, None, None));

        // The pane died under a card that was up. A state crossing, which is
        // not an activity change, so nothing else in this loop would fire.
        assert!(card_orphaned(true, true, None, Some(false)));
        // The agent went away inside a living pane: `Working` becomes `None`
        // when somebody quits claude, and there is no notice for that tier.
        assert!(card_orphaned(true, false, Some(AgentActivity::None), Some(false)));

        // A settled terminal with nothing moving is not re-retired every
        // second. A retirement is cheap, but a runner that sent one per idle
        // pane per tick would be a fleet's worth of requests a second, forever.
        assert!(!card_orphaned(true, false, None, Some(false)));

        // And the exception. `Done` carries its own notice, that notice is what
        // ends the card, and it leaves a minute of "Finished" up for whoever
        // picks the phone up because of the alert beside it. Retiring here as
        // well would race that end and win.
        assert!(!card_orphaned(true, true, Some(AgentActivity::Done), Some(false)));
    }

    /// Nobody is looking, so everything is announced exactly as it was.
    ///
    /// The regression guard for the change, not a demonstration of it. The
    /// overwhelming majority of transitions happen with no client watching that
    /// pane — a phone in a pocket, a Mac behind a browser, a fleet of six agents
    /// where five are off screen — and every one of them must reach the owner
    /// the way it did before any of this existed.
    #[test]
    fn an_unwatched_agent_is_announced_exactly_as_before() {
        assert!(matches!(attention(AgentActivity::Done, false), Attention::Announce));
        assert!(matches!(attention(AgentActivity::Blocked, false), Attention::Announce));
        // A turn that DIED is a `Done` like any other here: `attention` decides
        // whether to interrupt, and `notification` decides what the interruption
        // says. Keeping those apart is what stops a failed turn being silently
        // swallowed by a rule about attention.
        assert!(matches!(attention(AgentActivity::Done, false), Attention::Announce));
    }

    /// The bug, as reported: a codex pane open on screen, a question asked, an
    /// answer given, and a phone buzzing about a reply already being read.
    #[test]
    fn an_agent_being_watched_does_not_buzz_the_person_watching_it() {
        // The turn ends under your eyes. Nothing is sent — and the live card
        // still has to come down, because the `done` push was the only thing
        // that ever ended one, and suppressing it without retiring would leave
        // a card reading "Working" over an agent that stopped. That is
        // `5398dba`'s bug reintroduced through a different door.
        assert!(matches!(attention(AgentActivity::Done, true), Attention::Retire));

        // The agent asks a question you are already looking at. Nothing goes
        // out, and nothing comes down either: a blocked agent is MID-TURN, its
        // card is still earned — see `card_still_earned` — and taking it down
        // would remove the one notification this product exists for with
        // nothing able to put it back, since `Blocked -> Blocked` is not a
        // transition and pushes nothing.
        assert!(matches!(attention(AgentActivity::Blocked, true), Attention::Hold));
    }

    /// A claim of attention that nobody has renewed stops being believed.
    ///
    /// The single most important property here, because it is the only one whose
    /// failure is silent: everything else about this feature fails by sending a
    /// notification somebody did not need, and this one fails by a terminal that
    /// never notifies anybody again. A phone that crashed, a Mac suspended with
    /// the lid closed, an SSH link that died mid-poll — none of them get to run
    /// any cleanup, so the claim has to expire on its own.
    #[test]
    fn a_claim_nobody_renews_expires_on_its_own() {
        let at = 1_000_000;
        // Just made, and just under the bound: still watching.
        assert!(watch_is_live(at, at));
        assert!(watch_is_live(at, at + WATCHED_TTL_MS - 1));
        // The client stopped saying so. From here on the pane notifies again,
        // which is the behavior of a runner that has never heard of this
        // feature — the state this must always be able to fall back to.
        assert!(!watch_is_live(at, at + WATCHED_TTL_MS));
        assert!(!watch_is_live(at, at + 60_000));
        // A clock that went BACKWARDS — an NTP correction, a laptop resuming —
        // expires the claim rather than extending it for the length of the
        // jump. Saturating arithmetic on an `i64` of milliseconds is what makes
        // that true rather than an underflow that reads as freshly watched.
        assert!(!watch_is_live(at, at - 60_000));
    }

    /// Three seconds of slack, because that is what a poll is worth.
    ///
    /// Both clients renew on a three-second clock, so the bound has to swallow
    /// a lost round trip or two without deciding that somebody plainly sitting
    /// there watching has gone away. Stated as a test rather than left implicit
    /// in a constant, because shortening the TTL and lengthening a poll are
    /// changes made in different files, months apart, by people who would not
    /// otherwise find out they had just reintroduced the buzz.
    #[test]
    fn the_bound_survives_two_missed_polls() {
        let poll = 3_000;
        assert!(WATCHED_TTL_MS > poll * 3, "a claim must outlive three polls of its own clock");
    }

    /// A client can take back what it said, and nothing else can take it back
    /// for them.
    ///
    /// The reason the map is keyed by client and not by terminal. Keyed by
    /// terminal it could only ever be added to — nothing in it would record who
    /// said so — and a person who clicked from one pane to the next would leave
    /// the first silenced with no way to say otherwise.
    #[test]
    fn a_client_replaces_its_own_claim_and_only_its_own() {
        let mac = "-";
        let phone = "iphone";
        let reading = Uuid::now_v7();
        let next_door = Uuid::now_v7();
        let elsewhere = Uuid::now_v7();
        let now = 1_000_000;
        let mut watched = HashMap::new();

        // A Mac window showing a tiled layout: two panes on screen at once, and
        // a pane beside the focused one is as much on screen as the focused
        // one. Both are watched.
        record_watch(&mut watched, mac, vec![reading, next_door], now);
        assert!(anyone_watching(&watched, reading, now));
        assert!(anyone_watching(&watched, next_door, now));
        assert!(!anyone_watching(&watched, elsewhere, now));

        // The phone, on another pane entirely. Two clients, two claims, and
        // neither disturbs the other.
        record_watch(&mut watched, phone, vec![elsewhere], now);
        assert!(anyone_watching(&watched, reading, now));
        assert!(anyone_watching(&watched, elsewhere, now));

        // The Mac navigates away, to one pane. Its previous claim goes with the
        // call rather than waiting out `WATCHED_TTL_MS` — an agent finishing in
        // `next_door` a second from now is news again, immediately.
        record_watch(&mut watched, mac, vec![reading], now + 1);
        assert!(anyone_watching(&watched, reading, now + 1));
        assert!(!anyone_watching(&watched, next_door, now + 1));
        // And the phone's unrelated claim is untouched by any of it.
        assert!(anyone_watching(&watched, elsewhere, now + 1));

        // The Mac goes to the background and says so: an empty claim, which is
        // the release both apps send rather than simply falling silent.
        record_watch(&mut watched, mac, Vec::new(), now + 2);
        assert!(!anyone_watching(&watched, reading, now + 2));
        assert!(anyone_watching(&watched, elsewhere, now + 2));
    }

    /// A client that crashes cannot leave a terminal silent forever.
    ///
    /// The failure this feature has to be safe against, exercised through the
    /// map rather than through the predicate alone: a phone that is killed by
    /// iOS mid-poll runs no cleanup and sends no release, and the pane it named
    /// has to start notifying again anyway.
    #[test]
    fn a_client_that_vanishes_stops_silencing_the_pane_it_left() {
        let pane = Uuid::now_v7();
        let now = 1_000_000;
        let mut watched = HashMap::new();
        record_watch(&mut watched, "iphone", vec![pane], now);
        assert!(anyone_watching(&watched, pane, now));

        // Suspended. No release, no further calls, nothing to observe from
        // here at all.
        assert!(anyone_watching(&watched, pane, now + WATCHED_TTL_MS - 1));
        assert!(!anyone_watching(&watched, pane, now + WATCHED_TTL_MS));

        // And the row itself is gone the next time anything writes, so a
        // device that never comes back does not keep a slot for the life of the
        // daemon.
        record_watch(&mut watched, "ipad", vec![], now + WATCHED_TTL_MS);
        assert!(!watched.contains_key("iphone"));
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
