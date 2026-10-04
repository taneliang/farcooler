//! Telling the relay that an agent needs its owner.
//!
//! The daemon holds NO user credential and does no sign-in. It has a bearer
//! token the relay issued when a signed-in phone paired this runner, and that
//! token names nothing but "this account". So there is no WorkOS here, no
//! browser to open on a headless Linux box, and no user identity sitting on a
//! server the user does not own.
//!
//! What that buys, concretely: a stolen daemon token can notify the phone of
//! the person it was stolen from and can do nothing else — it cannot enumerate
//! devices, cannot name a destination, and is revocable from the app without
//! touching this runner.

use std::path::{Path, PathBuf};

/// Where a paired runner keeps its token.
///
/// Beside the database rather than in it: a token is a credential, not durable
/// intent, and keeping it out of the schema means a database copied for support
/// or debugging does not carry the ability to notify its owner.
pub fn config_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("push.json")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pairing {
    /// Overridable so a self-hosted relay is a setting rather than a fork.
    #[serde(default = "default_relay")]
    pub relay: String,
    pub token: String,
}

/// The relay this channel's clients talk to.
///
/// One relay per channel, the same partition the runtime directory and the
/// binary name follow. They are separate deployments with separate databases
/// and separate WorkOS environments, so a beta daemon cannot notify a release
/// app — which is correct, because it could not reach that app's machine
/// either.
///
/// Release's URL is unchanged and must stay that way: it is compiled into
/// binaries in the App Store, which cannot be told a new one for days.
pub fn default_relay() -> String {
    use farcooler_protocol::Channel;
    match farcooler_protocol::CHANNEL {
        Channel::Stable => "https://relay.farcooler.com",
        Channel::Preview => "https://relay-preview.farcooler.com",
        Channel::Canary => "https://relay-canary.farcooler.com",
        Channel::Local => "https://relay-local.farcooler.com",
    }
    .to_string()
}

impl Pairing {
    /// Load from wherever this daemon keeps its runtime state.
    pub fn load() -> Option<Self> {
        Self::load_in(&crate::paths::runtime_dir().ok()?)
    }

    pub fn load_in(runtime_dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(config_path(runtime_dir)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_in(&self, runtime_dir: &Path) -> std::io::Result<()> {
        use std::io::Write;

        let path = config_path(runtime_dir);
        // Not `unwrap_or_default()`. That wrote an EMPTY file, returned Ok, and
        // let the CLI print "paired" — after which `load_in` parses nothing,
        // `push status` says "not paired", and the runner is silently mute.
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;

        // Owner-only from the moment it exists. Setting the mode after writing
        // leaves a window in which a bearer token sits on disk with whatever
        // the umask says — usually world-readable — and the fix costs one
        // `OpenOptions` call.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)?.write_all(text.as_bytes())
    }

    pub fn forget_in(runtime_dir: &Path) {
        let _ = std::fs::remove_file(config_path(runtime_dir));
    }
}

/// What the relay is told.
///
/// A title, a line under it, which terminal to open, and the two facts a Live
/// Activity needs.
///
/// **One line at a time, and never the transcript itself.** `subtitle` is one
/// composed line — the agent's question while it is blocked, the composed
/// signal rung while it works, the last thing it said once it is done, each
/// under the name of the worktree it happened in — derived from the transcript
/// and the tool stream, redacted at `farcooler_core::feed`'s single choke
/// point, and cut there too. Never the conversation behind it, never a command
/// line, never raw output.
///
/// Two widths, because two surfaces: `feed::WIDTH`, forty characters, for the
/// rung a sidebar row and a live card draw, and `feed::SAID_WIDTH`, a hundred
/// and twenty, for the sentence a `done` banner quotes — a banner is the width
/// of a phone and about two lines tall, where a row is neither. Both are cut
/// on the host, and both are cut after redaction rather than before it.
///
/// The REPETITION is the part worth stating outright, because it is new and it
/// is not what "a notification" sounds like. A `working` notice moves the live
/// card for the whole length of a run — one line at most every
/// `watch::CARD_REFRESH_MS`, ten seconds, for as long as the agent works —
/// where `blocked` and `done` cross perhaps twice between them. So the relay
/// sees a slow drip of an agent's headline rather than two lines and silence.
///
/// A wider exposure than it was, and bounded by what is kept, which is a
/// card's worth per pane for 24 hours: `/v1/notify` stores the runner's
/// `version`, and on each pane's roster row (`live_activities`) the columns
/// of migrations 0008 (`label`, `machine`, `status`, `detail`, the three
/// counts and `trace`, with its `started_at` and `status_since`), 0011
/// (`workspace`) and 0014 (the open ask's `ask_id`, `ask_tool` and
/// `ask_until`). The relay deletes a row 24 hours after it last moved, and
/// clears the ask columns as soon as the ask ends or its hold runs out.
/// `install_cards` holds delivery metadata, and the worker refuses to log a
/// body at all. The relay is a delivery service; what it keeps is the card it
/// has to redraw, and no more.
#[derive(Debug, Default, serde::Serialize)]
struct Notification<'a> {
    /// What this notice is: absent for an agent notice, `"decision"` for a
    /// task entering Needs Decision, `"count"` for the runner's needs-you
    /// count moving with nothing else to say, `"ask"` for a blocked pane's
    /// held ask being offered or ending with the pane still blocked. Spec §7;
    /// ov-57's T0 contract, C2.
    ///
    /// Absent rather than `"agent"` on an agent notice, so a relay older than
    /// the field reads every agent notice exactly as it always did.
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'a str>,
    /// Absent on a count notice, which says nothing to a person. Present on
    /// every other kind: the relay refuses an alert with no title.
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle: Option<&'a str>,
    /// `"working"`, `"blocked"` or `"done"` — the same three `Notice` in
    /// `watch.rs` states, and nothing else.
    ///
    /// What the phone does with each is not one axis but two. `blocked` and
    /// `done` raise or dismiss the live card on the lock screen and alert;
    /// `working` only ever moves a card — starting one silently when there is
    /// none, updating it in place when there is — so an agent can be reported
    /// for a whole run without buzzing anybody.
    ///
    /// Told rather than worked out, because the only other place this fact
    /// exists in the payload is inside `title`, which is a human sentence. A
    /// relay that read "needs you" out of one would be a second copy of the
    /// same rule, in another language, breaking the day the copy changed.
    ///
    /// A daemon too old to send this omits the field and the relay falls back
    /// to a plain notification — so an empty or invented status is worse than
    /// none, and this is never either.
    ///
    /// An agent notice's alone: a decision and a count carry none.
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'a str>,
    /// What the agent is called, on its own: the resolved agent name — "claude",
    /// "codex", a harness preset — and not the worktree, which does not exist at
    /// the call site. Deliberately the same string already interpolated into
    /// `title`, so the live card and the notification under it cannot disagree
    /// about what they are naming.
    ///
    /// It is already inside `title` as a fragment, and that is exactly the
    /// problem: the live card puts the name and the status in separate places
    /// on the lock screen, and neither can be cut back out of a sentence.
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
    /// Whether the turn this `"done"` is about ENDED BADLY.
    ///
    /// A second field rather than a fourth status, because it answers a
    /// different question. `status` tells the relay what to do with the lock
    /// screen card, and a failed turn is over exactly as a finished one is —
    /// see `watch::exit_notice`. This tells the phone which MARK to draw, which
    /// nothing downstream can work out for itself: the only other place the
    /// fact exists in this payload is inside `title`, as the word "failed" in a
    /// human sentence, and reading a verb back out of one is the second copy of
    /// a rule that `status` already exists to avoid.
    ///
    /// It is here because of what the notification service extension draws.
    /// That extension has a status word and nothing else, `feed::glyph`'s `✗`
    /// is unreachable from `"done"` alone, and `accessoryCircular` draws ONLY
    /// the glyph — so without this an agent whose turn died wore `✓` on a lock
    /// screen widget until the app next polled.
    ///
    /// Always sent, including as `false`. A relay or a phone too old to know
    /// the field ignores it and behaves exactly as it did; skipping it when
    /// false would save nothing and make "absent" and "false" two spellings a
    /// reader has to tell apart. On every AGENT notice, that is: a decision and
    /// a count are about no turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    failed: Option<bool>,
    /// The pane an agent notice is about. Never on a decision or a count: the
    /// relay keys its roster rows by `(account, terminal)`, and a task is not
    /// a row.
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal: Option<&'a str>,
    /// The task a decision is about, by its key (`bil-7`), so a tap can open
    /// it.
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<&'a str>,
    /// The workspace this notice's agent or task belongs to, by name. Absent
    /// for none, never `""`.
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<&'a str>,
    /// This runner's needs-you count when the notice was sent, from the same
    /// `needs_you::assemble` its `needs_you.list` answers with. The relay
    /// overwrites this machine's count with it and sums machines.
    ///
    /// camelCase on the wire, as `startedAt` is and for its reason: a
    /// `needs_you` key reaches the relay as `undefined`.
    #[serde(rename = "needsYou", skip_serializing_if = "Option::is_none")]
    needs_you: Option<u32>,
    /// How many of this runner's worktrees have a diff that moved since anyone
    /// reviewed it: the number the app's review count sums over runners
    /// (`reviewsWaiting`), taken from the same `review_ops::inbox` rows. It
    /// rides every notice that carries `needsYou`, and the relay overwrites
    /// this runner's last one with it (migration 0019).
    ///
    /// Absent, never `0`, when the inbox could not be read: the relay then
    /// keeps what it had. A relay older than the field ignores it.
    #[serde(skip_serializing_if = "Option::is_none")]
    reviews: Option<u32>,
    /// This runner's install id, its `install-id` file: the one name that is
    /// stable across re-pairing and distinct per runner, where the label the
    /// app pairs under is "This Mac" on every Mac. The relay keys this runner's
    /// needs-you count by it (migration 0013), so a re-pair replaces the old
    /// token's count and two Macs never share one.
    ///
    /// An opaque UUIDv7 that names no person, path or host. A relay older than
    /// the field ignores it.
    #[serde(skip_serializing_if = "Option::is_none")]
    install: Option<&'a str>,
    /// This runner's id as a phone knows it, `Host.runner_id`, derived from
    /// the install id as the daemon derives it for every client. On a
    /// decision and a task notice, since a task key is only unique on its own
    /// runner, and this is what lets a phone with two runners open the right
    /// one (ov-72); and on an agent notice, so a tap waits for the runner its
    /// pane is on (ov-183). The beat's `runner` is the same value. Absent
    /// with no install id.
    #[serde(skip_serializing_if = "Option::is_none")]
    runner: Option<String>,
    /// The claude permission ask held open on this pane, so the lock screen's
    /// card can answer it with the app suspended (ov-57 T0 C1, C2).
    ///
    /// Only on a `blocked` agent notice and on a `kind:"ask"` notice. Absent
    /// means "no ask open now", on both; never `null`. It carries the ask's
    /// id, claude's tool name and when the hold ends, and never an option
    /// name, `tool_input` or a command line: for Bash the allow option's name
    /// IS the command line.
    #[serde(skip_serializing_if = "Option::is_none")]
    ask: Option<&'a WireAsk>,
    /// What this runner is running, so the devices screen can show which of
    /// someone's runners is behind without them going to each one to look.
    ///
    /// Sent here rather than on a route of its own because a daemon that
    /// notifies is a daemon that is running, which is exactly when the answer
    /// is worth recording — and it costs nothing on a request already being
    /// made.
    version: &'a str,
    /// When the turn began, in Unix MILLISECONDS, so the live card can run its
    /// own clock. `None` between turns.
    ///
    /// A timestamp and nothing else. It says WHEN a turn started, never what it
    /// is about, so it does not widen what a relay could leak — and it buys the
    /// one thing a card cannot compute for itself: how long this has been going
    /// on. The phone renders it as a native timer, which needs no push per tick
    /// and keeps counting with the device off the network entirely.
    ///
    /// Renamed on the wire because the other end is not Rust. The relay reads
    /// `body.startedAt` and copies it into the activity's attributes; a
    /// `started_at` key arrives there as `undefined`, nothing errors, and the
    /// card simply comes up with no timer — the exact silent nothing this field
    /// exists to fix.
    ///
    /// A NUMBER, which serde gives for free here and which is the whole
    /// contract: the phone's decoder in `AgentActivityAttributes` tells seconds
    /// from milliseconds apart by magnitude and reads the field with `try?`, so
    /// a value sent as a string costs the timer without costing the card. That
    /// is a failure nobody would report.
    ///
    /// Skipped when absent rather than sent as null or zero. Zero is a perfectly
    /// decodable instant, and a card counting up from January 1970 is worse than
    /// a card with no clock on it at all.
    #[serde(rename = "startedAt", skip_serializing_if = "Option::is_none")]
    started_at: Option<i64>,
    /// The worktree's diff against its base, and the commits inside this
    /// agent's trace window.
    ///
    /// **The first numbers this payload has ever carried**, and they are here
    /// because the lock screen's card grew a row per agent. A row reads
    /// `auth-refactor  force-push?  +142 −37  4 commits`, and none of that
    /// reached the relay before: `title` and `subtitle` are sentences, and a
    /// relay parsing `+142` back out of one would be the second copy of a rule
    /// that `status` and `failed` already exist to avoid.
    ///
    /// A fleet spans several runners, each with its own daemon, so no daemon
    /// can see the whole fleet and none can compose the card's header. The
    /// relay is the only place that sees every runner's notices for one
    /// account. These fields are what it accumulates.
    ///
    /// Not content, on the same test everything else here passes: a count of
    /// changed lines says how MUCH happened and nothing about what. It is
    /// strictly less than the composed line `subtitle` already carries.
    ///
    /// **Skipped when absent, never sent as zero.** A worktree nobody has
    /// probed yet and one with no base to compare against have both said
    /// nothing, and a card drawing `+0 −0` over either would be reporting a
    /// measurement that was never made — see `review::Counts`, which has three
    /// answers rather than a number with zero standing in for the other two.
    /// The relay stores an absent count as NULL and a row draws no numbers.
    #[serde(skip_serializing_if = "Option::is_none")]
    insertions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deletions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commits: Option<u32>,
    /// The thirteen buckets under the row, base64 of the wire's 66 bytes.
    ///
    /// Base64 rather than an array of 66 numbers because this one ends up on an
    /// APNs payload with a hard 4KB ceiling and several rows to fit inside it:
    /// the bytes encode to 88 characters, where `[12,0,7,...]` is upwards of
    /// two hundred. `farcooler_core::base64` is the encoder and the relay's
    /// column holds the string it produces, so nothing between here and the
    /// widget has to decode it.
    ///
    /// Empty is omitted rather than sent as `""`. A trace with nothing in it
    /// encodes to no bytes at all — deliberately, so a fleet at rest costs
    /// nothing — and 66 zeroes is a different statement: thirteen quiet
    /// buckets, which the glance spec says are drawn rather than omitted. See
    /// `farcooler_core::trace::Trace::encode`.
    #[serde(skip_serializing_if = "Option::is_none")]
    trace: Option<String>,
    /// Where `trace`'s newest bucket sits on the absolute grid, as a NUMBER:
    /// its index, `now.div_euclid(width)`. See
    /// `farcooler_core::trace::Trace::anchor`.
    ///
    /// **Its own key, never folded into the base64.** The relay stores and
    /// forwards `trace` without decoding it, and it must keep not decoding it —
    /// a relay that parsed the blob would be a third copy of an encoding that
    /// has two ends. So the one number the card's shared axis needs travels
    /// beside the blob, where the relay can carry it as a column.
    ///
    /// Renamed on the wire for `startedAt`'s reason: the other end reads
    /// `body.traceAnchor`, and a `trace_anchor` key would arrive there as
    /// `undefined` with nothing to say so.
    ///
    /// Skipped exactly when `trace` is: an anchor for no bytes anchors nothing.
    #[serde(rename = "traceAnchor", skip_serializing_if = "Option::is_none")]
    trace_anchor: Option<i64>,
    /// A task notice's id (ov-94): `t:<runner id>:<task key>`, which every
    /// platform replaces by, so a newer notice about a task replaces the
    /// older. At most 64 bytes, APNs's limit on a collapse id; see
    /// `watch::task_notice::notice_id`.
    #[serde(rename = "noticeId", skip_serializing_if = "Option::is_none")]
    notice_id: Option<&'a str>,
    /// A task notice's class: `decision`, `review`, `blocked`, `done` or
    /// `new`. Each device hears only the classes it kept on.
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<&'a str>,
    /// A task notice's interruption level: `time-sensitive`, `active` or
    /// `passive`.
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<&'a str>,
    /// A task DECISION's answer options, for the notification's buttons: the
    /// QUESTION note's own, which a person or the manager wrote. Never an
    /// agent's permission ask, whose option names can be a command line (see
    /// `ask`).
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    options: &'a [String],
    /// `false` on an agent notice whose task notice carries the alert: the
    /// relay moves the card and buzzes nobody (ov-94). Absent otherwise, so a
    /// relay too old to know it alerts as it always did.
    #[serde(skip_serializing_if = "Option::is_none")]
    alert: Option<bool>,
}

/// What `hook-ask-` is followed by: a UUID, hyphenated, or anything the
/// relay would take. The relay's rule is `^hook-ask-[0-9A-Za-z-]{1,55}$`.
const ASK_ID_TAIL_MAX: usize = 55;

/// claude's `tool_name`, or no tool. MCP names (`mcp__server__tool`) run
/// long, hence 64. The relay's rule is `^[A-Za-z0-9_.:-]{1,64}$`.
const ASK_TOOL_MAX: usize = 64;

/// A held ask as the relay is told it (ov-57 T0 C1): its id, the tool, and
/// when the hold ends, in Unix milliseconds by this runner's clock.
///
/// Built only through `WireAsk::new`, which applies the contract's bounds, so
/// no body can carry an ask the relay would refuse or a tool it would drop.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WireAsk {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    until: i64,
}

impl WireAsk {
    /// The ask as it may cross, or `None` when its id or its end fails the
    /// contract's rule. A tool that fails its rule costs the tool, not the
    /// ask.
    pub fn new(id: &str, tool: Option<&str>, until: std::time::SystemTime) -> Option<Self> {
        let tail = id.strip_prefix(crate::hook_asks::HOOK_ASK_PREFIX)?;
        let tail_ok = (1..=ASK_ID_TAIL_MAX).contains(&tail.len())
            && tail.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        let until = until.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
        let until = i64::try_from(until).ok().filter(|ms| *ms > 0)?;
        let tool = tool.filter(|t| {
            (1..=ASK_TOOL_MAX).contains(&t.len())
                && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        });
        tail_ok.then(|| Self { id: id.to_string(), tool: tool.map(str::to_string), until })
    }

    /// The open ask on a pane, as it may cross.
    pub fn of(open: &crate::hook_asks::OpenAsk) -> Option<Self> {
        Self::new(&open.id, open.tool.as_deref(), open.until)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

/// What one call to `notify` is about.
///
/// A struct rather than seven positional arguments, and for the same reason
/// `watch::Notice` is one: `title`, `subtitle`, `status`, `label` and
/// `terminal` are all `&str`, so any two of them transposed still compiles and
/// shows up only as a wrong lock screen. Named fields make that a build error
/// instead of a bug report.
///
/// Borrowed throughout. Every field is already owned by the caller's stack for
/// the length of the await, and a payload that allocated five strings per
/// notification would be paying for a copy nothing keeps.
pub struct Outgoing<'a> {
    /// `None` for an agent notice, else `"decision"`, `"count"` or `"ask"`.
    /// See `Notification::kind` for what each carries; `wire_body` is what
    /// enforces it.
    pub kind: Option<&'a str>,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub status: &'a str,
    pub failed: bool,
    pub label: &'a str,
    /// Required when `kind` is absent: an agent notice names its pane.
    pub terminal: Option<&'a str>,
    /// A decision's task key.
    pub task: Option<&'a str>,
    pub workspace: Option<&'a str>,
    pub needs_you: Option<u32>,
    /// Worktrees to review. See `Notification::reviews`.
    pub reviews: Option<u32>,
    /// This runner's install id. See `Notification::install`. Stamped by the
    /// watcher's `deliver` on every notice, so no caller can forget it.
    pub install: Option<&'a str>,
    /// The pane's open ask. Sent on a `blocked` agent notice and on
    /// `kind:"ask"`, and dropped from every other. See `Notification::ask`.
    pub ask: Option<&'a WireAsk>,
    pub started_at: Option<i64>,
    /// What this agent's row on the card draws beside its name. See
    /// `Notification::insertions` for why absent is not zero, and
    /// `watch::CardStats`, which is where all four are sampled together.
    pub insertions: Option<u32>,
    pub deletions: Option<u32>,
    pub commits: Option<u32>,
    /// The wire's 66 trace bytes, or empty for a terminal with no history.
    /// Encoded to base64 at the boundary below rather than by the caller, so
    /// the one place that knows this crosses an HTTP wire is the one that
    /// spells it.
    pub trace: &'a [u8],
    /// `trace`'s newest bucket on the absolute grid, sampled at the same
    /// moment. See `Notification::trace_anchor`.
    pub trace_anchor: Option<i64>,
    /// A task notice's id, class, level and a decision's options. See
    /// `Notification::notice_id` and the three after it.
    pub notice_id: Option<&'a str>,
    pub event: Option<&'a str>,
    pub level: Option<&'a str>,
    pub options: &'a [String],
    /// `false` for an agent notice whose task notice carries the alert. See
    /// `Notification::alert`.
    pub alert: bool,
}

impl Default for Outgoing<'_> {
    fn default() -> Self {
        Outgoing {
            kind: None,
            title: "",
            subtitle: "",
            status: "",
            failed: false,
            label: "",
            terminal: None,
            task: None,
            workspace: None,
            needs_you: None,
            reviews: None,
            install: None,
            ask: None,
            started_at: None,
            insertions: None,
            deletions: None,
            commits: None,
            trace: &[],
            trace_anchor: None,
            notice_id: None,
            event: None,
            level: None,
            options: &[],
            // Every notice alerts as it always did unless it says otherwise.
            alert: true,
        }
    }
}

/// The trace's bytes as the body spells them: base64, or no key for none.
fn wire_trace(trace: &[u8]) -> Option<String> {
    (!trace.is_empty()).then(|| farcooler_core::base64::encode(trace))
}

/// The anchor as the body spells it: only beside the bytes it anchors.
///
/// Whatever the caller handed in. An anchor with no trace would give the relay a
/// column that says where a trace is for a row that has none — and the relay
/// carries a stored anchor forward with its stored trace, so a lone one could
/// end up beside a blob it was never measured with.
fn wire_anchor(trace: &[u8], anchor: Option<i64>) -> Option<i64> {
    anchor.filter(|_| !trace.is_empty())
}

/// What `notify` sends for an `Outgoing`, keyed by its kind: spec §7's contract.
///
/// - An agent notice (no kind) carries everything it always has, plus its
///   workspace, the count and its runner. `None` if it names no terminal:
///   the relay would write a roster row keyed by nothing.
/// - A decision carries its kind, task, workspace, title, subtitle and count,
///   and no terminal, status, label or `failed`.
/// - A count carries its kind and the count, and nothing else.
/// - An ask carries its kind, terminal, count and the ask, or no `ask` key when
///   none is open. `None` if it names no terminal.
/// - The ask rides on a `blocked` agent notice, and on nothing else.
/// - A task notice (ov-94) carries its kind, task, workspace, title,
///   subtitle, count, runner, id, class and level, a decision's options, and
///   no terminal, status, label or ask.
/// - An agent notice carries `alert: false` when its task's notice alerts.
///
/// Every kind carries the runner's install id when the caller has one.
fn wire_body<'a>(o: &Outgoing<'a>) -> Option<Notification<'a>> {
    let shared = Notification {
        kind: o.kind,
        needs_you: o.needs_you,
        reviews: o.reviews,
        install: o.install,
        version: farcooler_protocol::BUILD,
        ..Notification::default()
    };
    Some(match o.kind {
        None => Notification {
            title: Some(o.title),
            subtitle: Some(o.subtitle),
            status: Some(o.status),
            label: Some(o.label),
            failed: Some(o.failed),
            terminal: Some(o.terminal?),
            workspace: o.workspace,
            // So a tap can wait for this runner rather than search every
            // one for the pane (ov-183). Older relays and apps ignore it.
            runner: o.install.map(|id| crate::service::stable_host_id(id).to_string()),
            started_at: o.started_at,
            insertions: o.insertions,
            deletions: o.deletions,
            commits: o.commits,
            trace: wire_trace(o.trace),
            trace_anchor: wire_anchor(o.trace, o.trace_anchor),
            ask: o.ask.filter(|_| o.status == "blocked"),
            alert: (!o.alert).then_some(false),
            ..shared
        },
        Some("ask") => Notification { terminal: Some(o.terminal?), ask: o.ask, ..shared },
        // A decision from a runner older than ov-94, which no runner sends
        // any more (ov-108): kept so the contract fixture for it still builds.
        Some("decision") => Notification {
            title: Some(o.title),
            subtitle: Some(o.subtitle),
            task: o.task,
            workspace: o.workspace,
            runner: o.install.map(|id| crate::service::stable_host_id(id).to_string()),
            notice_id: o.notice_id,
            event: o.event,
            level: o.level,
            options: if o.event == Some("decision") { o.options } else { &[] },
            ..shared
        },
        // A task notice (ov-94): about a task, so like a decision it names
        // no terminal and writes no roster row. Options only on a decision.
        Some("task") => Notification {
            title: Some(o.title),
            subtitle: Some(o.subtitle),
            task: o.task,
            workspace: o.workspace,
            runner: o.install.map(|id| crate::service::stable_host_id(id).to_string()),
            notice_id: o.notice_id,
            event: o.event,
            level: o.level,
            options: if o.event == Some("decision") { o.options } else { &[] },
            ..shared
        },
        Some(_) => shared,
    })
}

/// Send one, or quietly do nothing if this runner was never paired.
///
/// Failure is logged and swallowed on purpose. A push that does not arrive is a
/// missed notification; a push that takes the watcher down with it is every
/// future notification missed as well, plus the fleet.
pub async fn notify(client: &reqwest::Client, pairing: &Pairing, notice: Outgoing<'_>) -> bool {
    let Some(body) = wire_body(&notice) else {
        tracing::warn!(kind = ?notice.kind, "a notice about a pane named no terminal, and was not sent");
        return false;
    };
    let url = format!("{}/v1/notify", pairing.relay.trim_end_matches('/'));
    let result = client.post(&url).bearer_auth(&pairing.token).json(&body).send().await;

    match result {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            tracing::warn!(status = %response.status(), "relay refused a notification");
            false
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not reach the relay");
            false
        }
    }
}

/// The terminals whose live cards this runner can no longer account for.
///
/// A list rather than one id per request, because the case that matters sends
/// many at once: a daemon that has just started sweeps every terminal it can
/// see, and one request per idle pane would be a fleet's worth of round trips
/// in the first second of every runner update.
#[derive(Debug, serde::Serialize)]
struct Retirement<'a> {
    terminals: &'a [String],
}

/// How many terminals one retirement request may name.
///
/// The relay's own `RETIRE_LIMIT`, stated here because the wire contract has two
/// ends and a sweep that quietly lost everything past the bound would lose it
/// permanently: the daemon marks a terminal settled once it has decided about
/// it, so a card dropped on the floor here is a card nothing mentions again.
/// Sending the remainder as a second request costs one round trip on a path
/// nothing waits for.
const RETIRE_BATCH: usize = 100;

/// Ask the relay to take down cards for runs that are over.
///
/// The daemon does not know which cards exist — the relay does, in its
/// `install_cards` table — so this says what the runner knows instead: these
/// terminals have no run behind them any more. What the relay does about a
/// terminal it holds no card for is nothing, which is what makes it safe to
/// name every terminal a sweep is unsure about rather than only the ones a card
/// was pushed for.
///
/// TERMINALS and not cards, and that survived the relay going to one card per
/// app install unchanged, because it was never a statement about cards: this
/// side knows which runs are over and the other side knows what is on the lock
/// screen. What changed is on the far end. There is one card now, it leads with
/// one agent, and at most one id in this list can be the agent it is leading
/// with — so a sweep takes down the card only when the run behind its LEADER is
/// the one that ended. Naming a terminal the card was never about does nothing,
/// which is the same answer as before and now matters more: ending on any named
/// id would clear the lock screen of a running agent because a different one
/// stopped.
///
/// Carries NO destination and no content, the same as `notify` and for the same
/// reason: a stolen daemon token can end the cards of the account it was stolen
/// from and nothing else.
///
/// Failure is logged and swallowed, again like `notify`. A retirement that does
/// not arrive leaves a card up until its stale date, which is where it was
/// before this existed; a retirement that took the watcher down with it would
/// cost every future notification as well.
///
/// A relay too old to know this route answers 404, and that is the whole of the
/// compatibility story: the runner has done what it can, the card is bounded by
/// the same stale date it always was, and nobody is buzzed by the attempt.
pub async fn retire(client: &reqwest::Client, pairing: &Pairing, terminals: &[String]) {
    let url = format!("{}/v1/notify/retire", pairing.relay.trim_end_matches('/'));
    for batch in terminals.chunks(RETIRE_BATCH) {
        let result = client
            .post(&url)
            .bearer_auth(&pairing.token)
            .json(&Retirement { terminals: batch })
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                tracing::warn!(status = %response.status(), "relay refused a card retirement")
            }
            Err(e) => tracing::warn!(error = %e, "could not reach the relay"),
        }
    }
}

/// How often a paired runner tells the relay it's alive.
///
/// Five minutes: the phone's widget reads the relay about every twenty to
/// thirty minutes at best (WidgetKit's reload budget), so beating faster buys
/// nothing it can see, and each beat is a D1 write. The phone calls a runner
/// quiet after two missed beats and five minutes' slack
/// (`RunnerPulse.quietAfter` in AgentKit), so this number is also the promise
/// the phone judges silence by — sent with every beat rather than assumed.
/// See docs/superpowers/specs/2026-09-30-runner-heartbeat-design.md.
pub const BEAT_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// What a heartbeat carries: that this runner is alive, which runner it is,
/// what it's called, what it runs, and how often to expect the next one.
/// Nothing else — no count, no agent, no card.
#[derive(Debug, serde::Serialize)]
struct Beat<'a> {
    #[serde(rename = "beatEvery")]
    beat_every: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    install: Option<&'a str>,
    /// This runner's id as a phone knows it, `Host.runner_id`: derived from
    /// the install id, as the daemon derives it for every client. The relay
    /// keeps only a per-account hash of it, and a watch hashes the id its
    /// phone learned to match, so a quiet runner's agents are the ones it
    /// stops vouching for (ov-71).
    #[serde(skip_serializing_if = "Option::is_none")]
    runner: Option<String>,
    name: &'a str,
    version: &'a str,
}

fn beat_body(install: Option<&str>) -> Beat<'_> {
    Beat {
        beat_every: BEAT_EVERY.as_secs(),
        install,
        runner: install.map(|id| crate::service::stable_host_id(id).to_string()),
        name: runner_name(),
        version: farcooler_protocol::BUILD,
    }
}

/// What this runner calls itself on a phone: the computer's name on a Mac
/// (what System Settings ▸ Sharing shows, and what the Mac app labels this
/// device with), and the short hostname elsewhere. At most 64 characters.
///
/// Sent on every beat because the pairing label names nothing on a phone:
/// the Mac pairs its own runner as "This Mac". Read once: it changes only
/// when someone renames the computer, and a restart picks that up.
pub fn runner_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let name = computer_name().unwrap_or_else(|| short_host(&crate::hostname()).to_string());
        let name = if name.is_empty() { "Runner".to_string() } else { name };
        name.chars().take(64).collect()
    })
}

/// The Mac's computer name, or `None` anywhere else or when `scutil` can't
/// say. A process spawn, once per daemon.
fn computer_name() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output().ok()?;
    let name = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (output.status.success() && !name.is_empty()).then_some(name)
}

/// A hostname cut at its first dot: `studio.local` is `studio`.
fn short_host(host: &str) -> &str {
    host.split('.').next().unwrap_or(host)
}

/// Tell the relay this runner is being unpaired on purpose, so the phone's
/// widget drops it rather than calling it lost. `true` once it has landed.
///
/// A short timeout of its own: this runs from `push forget`, where a person
/// is waiting, and the local unpair must not wait on an unreachable relay.
///
/// `install` is this runner's install id, which the relay uses to withdraw
/// every pairing of it, including a stale row under an older token, and
/// including this token's own when it never beat (ov-77). Absent, the relay
/// withdraws this token's row alone, as it did before.
pub async fn withdraw(pairing: &Pairing, install: Option<&str>) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build() else {
        return false;
    };
    let url = format!("{}/v1/heartbeat", pairing.relay.trim_end_matches('/'));
    let body = withdraw_body(install);
    match client.post(&url).bearer_auth(&pairing.token).json(&body).send().await {
        Ok(response) => response.status().is_success(),
        Err(e) => {
            tracing::warn!(error = %e, "could not reach the relay to withdraw this runner");
            false
        }
    }
}

/// The body of a withdrawal: the word, and the install id when there is one.
/// Never `"install": null`, which an older relay would read as a value.
fn withdraw_body(install: Option<&str>) -> serde_json::Value {
    match install {
        Some(id) => serde_json::json!({ "withdrawn": true, "install": id }),
        None => serde_json::json!({ "withdrawn": true }),
    }
}

/// Unpair: withdraw from the relay if it answers, then forget the pairing
/// whatever it said. `true` when the relay heard the withdrawal.
///
/// When it didn't, the runner still stops beating, and the relay drops it
/// from the phone's pulse a day after its last beat; until then the widget
/// may say it lost touch with it.
pub async fn forget_and_withdraw(runtime_dir: &Path) -> bool {
    let withdrawn = match Pairing::load_in(runtime_dir) {
        Some(pairing) => {
            // Read, never created: a runner with no install id has nothing to
            // name, and `forget` must not mint one.
            let install = std::fs::read_to_string(runtime_dir.join("install-id"))
                .ok()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty());
            withdraw(&pairing, install.as_deref()).await
        }
        None => false,
    };
    Pairing::forget_in(runtime_dir);
    withdrawn
}

/// Tell the relay this runner is alive.
///
/// Logged and swallowed, like `notify`: a beat that doesn't land costs the
/// phone's widget one "lost touch" it didn't need, and a beat that took the
/// watcher down would cost every notice after it. A relay too old for the
/// route answers 404, which is logged at debug: it would otherwise be a
/// warning every five minutes for as long as that relay runs.
pub async fn heartbeat(client: &reqwest::Client, pairing: &Pairing, install: Option<&str>) -> bool {
    let url = format!("{}/v1/heartbeat", pairing.relay.trim_end_matches('/'));
    let result = client.post(&url).bearer_auth(&pairing.token).json(&beat_body(install)).send().await;
    match result {
        Ok(response) if response.status().is_success() => true,
        Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
            tracing::debug!("the relay has no heartbeat route");
            false
        }
        Ok(response) => {
            tracing::warn!(status = %response.status(), "relay refused a heartbeat");
            false
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not reach the relay");
            false
        }
    }
}

#[cfg(test)]
mod review_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_beat_carries_its_promise_its_runner_its_name_and_nothing_else() {
        // Three programs read this body. The relay reads `beatEvery` in
        // seconds and clamps it; a rename or a unit change here is a runner
        // the phone judges by the wrong clock.
        let sent = serde_json::to_value(beat_body(Some("0190-abc"))).expect("serialize");
        assert_eq!(sent["beatEvery"], 300, "{sent}");
        assert_eq!(sent["install"], "0190-abc", "{sent}");
        assert_eq!(sent["version"], farcooler_protocol::BUILD, "{sent}");
        // The name the phone says "lost touch with" — never "This Mac".
        assert_eq!(sent["name"], runner_name(), "{sent}");
        // Which runner, as its phone knows it (`Host.runner_id`), so a watch
        // can tell which agents are this runner's (ov-71).
        assert_eq!(sent["runner"], crate::service::stable_host_id("0190-abc").to_string(), "{sent}");
        let keys: Vec<&str> = sent.as_object().expect("an object").keys().map(String::as_str).collect();
        assert_eq!(keys.len(), 5, "a beat says nothing else: {sent}");
    }

    /// The runner id a beat carries is the one `Host.runner_id` gives a
    /// phone, pinned as text: the relay keys it and the phone hashes it, and
    /// a change of spelling on one side is a watch that attributes nothing.
    /// `services/relay/test/relay.test.ts` and `RunnerPulseTests` pin the
    /// same literal.
    #[test]
    fn a_beats_runner_is_the_hosts_runner_id() {
        assert_eq!(
            crate::service::stable_host_id("install-with-more-than-sixteen-bytes").to_string(),
            "7537626f-0002-415e-1e11-000d48034210"
        );
    }

    #[test]
    fn a_runner_has_a_name_and_it_is_not_a_domain() {
        let name = runner_name();
        assert!(!name.is_empty(), "a runner with no name would be named by its pairing label");
        assert!(name.chars().count() <= 64, "{name}");
        // A bare hostname is cut at its first dot: `studio.local` is Studio's.
        assert_eq!(short_host("studio.local"), "studio");
        assert_eq!(short_host("studio"), "studio");
    }

    /// Accept one request on `listener`, answer `status`, and hand back its
    /// request line and body.
    async fn one_request(listener: tokio::net::TcpListener, status: &'static str) -> (String, serde_json::Value) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            seen.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&seen).to_string();
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                if seen.len() >= end + 4 + length {
                    let reply = format!("HTTP/1.1 {status}\r\ncontent-length: 2\r\n\r\n{{}}");
                    socket.write_all(reply.as_bytes()).await.unwrap();
                    let line = text.lines().next().unwrap_or_default().to_string();
                    return (line, serde_json::from_slice(&seen[end + 4..end + 4 + length]).unwrap());
                }
            }
            assert!(n != 0, "the relay's socket closed before a whole request");
        }
    }

    #[tokio::test]
    async fn forgetting_a_pairing_withdraws_the_runner_first() {
        // Stop Notifying: the runner is fine, and must leave the phone's pulse
        // rather than read as "lost touch" for a day.
        let dir = tempfile::tempdir().expect("tempdir");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        Pairing { relay: format!("http://{}", listener.local_addr().unwrap()), token: "t".into() }
            .save_in(dir.path())
            .expect("save");
        let relay = tokio::spawn(one_request(listener, "200 OK"));
        assert!(forget_and_withdraw(dir.path()).await, "the withdrawal landed");
        let (line, body) = relay.await.unwrap();
        assert!(line.starts_with("POST /v1/heartbeat "), "{line}");
        assert_eq!(body["withdrawn"], true, "{body}");
        assert!(body.get("install").is_none(), "no install id on disk, none sent: {body}");
        assert!(Pairing::load_in(dir.path()).is_none(), "forget means forgotten");
        assert!(!dir.path().join("install-id").exists(), "forget mints no install id");
    }

    /// A withdrawal names the runner by its install id, so the relay can
    /// clear every pairing of it (ov-77).
    ///
    /// Mutation: `forget_and_withdraw` passing `None`. Red.
    #[tokio::test]
    async fn forgetting_a_pairing_names_the_runner_by_its_install_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("install-id"), "0199-abc\n").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        Pairing { relay: format!("http://{}", listener.local_addr().unwrap()), token: "t".into() }
            .save_in(dir.path())
            .expect("save");
        let relay = tokio::spawn(one_request(listener, "200 OK"));
        assert!(forget_and_withdraw(dir.path()).await);
        let (_, body) = relay.await.unwrap();
        assert_eq!(body["withdrawn"], true, "{body}");
        assert_eq!(body["install"], "0199-abc", "{body}");
    }

    #[tokio::test]
    async fn forgetting_still_forgets_when_the_relay_is_unreachable() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A port nobody listens on: bound, then dropped.
        let addr = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        Pairing { relay: format!("http://{addr}"), token: "t".into() }.save_in(dir.path()).expect("save");
        assert!(!forget_and_withdraw(dir.path()).await, "nothing landed");
        assert!(Pairing::load_in(dir.path()).is_none(), "the local unpair happens anyway");
    }

    #[test]
    fn a_saved_pairing_comes_back_and_can_be_forgotten() {
        // The whole contract of `farcooler push pair|status|forget`, which had
        // no test at all: whether a runner is paired is the difference between
        // being told an agent is stuck and finding out in the morning.
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(Pairing::load_in(dir.path()).is_none(), "nothing is paired yet");

        Pairing { relay: "https://mine.example".into(), token: "t".into() }
            .save_in(dir.path())
            .expect("save");
        let back = Pairing::load_in(dir.path()).expect("saved");
        assert_eq!(back.token, "t");
        assert_eq!(back.relay, "https://mine.example");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(config_path(dir.path()))
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "a bearer token must not be readable by anyone else");
        }

        Pairing::forget_in(dir.path());
        assert!(Pairing::load_in(dir.path()).is_none(), "forget means forgotten");
    }

    #[test]
    fn a_pairing_round_trips_and_defaults_its_relay() {
        // The relay is optional in the file so an existing pairing keeps
        // working when the default moves, and a self-hoster can set it without
        // the daemon needing a different shape of config.
        let parsed: Pairing = serde_json::from_str(r#"{"token":"abc"}"#).expect("parse");
        assert_eq!(parsed.token, "abc");
        assert!(parsed.relay.starts_with("https://"));
        assert_eq!(parsed.relay, default_relay(), "an absent relay is this channel's own");
    }

    /// Build the body `notify` sends, without sending it.
    ///
    /// The request itself is a network call no test can reach through, and the
    /// part worth guarding is not the sending — it is the JSON, which three
    /// separate programs have to agree about.
    fn body(started_at: Option<i64>) -> serde_json::Value {
        stats_body(started_at, None, None, None, &[], None)
    }

    /// The same body with a row's numbers on it. See `Notification::insertions`.
    fn stats_body(
        started_at: Option<i64>,
        insertions: Option<u32>,
        deletions: Option<u32>,
        commits: Option<u32>,
        trace: &[u8],
        trace_anchor: Option<i64>,
    ) -> serde_json::Value {
        serde_json::to_value(
            wire_body(&Outgoing {
                title: "claude",
                subtitle: "3/7 · Designing test matrix",
                status: "working",
                label: "claude",
                terminal: Some("term-1"),
                started_at,
                insertions,
                deletions,
                commits,
                trace,
                trace_anchor,
                ..Outgoing::default()
            })
            .expect("an agent notice with a terminal"),
        )
        .expect("serialize")
    }

    #[test]
    fn a_turn_clock_goes_out_as_a_number_or_not_at_all() {
        // The seam this field exists to close, and the one place it can be
        // checked. The phone renders a native timer from `startedAt` and its
        // decoder takes a NUMBER — a string reads back as nil there, which
        // costs the timer and reports nothing at all. Nothing else in this
        // repository would notice.
        let sent = body(Some(1_755_000_000_000));
        assert_eq!(
            sent["startedAt"],
            serde_json::json!(1_755_000_000_000_i64),
            "the relay reads `startedAt`, not `started_at`"
        );
        assert!(sent["startedAt"].is_number(), "a string decodes to nil on the phone: {sent}");

        // Between turns there is no clock, and no key. A `null` would be
        // harmless and a `0` would not: it is a decodable instant, and the card
        // would come up counting the fifty-odd years since January 1970.
        let quiet = body(None);
        assert!(
            quiet.get("startedAt").is_none(),
            "no turn is running, so the card must be given no clock: {quiet}"
        );
    }

    #[test]
    fn a_row_sends_its_numbers_or_no_key_at_all() {
        // The three counts and the trace are what a card row is made of, and
        // this is the only place on this side that spells their keys. The
        // relay reads them by name and stores what it reads; a rename here
        // would show up as a fleet card with no numbers on it, which is a card
        // that still renders and says less — the kind of failure nobody
        // reports.
        let trace = vec![0x10u8; farcooler_core::trace::ENCODED_LEN];
        let sent = stats_body(None, Some(142), Some(37), Some(4), &trace, Some(5_960_000));
        assert_eq!(sent["insertions"], serde_json::json!(142));
        assert_eq!(sent["deletions"], serde_json::json!(37));
        assert_eq!(sent["commits"], serde_json::json!(4));

        // Base64 and not sixty-six numbers. The card these end up on has a hard
        // 4KB ceiling and several rows to fit inside it — see the field's own
        // note — so this is a size contract, not a formatting preference.
        let encoded = sent["trace"].as_str().expect("a trace is a string");
        assert_eq!(encoded.len(), 88, "66 bytes is 88 base64 characters: {encoded}");
        assert_eq!(
            farcooler_core::base64::decode(encoded).as_deref(),
            Some(trace.as_slice()),
            "the relay stores this string and the widget decodes it; it has to round trip"
        );

        // The anchor is its own key and a NUMBER, beside the blob and never
        // inside it: the relay carries it as a column without decoding base64,
        // and the card's decoder reads it with `try?` as an integer, so a string
        // here would cost the shared axis without costing anything visible.
        assert_eq!(sent["traceAnchor"], serde_json::json!(5_960_000_i64));
        assert!(sent.get("trace_anchor").is_none(), "the relay reads `traceAnchor`: {sent}");

        // Absent is not zero, on every one of them. A worktree nobody has
        // probed and one with no base have both said nothing, and a card
        // drawing `+0 −0` over either would report a measurement nobody made.
        // A trace is the same shape of claim: no bytes means no history seen,
        // where sixty-six zeroes would mean thirteen buckets of observed quiet.
        let quiet = stats_body(None, None, None, None, &[], None);
        for key in ["insertions", "deletions", "commits", "trace", "traceAnchor"] {
            assert!(quiet.get(key).is_none(), "nothing measured `{key}`, so no key: {quiet}");
        }

        // And an anchor is never sent without the bytes it anchors, whatever
        // the caller handed in.
        let lone = stats_body(None, None, None, None, &[], Some(5_960_000));
        assert!(lone.get("traceAnchor").is_none(), "an anchor for no trace: {lone}");
    }

    /// The count's key is the relay's spelling. A `needs_you` key arrives
    /// there as `undefined`, the machine's count never moves, and the lock
    /// screen keeps showing the blocked count as if nothing were decided.
    #[test]
    fn the_body_spells_needs_you_in_camel_case() {
        let agent = serde_json::to_value(
            wire_body(&Outgoing {
                title: "Billing · claude needs you",
                status: "blocked",
                label: "claude",
                terminal: Some("term-1"),
                workspace: Some("Billing"),
                needs_you: Some(3),
                ..Outgoing::default()
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(agent["needsYou"], serde_json::json!(3), "{agent}");
        assert!(agent.get("needs_you").is_none(), "{agent}");
        assert_eq!(agent["workspace"], "Billing");
        assert!(agent.get("kind").is_none(), "an agent notice has no kind: {agent}");

        let count = serde_json::to_value(
            wire_body(&Outgoing { kind: Some("count"), needs_you: Some(0), ..Outgoing::default() }).unwrap(),
        )
        .unwrap();
        let mut keys: Vec<_> = count.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["kind", "needsYou", "version"], "a count is the count and nothing else: {count}");
        assert_eq!(count["needsYou"], serde_json::json!(0), "zero is a count, and is sent");
    }

    /// A decision is about a task, which is not a roster row: no terminal,
    /// and none of an agent notice's status fields.
    #[test]
    fn a_decision_notice_names_its_task_and_no_terminal() {
        let decision = serde_json::to_value(
            wire_body(&Outgoing {
                kind: Some("decision"),
                title: "Billing · bil-7 needs a decision",
                subtitle: "Which PDF library?",
                terminal: Some("term-1"),
                task: Some("bil-7"),
                workspace: Some("Billing"),
                needs_you: Some(2),
                ..Outgoing::default()
            })
            .unwrap(),
        )
        .unwrap();
        for key in ["terminal", "status", "label", "failed"] {
            assert!(decision.get(key).is_none(), "a decision carries no `{key}`: {decision}");
        }
        assert_eq!(decision["task"], "bil-7");
        assert_eq!(decision["kind"], "decision");
        assert_eq!(decision["title"], "Billing · bil-7 needs a decision");
    }

    /// A task notice (ov-94) is about a task: no terminal, none of an agent
    /// notice's fields, its id, class and level, and a decision's options,
    /// which no other class carries, ever.
    #[test]
    fn a_task_notice_names_its_task_its_id_and_never_a_terminal() {
        let options = vec!["pdfkit".to_string(), "pdf.js".to_string()];
        let notice = |event: &'static str| {
            serde_json::to_value(
                wire_body(&Outgoing {
                    kind: Some("task"),
                    title: "ov-90 Wake the agent",
                    subtitle: "Needs your decision · Which?",
                    terminal: Some("term-1"),
                    status: "blocked",
                    task: Some("ov-90"),
                    install: Some("install-1"),
                    notice_id: Some("t:r-1:ov-90"),
                    event: Some(event),
                    level: Some("time-sensitive"),
                    options: &options,
                    ..Outgoing::default()
                })
                .unwrap(),
            )
            .unwrap()
        };
        let decision = notice("decision");
        for key in ["terminal", "status", "label", "failed", "ask", "alert"] {
            assert!(decision.get(key).is_none(), "a task notice carries no `{key}`: {decision}");
        }
        assert_eq!(decision["kind"], "task");
        assert_eq!(decision["task"], "ov-90");
        assert_eq!(decision["noticeId"], "t:r-1:ov-90");
        assert_eq!(decision["event"], "decision");
        assert_eq!(decision["level"], "time-sensitive");
        assert_eq!(decision["options"], serde_json::json!(["pdfkit", "pdf.js"]));
        assert_eq!(decision["runner"], crate::service::stable_host_id("install-1").to_string());
        let review = notice("review");
        assert!(review.get("options").is_none(), "only a decision has buttons: {review}");
    }

    /// An agent working on a task sends its notice for the card alone:
    /// `alert: false`, and every other agent notice says nothing about it.
    #[test]
    fn an_agent_notice_says_alert_false_only_when_its_task_alerts() {
        let agent = |alert| {
            serde_json::to_value(
                wire_body(&Outgoing { terminal: Some("term-1"), status: "blocked", alert, ..Outgoing::default() })
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(agent(false)["alert"], false);
        assert!(agent(true).get("alert").is_none());
    }

    /// A decision names the runner it is on as a phone knows it
    /// (`Host.runner_id`), the beat's own spelling, so a task key on two
    /// runners can be routed (ov-72). So does an agent notice, so a tap can
    /// wait for the runner its pane is on (ov-183). A count doesn't, and none
    /// with no install id.
    #[test]
    fn a_decision_names_its_runner_as_the_phone_knows_it() {
        let sent = |kind: Option<&'static str>, install| {
            serde_json::to_value(
                wire_body(&Outgoing {
                    kind,
                    terminal: Some("term-1"),
                    task: Some("bil-7"),
                    install,
                    ..Outgoing::default()
                })
                .unwrap(),
            )
            .unwrap()
        };
        let install = "install-with-more-than-sixteen-bytes";
        assert_eq!(
            sent(Some("decision"), Some(install))["runner"],
            "7537626f-0002-415e-1e11-000d48034210"
        );
        assert!(sent(Some("decision"), None).get("runner").is_none());
        assert_eq!(sent(None, Some(install))["runner"], "7537626f-0002-415e-1e11-000d48034210");
        assert!(sent(None, None).get("runner").is_none());
        assert!(sent(Some("count"), Some(install)).get("runner").is_none());
    }

    /// Every kind names the runner by its install id, so the relay can tell
    /// two Macs both labeled "This Mac" apart, and a re-paired runner from a
    /// second one. Absent when the caller has none, never `""`.
    #[test]
    fn every_kind_carries_the_install_id() {
        for outgoing in [
            Outgoing { terminal: Some("term-1"), install: Some("0199-abc"), ..Outgoing::default() },
            Outgoing { kind: Some("decision"), title: "bil-7", install: Some("0199-abc"), ..Outgoing::default() },
            Outgoing { kind: Some("count"), needs_you: Some(1), install: Some("0199-abc"), ..Outgoing::default() },
        ] {
            let sent = serde_json::to_value(wire_body(&outgoing).unwrap()).unwrap();
            assert_eq!(sent["install"], "0199-abc", "{sent}");
        }
        let bare = serde_json::to_value(
            wire_body(&Outgoing { kind: Some("count"), needs_you: Some(1), ..Outgoing::default() }).unwrap(),
        )
        .unwrap();
        assert!(bare.get("install").is_none(), "{bare}");
    }

    /// An ask as the contract spells it: 45 bytes of id, a tool, and `until`
    /// in milliseconds.
    fn an_ask() -> WireAsk {
        let until = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_790_551_063_000);
        WireAsk::new("hook-ask-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b", Some("Bash"), until).expect("a valid ask")
    }

    fn agent_body(status: &str, ask: &WireAsk) -> serde_json::Value {
        serde_json::to_value(
            wire_body(&Outgoing {
                title: "claude needs you",
                status,
                label: "claude",
                terminal: Some("term-1"),
                ask: Some(ask),
                ..Outgoing::default()
            })
            .unwrap(),
        )
        .unwrap()
    }

    /// A blocked notice carries the ask open on its pane, so the card can
    /// answer it with the app suspended (ov-57 C2.1).
    #[test]
    fn a_blocked_notice_carries_its_ask() {
        let sent = agent_body("blocked", &an_ask());
        assert_eq!(
            sent["ask"],
            serde_json::json!({
                "id": "hook-ask-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b",
                "tool": "Bash",
                "until": 1_790_551_063_000_i64,
            }),
            "{sent}"
        );
        assert!(sent["ask"]["until"].is_i64(), "milliseconds, as a number: {sent}");
    }

    /// Only `blocked` carries an ask, whatever the caller handed in.
    #[test]
    fn a_working_notice_never_carries_an_ask() {
        for status in ["working", "done"] {
            let sent = agent_body(status, &an_ask());
            assert!(sent.get("ask").is_none(), "{status}: {sent}");
        }
    }

    /// A `kind:"ask"` notice names its pane, and says "none open now" by
    /// having no `ask` key at all, never `"ask": null`.
    #[test]
    fn an_ask_notice_carries_terminal_and_omits_an_absent_ask() {
        let ask = an_ask();
        let told = serde_json::to_value(
            wire_body(&Outgoing {
                kind: Some("ask"),
                title: "never sent",
                status: "blocked",
                terminal: Some("term-1"),
                needs_you: Some(1),
                ask: Some(&ask),
                ..Outgoing::default()
            })
            .unwrap(),
        )
        .unwrap();
        let mut keys: Vec<_> = told.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["ask", "kind", "needsYou", "terminal", "version"], "{told}");
        assert_eq!(told["ask"]["id"], ask.id());

        let none = serde_json::to_value(
            wire_body(&Outgoing { kind: Some("ask"), terminal: Some("term-1"), ..Outgoing::default() }).unwrap(),
        )
        .unwrap();
        assert!(none.get("ask").is_none(), "absent, never null: {none}");
        assert_eq!(none["terminal"], "term-1");
        assert!(
            wire_body(&Outgoing { kind: Some("ask"), ask: Some(&ask), ..Outgoing::default() }).is_none(),
            "an ask notice about no pane is not a body"
        );
    }

    /// With ov-61: an ask notice names the runner as every kind does.
    #[test]
    fn an_ask_notice_carries_the_install_id() {
        let sent = serde_json::to_value(
            wire_body(&Outgoing {
                kind: Some("ask"),
                terminal: Some("term-1"),
                install: Some("0199-abc"),
                ..Outgoing::default()
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(sent["install"], "0199-abc", "{sent}");
    }

    /// The contract's bounds, applied before anything crosses: a bad id or
    /// end costs the ask, a bad tool costs only the tool.
    #[test]
    fn an_ask_crosses_only_inside_its_bounds() {
        let until = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_790_551_063_000);
        let id = "hook-ask-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
        assert!(WireAsk::new("chat-1", None, until).is_none(), "not a hook ask");
        assert!(WireAsk::new("hook-ask-", None, until).is_none(), "no tail");
        assert!(WireAsk::new(&format!("hook-ask-{}", "a".repeat(56)), None, until).is_none(), "65 bytes");
        assert!(WireAsk::new(&format!("hook-ask-{}", "a".repeat(55)), None, until).is_some(), "64 bytes");
        assert!(WireAsk::new("hook-ask-a b", None, until).is_none(), "a space");
        assert!(WireAsk::new(id, None, std::time::UNIX_EPOCH).is_none(), "an end of zero");
        for bad in ["", "Bash ls", "rm -rf /;", &"x".repeat(65)] {
            let ask = WireAsk::new(id, Some(bad), until).expect("a bad tool keeps the ask");
            assert_eq!(ask.tool, None, "{bad:?}");
        }
        for good in ["Bash", "mcp__github__create_issue", "a.b:c-d_e", &"x".repeat(64)] {
            assert_eq!(WireAsk::new(id, Some(good), until).unwrap().tool.as_deref(), Some(good));
        }
    }

    /// And an agent notice that names no pane is not sent at all.
    #[test]
    fn an_agent_notice_without_a_terminal_is_not_a_body() {
        assert!(wire_body(&Outgoing { title: "claude needs you", status: "blocked", ..Outgoing::default() }).is_none());
    }

    #[test]
    fn a_retirement_names_terminals_and_nothing_else() {
        // The other end 400s a body whose `terminals` is not an array, and this
        // is the only place on this side that spells the key — so a rename here
        // would show up as cards that never come down, on a route whose whole
        // job is that they do. It carries no content for the same reason
        // `Notification` explains at length: the relay is a delivery service,
        // and a payload it does not hold is a payload it cannot leak. A
        // terminal id is a UUID this runner minted and says nothing about the
        // work in the pane.
        let terminals = vec!["term-1".to_string(), "term-2".to_string()];
        let sent = serde_json::to_value(Retirement { terminals: &terminals }).expect("serialize");
        assert_eq!(sent, serde_json::json!({ "terminals": ["term-1", "term-2"] }));
    }

    #[test]
    fn each_channel_talks_to_its_own_relay() {
        // One deployment per channel: its own database and its own WorkOS
        // environment. Two channels sharing a relay would mean a preview
        // pairing could notify a stable app — an app that cannot reach the
        // runner that sent it, because they are different binaries at
        // different paths.
        use farcooler_protocol::Channel;
        let urls: Vec<_> = [Channel::Local, Channel::Canary, Channel::Preview, Channel::Stable]
            .iter()
            .map(|c| match c {
                Channel::Stable => "https://relay.farcooler.com",
                Channel::Preview => "https://relay-preview.farcooler.com",
                Channel::Canary => "https://relay-canary.farcooler.com",
                Channel::Local => "https://relay-local.farcooler.com",
            })
            .collect();
        let unique: std::collections::BTreeSet<_> = urls.iter().collect();
        assert_eq!(unique.len(), 4, "two channels cannot share a relay: {urls:?}");

        // Stable's is compiled into App Store binaries that cannot be told a
        // new one for days. It does not move.
        assert_eq!(urls[3], "https://relay.farcooler.com");

        let explicit: Pairing =
            serde_json::from_str(r#"{"token":"abc","relay":"https://mine.example"}"#)
                .expect("parse");
        assert_eq!(explicit.relay, "https://mine.example");
    }
}

/// The runner's bodies against the shared contract fixtures in
/// `test/fixtures/contracts/`; see `push/contracts.rs`.
#[cfg(test)]
mod contracts;
