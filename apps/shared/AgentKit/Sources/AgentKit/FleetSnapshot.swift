import Foundation

/// The fleet as some surface last knew it.
///
/// A widget cannot hold an SSH session and neither can a watch, so every
/// surface outside the app renders whatever was last written here. Two things
/// follow from that, and both are load-bearing.
///
/// **The fields ARE the wire's fields.** `glyph`, `headline`, `line`, `rank`
/// and `feed` are copied across unchanged from what the daemon derived, never
/// recomputed. The whole point of the ladder is that it is decided once on the
/// host; a snapshot that re-derived a headline would put a widget and the app
/// into exactly the disagreement the ladder exists to prevent.
///
/// **Age is part of the value.** Nothing here is current by construction. Every
/// reader asks `confidence(in:at:)` rather than trusting `status` outright, and
/// `complete` says whether these are all the agents there are.
public struct FleetSnapshot: Codable, Sendable, Equatable {
    /// One agent, in the vocabulary the daemon already sent.
    public struct Agent: Codable, Sendable, Equatable, Identifiable {
        /// The terminal id. Named `id` for `Identifiable`, which is what lets a
        /// widget's `ForEach` be written the same way the app's is.
        public var id: String
        public var label: String
        public var machine: String
        /// `working`, `blocked` or `done`.
        ///
        /// A String rather than an enum, for the same reason
        /// `AgentActivityAttributes.ContentState.status` is one: the daemon and
        /// the relay are not Swift, and a value none of them knew about would
        /// fail to decode and take the whole snapshot down rather than one word
        /// of it.
        public var status: String
        public var glyph: String
        public var headline: String
        public var line: String
        public var feed: [String]
        public var rank: UInt32
        public var turnFailed: Bool
        /// When this state began, when the host said. Nil is "not told", which
        /// is different from "just now" and must not be rendered as it.
        ///
        /// **This is not how old the news is**, and reading it as such was a
        /// shipped bug — see `observedAt` below and `confidence(in:at:)`. An
        /// agent that has been working for ten minutes has an
        /// `activityChangedAt` ten minutes old on a snapshot polled one second
        /// ago, because ten minutes is how long it has been working and not how
        /// long ago we heard.
        public var activityChangedAt: Date?

        /// When THIS agent was last actually heard about, stamped by whoever
        /// assembled the snapshot. Nil is "not told", the same way every other
        /// optional here is.
        ///
        /// **The only field on this type nothing on the wire supplies.** Every
        /// other one is copied across from what the daemon derived — the rule
        /// at the top of the file — and this one deliberately is not, because
        /// it is not a fact about the agent at all. It is a fact about this
        /// client's own knowledge: the daemon has no idea when a phone last
        /// managed to talk to it. So it is stamped where the knowledge is
        /// acquired, which is `FleetSnapshotWriter` for a poll and `merging`
        /// for a push, and it costs the proto nothing.
        ///
        /// It exists because the two other dates cannot answer "how old is
        /// this" between them, and the history is worth keeping:
        ///
        ///   - `capturedAt` is when the SNAPSHOT was assembled, so it is one
        ///     answer for the whole fleet. `merging` moves it to `now` for a
        ///     push about one agent, which made news about A read as news about
        ///     B through F. That was a real bug and it is why `age(of:at:)`
        ///     stopped using it.
        ///   - `activityChangedAt` is when this agent's STATE began, which is
        ///     per agent and fixed the above — but it answers a different
        ///     question. A ten-minute-old `working` reads as stale on a
        ///     one-second-old poll, and agents work for many minutes here, so
        ///     that was the ordinary case rather than the edge one. Hence "a
        ///     lot of last-seen headers".
        ///
        /// This is the third thing, and it is the one the staleness rule
        /// actually wants: how old our INFORMATION is, per agent. It moves
        /// forward every time the fleet is polled and the agent is still in it,
        /// whether or not anything about the agent changed — a daemon that says
        /// "still working" has been heard from.
        ///
        /// **Optional, and nil is not "just now".** A snapshot written by a
        /// build that predates this decodes with nil, and `lastHeard(of:)` then
        /// falls back down the same ladder this type used before — which
        /// understates freshness rather than overstating it, and is exactly the
        /// behavior that build already had.
        ///
        /// **Not part of what a surface draws**, which is why
        /// `saysTheSame(as:)` exists to leave it out of a change test. See
        /// there.
        public var observedAt: Date?
        /// How far the agent is through its OWN task list, as the host counted
        /// it. `planDone` of 4 and `planTotal` of 7 is `4/7`.
        ///
        /// Copied across unchanged, like every other field on this type, and
        /// for the reason at the top of the file made sharper. The host already
        /// states this position — `line` reads `4/7 · Designing test matrix` —
        /// but `line` is a COMPOSED rung, and a surface that scraped `4/7` back
        /// out of it would be a second derivation of a fact the ladder exists
        /// to derive once. It would also be wrong exactly where it matters: a
        /// blocked agent's `line` is the question, because that is the string
        /// worth the row, and the numbers are not in it at all. So they travel
        /// as numbers. See `plan_done` in `proto/farcooler.proto`.
        ///
        /// **Nothing here computes or infers.** No fraction, no denominator, no
        /// fallback — a surface is handed what the host said or it is handed
        /// nothing.
        ///
        /// **Optional, and nil is not zero.** Swift's synthesized `Decodable`
        /// throws on a missing key, so a snapshot written before these existed,
        /// or one written by a phone talking to a daemon too old to send them,
        /// has to decode and simply not draw a bar — the same rule
        /// `reviewsWaiting` states below and for the same reason. And the two
        /// answers are genuinely different: `0` of `7` is an agent that has
        /// written seven tasks and finished none, which has a bar, an empty
        /// one; nil is an agent nobody has said anything about, which has no
        /// bar and no space reserved for one.
        ///
        /// **They arrive together or not at all**, and `planTotal` is never
        /// sent as `0`: `feed::signal` refuses to compose a position out of an
        /// empty list, so the host never sends one. A reader that wants both
        /// should still say so in one `guard` rather than trusting that.
        ///
        /// **`planTotal` can move in both directions while a turn runs.** An
        /// agent adds tasks to its own list as it discovers work, so `2/5`
        /// becoming `2/9` is honest rather than a glitch; and a task the agent
        /// deletes counts toward neither half, so `3/7` can become `3/6`.
        /// Anything that animates this must not read a rising denominator as
        /// progress running backwards.
        ///
        /// **Not every agent can say this.** It is folded out of the agent's own
        /// session log, and only claude's records a task list — see the field
        /// comments in `proto/farcooler.proto` for the counts. A fleet will
        /// carry these on some rows and not others, which is the distribution
        /// `line` already has.
        public var planDone: UInt32?
        public var planTotal: UInt32?

        /// This terminal's activity trace, **as the wire's 66 bytes and not as
        /// anything decoded**. §04's thirteen buckets; see `ActivityTrace`.
        ///
        /// `Data` and not three arrays, and that is the one decision this field
        /// exists to preserve. `proto/farcooler.proto` chose bytes over three
        /// `repeated uint32` fields for a consumer that is exactly this type:
        /// `FleetWidget` holds a whole snapshot per timeline entry, up to
        /// thirteen entries, in an extension with a hard memory ceiling, so the
        /// arithmetic is buckets x series x agents x entries on the DECODED
        /// size. Twelve agents over thirteen entries is 44,928 bytes across 468
        /// heap objects as arrays and 17,472 across 156 as `Data`. A field here
        /// typed `[UInt16]` would hand that saving straight back.
        ///
        /// So the bytes are carried, and `ActivityTrace` reads them **where the
        /// drawing happens** — a `body` that runs once per rendered row rather
        /// than a decode held for every entry of every timeline.
        ///
        /// **Carrying is not recomputing.** The rule at the top of this file is
        /// that the fields ARE the wire's fields; this one is the wire's field
        /// byte for byte. What would break the rule is deriving activity the
        /// daemon did not send — summing these into a fleet trace on the phone,
        /// for instance, which is why `FleetSnapshot.fleetTrace` is its own wire
        /// field and says so.
        ///
        /// **Optional, and nil is not 66 zeroes.** The proto: "Empty when the
        /// terminal has done nothing the trace can see, rather than 66 zero
        /// bytes: a fleet at rest costs nothing." Those are different drawings —
        /// nil is no trace at all, and 66 zeroes is a trace of thirteen quiet
        /// buckets, which §04 says are "Drawn, not omitted". `ActivityTrace`
        /// keeps them apart and `traceDrawsAsAbsentRatherThanFlatZero` holds it
        /// down.
        public var trace: Data?

        public init(
            id: String, label: String, machine: String, status: String,
            glyph: String, headline: String, line: String, feed: [String],
            rank: UInt32, turnFailed: Bool, activityChangedAt: Date?,
            observedAt: Date? = nil,
            planDone: UInt32? = nil, planTotal: UInt32? = nil,
            trace: Data? = nil
        ) {
            self.id = id
            self.label = label
            self.machine = machine
            self.status = status
            self.glyph = glyph
            self.headline = headline
            self.line = line
            self.feed = feed
            self.rank = rank
            self.turnFailed = turnFailed
            self.activityChangedAt = activityChangedAt
            self.observedAt = observedAt
            self.planDone = planDone
            self.planTotal = planTotal
            self.trace = trace
        }

        /// Whether this status stays true as the snapshot ages.
        ///
        /// Blocked and done are facts about something that already happened and
        /// that only a person un-does. Working is a claim about right now, and
        /// right now passes.
        public var isLatched: Bool { status == "blocked" || status == "done" }

        /// Whether this agent SAYS the same thing as another — everything a
        /// surface draws, and not when we heard it.
        ///
        /// `==` cannot answer this and must not be changed to: `observedAt` is
        /// genuinely part of the value, `confidence(in:at:)` reads it, and a
        /// snapshot that compared equal while vouching differently would be a
        /// worse trap than the one this solves.
        ///
        /// It exists for exactly one caller, `WatchLinkHost.send(snapshot:)`,
        /// whose question is "would the watch draw anything different". Once
        /// `observedAt` moves on every poll, plain equality answers "no, never"
        /// — and that guard is what keeps an idle fleet from paying for a
        /// Bluetooth write twenty times a minute. The watch is not left behind
        /// by the omission, because that same function resends unconditionally
        /// every thirty seconds and `staleAfter` is an hour.
        ///
        /// **Written by blanking the field rather than by listing the others.**
        /// A hand-written list of eleven comparisons is a list that silently
        /// stops covering the twelfth field the day somebody adds one, and the
        /// failure would be a wrist that never hears about it. Copying two
        /// small values to reuse the synthesized `==` is the cheaper mistake.
        ///
        /// **`trace` IS compared, and that is the answer rather than an
        /// oversight.** It was the one field added since where the question had
        /// to be asked again, so here is the working:
        ///
        ///   - The wrist DRAWS it, which is the whole test this function
        ///     applies. `observedAt` is blanked precisely because no surface
        ///     draws it; a field the watch renders and this ignored would let
        ///     the guard suppress a write that changes the picture.
        ///   - The case the guard exists for survives. Its own caller says so:
        ///     "This buys an IDLE fleet only" (`WatchLinkHost.send(snapshot:)`).
        ///     An idle terminal records nothing, and `farcooler_core::trace`
        ///     buckets on ABSOLUTE wall-clock rather than backwards from now, so
        ///     two polls of a quiet agent inside one bucket produce byte-
        ///     identical output. That property is in the producer for exactly
        ///     this reason and its module header names this function.
        ///   - The only churn a quiet fleet gains is one bucket boundary per
        ///     window width, and the narrowest width is five minutes — against
        ///     a floor in that same caller that resends unconditionally every
        ///     thirty seconds. So it cannot add a write that the floor was not
        ///     already paying for.
        ///   - A BUSY fleet's bytes do move on nearly every poll. That is not a
        ///     regression either: `line` and `feed` are in this comparison and
        ///     already churn on nearly every poll while an agent works, so a
        ///     working fleet has always pushed at the full poll rate, and the
        ///     caller calls that "the right way round".
        ///
        /// Written down because the cheap-looking edit is to blank it beside
        /// `observedAt`, and that would trade a correct wrist for nothing.
        public func saysTheSame(as other: Agent) -> Bool {
            var mine = self
            var theirs = other
            mine.observedAt = nil
            theirs.observedAt = nil
            return mine == theirs
        }
    }

    public var agents: [Agent]
    public var capturedAt: Date
    /// Whether these are all the agents there are.
    ///
    /// True only after a real fleet poll. A snapshot assembled from pushes
    /// knows about the agents that happened to notify, and a surface that drew
    /// it as the fleet would be asserting that the others do not exist.
    public var complete: Bool

    /// How many worktrees have moved since anyone reviewed them, when the host
    /// could say. **Nil is "not told", and must never be rendered as zero.**
    ///
    /// Copied from what the host derived, like every other field here: it is
    /// the number of rows `changes.inbox` returned with `changed_since_reviewed`
    /// set and a diff to show for it, and the gate behind that flag —
    /// `review::cheap_gate`, two `stat`s over HEAD and the index — runs on the
    /// runner. Nothing in this type recounts it, for the reason stated at the
    /// top of the file.
    ///
    /// A count and not the rows, because no surface that renders this snapshot
    /// needs a name. A widget and a complication have room for a number and a
    /// word; the screen that will want names is the phone's Needs You list,
    /// which runs in the app where `Connection.inbox` already holds the rows in
    /// full. Rows here would put a per-workspace list inside a value that is
    /// serialized once per timeline entry into a process with a hard memory
    /// ceiling — see `FleetProvider.wakeLimit` — in order to carry something
    /// none of these surfaces would print.
    ///
    /// Fleet-wide rather than a field on `Agent`, because that is the shape of
    /// the fact. `changes.inbox` answers per WORKTREE and an agent is a
    /// terminal; several terminals routinely share one worktree, so hanging
    /// this on an agent would either count a workspace once per agent in it or
    /// make every row claim its neighbor's diff.
    ///
    /// **Optional, and never defaulted to a number.** Swift's synthesized
    /// `Decodable` throws on a missing key for a non-optional, so one absent
    /// field fails the whole decode — and a snapshot written by a build that
    /// predates this, or by a phone talking to a daemon too old to answer
    /// `changes.inbox`, still has to decode and simply not show a review line.
    /// The `nil` on the initializer is the same rule pointed at callers: a
    /// writer that knows nothing about reviews starts from "unknown", which is
    /// the only honest thing for it to say.
    ///
    /// **Latched, the way `blocked` is.** An unreviewed diff stays unreviewed
    /// until a person reviews it, so this number does not stop being true as
    /// the snapshot ages the way `working` does, and `confidence(in:at:)`
    /// deliberately does not touch it. It can go out of date in one direction —
    /// a diff that moved after this was written is missing from it, and one
    /// reviewed on a Mac since is still counted here — which is precisely why
    /// the rendering rule reserves amber for `blocked` and forbids it for
    /// reviews. A number that can overstate by one must not wear the color that
    /// means "stop what you are doing and answer me".
    public var reviewsWaiting: Int?

    /// The whole fleet's trace, summed **on the runner**, as the wire's bytes.
    ///
    /// `TerminalList.fleet_trace`, carried across exactly like every agent's own
    /// `trace` and empty on the same terms.
    ///
    /// **Nothing here may add the rows up instead, and the reason is arithmetic
    /// rather than tidiness.** Each terminal's row snaps to the shortest of
    /// §04's three windows that holds its own activity, so bucket 4 of a
    /// five-minute row and bucket 4 of a two-hour row are two different spans of
    /// time and adding them adds unlike things. The daemon holds every ring and
    /// can pick one width for all of them; a client holding only the rendered
    /// rows cannot. `proto/farcooler.proto` states this at the field and
    /// `farcooler_core::trace::Trace::absorb` is where it is done.
    ///
    /// Summing is legitimate at all only because it stays inside one channel —
    /// code with code, output with output. §04: "Never sum the two channels.
    /// Different units; the total would mean nothing."
    ///
    /// **Optional, and nil is not zero**, the same rule `reviewsWaiting` states
    /// above: a snapshot written by a build that predates this, or by a phone
    /// talking to a daemon too old to send it, decodes and simply draws no
    /// fleet trace.
    public var fleetTrace: Data?

    public init(
        agents: [Agent], capturedAt: Date, complete: Bool, reviewsWaiting: Int? = nil,
        fleetTrace: Data? = nil
    ) {
        self.agents = agents
        self.capturedAt = capturedAt
        self.complete = complete
        self.reviewsWaiting = reviewsWaiting
        self.fleetTrace = fleetTrace
    }

    /// Nothing known yet. `complete` is false, which is the honest answer
    /// before anything has ever polled.
    public static let empty = FleetSnapshot(
        agents: [], capturedAt: Date(timeIntervalSince1970: 0), complete: false)

    /// How long a volatile status may be shown as current.
    ///
    /// One hour, which is `STALE_AFTER_S` in `services/relay/src/push.ts`.
    /// Deliberately the same number: two staleness thresholds with two
    /// definitions is two answers to "is this still true", and they drift.
    public static let staleAfter: TimeInterval = 60 * 60

    /// How confident a surface may sound about one agent.
    public enum Confidence: Sendable, Equatable {
        /// Render it plainly. Either the snapshot is fresh, or this status is
        /// one that stays true.
        case known
        /// Render it as the past — "last seen working" — and drop the
        /// confident styling. Do not assert the status.
        case lastSeen
    }

    public func age(at now: Date) -> TimeInterval {
        max(0, now.timeIntervalSince(capturedAt))
    }

    /// When we last actually heard anything about ONE agent.
    ///
    /// The whole of the staleness rule's input, in one place, because three
    /// things ask it — `age(of:at:)`, `confidence(in:at:)` through it, and
    /// `stalenessMoments(after:)` — and two spellings of "how old is this" is
    /// two answers to it.
    ///
    /// A ladder, most honest first:
    ///
    ///   1. **`observedAt`**, which is the actual answer. Stamped per agent by
    ///      whoever assembled this: `now` for every agent a poll listed, `now`
    ///      for the one agent a push was about, and left alone for the agents
    ///      that push was not about.
    ///   2. **`activityChangedAt`**, for a snapshot written before `observedAt`
    ///      existed. It is when the STATE began, which is not this question,
    ///      and using it as the answer is the bug `observedAt` was added to
    ///      fix. It is kept as the fallback anyway because it is what that
    ///      build already behaved like, and because it errs toward saying LESS:
    ///      an agent that has held one state a long time reads as "last seen"
    ///      when it might not be, which understates rather than overstates.
    ///   3. **`capturedAt`**, for a snapshot that carries neither — an old file
    ///      from a daemon too old to send `activitySince` either. It is
    ///      fleet-wide, so a push about one agent re-dates the rest, which is
    ///      exactly the flaw rung 2 was introduced to end. Nothing writes this
    ///      case any more; it is here so an old file still decodes into
    ///      something rather than into "just now".
    func lastHeard(of agent: Agent) -> Date {
        agent.observedAt ?? agent.activityChangedAt ?? capturedAt
    }

    /// How old what is known about ONE agent is. See `lastHeard(of:)`.
    public func age(of agent: Agent, at now: Date) -> TimeInterval {
        max(0, now.timeIntervalSince(lastHeard(of: agent)))
    }

    /// Whether a surface may still state this agent's status as current.
    ///
    /// Judged per agent rather than per snapshot — see `lastHeard(of:)`, which
    /// is where the date comes from and where the history of this decision is
    /// written down.
    ///
    /// **What this asks is how old our INFORMATION is, and it took two goes to
    /// say that.** The question is not how long the agent has been in this
    /// state; a working agent is meant to stay working, and there is nothing
    /// suspect about one that has done so for an hour. What makes `working`
    /// unsafe to assert is not having heard lately.
    ///
    /// This used to measure from `activityChangedAt` — when the state began —
    /// and the comment here called that a deliberate cost "in the safe
    /// direction". It was not safe, because it was not rare: agents here work
    /// for many minutes, so a fleet that was polling perfectly filled up with
    /// "last seen working" headers that meant nothing, and a qualifier that
    /// appears when nothing is wrong is a qualifier nobody reads when
    /// something is. The two dates are two different facts and the code was
    /// using one for the other.
    ///
    /// What has NOT changed is the rule itself. `.lastSeen` is still the
    /// answer once nothing has been heard for `staleAfter`, and several
    /// surfaces depend on that: `stalenessMoments(after:)` schedules a widget
    /// wake-up for it, `glance(at:)` stops counting a working agent it can no
    /// longer vouch for, and `stated(_:_:)` prefixes the row. A phone out of
    /// range still takes every one of them to "last seen" on the hour, which is
    /// the case the rule was written for.
    public func confidence(in agent: Agent, at now: Date) -> Confidence {
        if agent.isLatched { return .known }
        return age(of: agent, at: now) >= Self.staleAfter ? .lastSeen : .known
    }

    /// EVERY moment still ahead at which a surface drawing this snapshot would
    /// have to say something different, with no news arriving. Ascending, and
    /// each one distinct.
    ///
    /// Empty when nothing in here can go stale — every agent is latched, or
    /// there are none. A widget renders one timeline entry per moment, because a
    /// surface that could only learn it had gone stale from a reload it is not
    /// receiving would assert `working` forever.
    ///
    /// All of them rather than the first. Ages are per agent — see
    /// `lastHeard(of:)` — so agents go stale at different moments, and a timeline
    /// that stopped at the earliest one would render that entry from then on
    /// with every LATER agent still reading as current. Permanently, on the
    /// exact case this exists for: a working push sends no alert, so there is no
    /// extension run and no reload to rescue it.
    ///
    /// Bounded by construction, which is what makes it safe to return a list:
    /// no timestamp is in the future, so every moment here falls inside
    /// `staleAfter` of `now`.
    ///
    /// **A freshly polled fleet now yields ONE moment**, not one per agent,
    /// because a poll hears about every agent in the same instant and
    /// `lastHeard(of:)` says so. That is correct rather than a regression — the
    /// list stayed per agent for the case that still needs it, a snapshot some
    /// of whose agents were folded in by pushes at different times — but it is
    /// worth knowing before reading a timeline of one entry as a bug.
    public func stalenessMoments(after now: Date) -> [Date] {
        let moments = agents
            .filter { !$0.isLatched }
            .map { lastHeard(of: $0).addingTimeInterval(Self.staleAfter) }
            .filter { $0 > now }
        return Array(Set(moments)).sorted()
    }

    /// The order every surface shows agents in.
    ///
    /// `rank` is computed on the host precisely so a complication showing one
    /// agent and a list showing twelve pick the same one.
    ///
    /// The id breaks a tie, and it is not decoration. `sorted` is NOT stable, so
    /// two agents sharing a rank can swap places between one render and the
    /// next with nothing having changed — a complication that names a different
    /// agent every time the timeline reloads. Ranks do collide: two agents that
    /// entered the same tier in the same second get the same number, and so does
    /// every agent a push ranked while the app was closed.
    ///
    /// **It sorts and copies the fleet on every access, so a caller on a render
    /// path holds the result rather than asking again.** A computed property
    /// and not a cache, because the alternative is a stored one, and a stored
    /// one on this type is a mutable box inside a `Sendable`, `Codable`,
    /// `Equatable` value that crosses process and actor boundaries — with a
    /// `didSet` on `agents` to invalidate it, a `CodingKeys` list to keep it
    /// off the wire, and a hand-written `==` to keep it out of equality. That
    /// is a great deal of machinery guarding a sort of at most a few dozen
    /// agents, and every piece of it is a way for two surfaces to end up
    /// ordering the fleet differently, which is the one thing `rank` is on the
    /// wire to prevent.
    ///
    /// The answer changes only when `agents` does, so the place to hold it is
    /// wherever a snapshot is adopted. `FleetListView` does that — its `rows`
    /// is this list, recomputed when a snapshot lands instead of on every body
    /// evaluation — and `SnapshotSink.Complication` asks for `ranked.first`
    /// once per snapshot off the drawing actor.
    public var ranked: [Agent] {
        agents.sorted { ($0.rank, $0.id) < ($1.rank, $1.id) }
    }

    /// How many agents are waiting on a person. The number a small widget shows.
    public var needingYou: Int { agents.filter { $0.status == "blocked" }.count }

    /// How many worktrees are waiting to be looked at, or nil when this
    /// snapshot cannot say.
    ///
    /// A pass-through to `reviewsWaiting` on purpose. The count is derived on
    /// the host and this type recomputes nothing; what this adds is the reader's
    /// half of a PAIR, so that a family deciding what to draw asks two questions
    /// spelled alike — `needingYou` and `needsReview` — rather than one question
    /// and one stored field, and so that the answer arrives already carrying the
    /// distinction the stored field exists to preserve.
    ///
    /// **Nil and 0 are different answers.** 0 is "nothing is waiting to be
    /// reviewed"; nil is "nobody has told this phone anything about reviews",
    /// which is what a snapshot written before this field existed says, and what
    /// a runner whose daemon predates `changes.inbox` says forever. A surface
    /// that printed "0 to review" for the second would be stating a fact no
    /// host supplied — the same mistake as drawing `FleetSnapshot.empty` as an
    /// empty fleet, and `Optional` is here so it cannot be made by accident.
    public var needsReview: Int? { reviewsWaiting }

    /// What a surface with room for ONE thing is about, and how to say it.
    ///
    /// The cross-surface law, kept in the one file both widget binaries
    /// compile: **amber means "needs you"**, and it is reserved for an agent
    /// stopped and waiting on a person. A diff waiting to be reviewed is not
    /// that — a blocked agent cannot continue, an unreviewed diff is merely
    /// waiting — so it gets a mark and a tint of its own and never amber.
    ///
    /// The symbol and the words live on the case rather than in each view, and
    /// that is a deliberate departure from how `agentTitle` is handled. Three
    /// copies of that function are tolerable because the rule they keep is
    /// "read the wire's own fields, in this order", which is one sentence and is
    /// written out in all three. A table of glyph, tint, word and precedence is
    /// not one sentence, and two surfaces that drew the same fleet with
    /// different glyphs would be exactly the disagreement the table exists to
    /// end — visibly, since a phone widget and a wrist are looked at within a
    /// minute of each other.
    ///
    /// Only the color is left to the views, because a color is a SwiftUI type
    /// and this file has no business importing SwiftUI: it is the wire's shape,
    /// compiled into a notification service extension and a watch complication
    /// that have no views at all. The mapping each view writes is three cases
    /// long and is stated beside both.
    public enum Glance: Sendable, Equatable {
        /// Agents stopped, waiting on a person. Amber, and nothing else is.
        case blocked(Int)
        /// Workspaces whose diff moved since anyone last looked. Never amber.
        case review(Int)
        /// Agents getting on with it, with nothing waiting on you.
        case working(Int)

        public var count: Int {
            switch self {
            case let .blocked(n), let .review(n), let .working(n): n
            }
        }

        /// The SF Symbol every surface draws for this state.
        ///
        /// A check for `working` rather than nothing at all. The rule allows
        /// either, and a mark is the better half of it here: the circular
        /// families draw a glyph over a number and have no room for a word, so
        /// an absent symbol would leave "4" alone on a watch face with no way
        /// to tell it from "4 need you" — which is the one confusion this whole
        /// table exists to prevent.
        public var symbol: String {
            switch self {
            case .blocked: "exclamationmark.triangle.fill"
            case .review: "plus.forwardslash.minus"
            case .working: "checkmark"
            }
        }

        /// The count and what it counts, for a family whose whole budget is one
        /// line: "2 need you", "3 to review", "4 working".
        ///
        /// The count LEADS. These lines are drawn in inline accessories, which
        /// truncate at the tail on a narrow face, and a line that dropped the
        /// number and kept the qualifier would be a widget with nothing to say.
        public var phrase: String {
            switch self {
            case let .blocked(n): "\(n) \(n == 1 ? "needs" : "need") you"
            case let .review(n): "\(n) to review"
            case let .working(n): "\(n) working"
            }
        }

        /// The same fact under a number that is already drawn large: "agents
        /// need you", "workspaces to review", "agents working".
        ///
        /// **Workspaces**, not agents, for `review`. The two counts are counts of
        /// different things — `changes.inbox` answers per workspace — and a
        /// caption that called both of them agents would make "2 need you" and
        /// "3 to review" look like five agents.
        public var caption: String {
            switch self {
            case let .blocked(n): n == 1 ? "agent needs you" : "agents need you"
            case let .review(n): n == 1 ? "workspace to review" : "workspaces to review"
            case let .working(n): n == 1 ? "agent working" : "agents working"
            }
        }
    }

    /// The one thing this fleet is about at `now`, or nil when it is about
    /// nothing.
    ///
    /// **Blocked, then review, then working**, decided here rather than in each
    /// family so that a home screen widget, a lock screen accessory and a watch
    /// complication cannot pick differently. It is the same argument `rank`
    /// makes about which AGENT leads, one level up: this is which QUESTION
    /// leads, and there may only be one answer to it or the surfaces disagree.
    ///
    /// It takes a date because exactly one of the three rungs is volatile.
    /// Blocked is latched and a waiting review is latched — see
    /// `reviewsWaiting` — so neither stops being true while the snapshot sits
    /// on disk. `working` is a claim about right now, so the working rung
    /// counts only the agents `confidence(in:at:)` will still vouch for, which
    /// is the same rule every row on every one of these surfaces already obeys,
    /// applied to a count instead of a row.
    ///
    /// **Nil rather than `.working(0)`** when nothing qualifies. A nil answer is
    /// what lets each family keep the sentence it already had for a fleet with
    /// nothing to report — the top agent's headline, "No agents", "Open
    /// <app>" — instead of this type inventing a fourth state that every
    /// surface would then have to special-case back out of. It also keeps the
    /// hour-old snapshot honest without any new vocabulary: once nothing
    /// working can still be asserted, the fleet-wide claim simply stops being
    /// made and the surface falls back to naming one agent in the past tense.
    public func glance(at now: Date) -> Glance? {
        let blocked = needingYou
        if blocked > 0 { return .blocked(blocked) }
        if let reviews = needsReview, reviews > 0 { return .review(reviews) }
        let working = agents.filter {
            $0.status == "working" && confidence(in: $0, at: now) == .known
        }.count
        return working > 0 ? .working(working) : nil
    }

    /// Fold in the one agent a push was about, keeping every other.
    ///
    /// `complete` is carried through rather than recomputed: merging does not
    /// discover agents, so a partial snapshot stays partial however many
    /// pushes arrive, and a complete one stays complete.
    ///
    /// `reviewsWaiting` is carried through as well, and for a related reason:
    /// a push is about ONE agent's turn ending and says nothing whatsoever
    /// about whether some other worktree's diff moved, so there is nothing here
    /// to update it with. Carried rather than cleared, because clearing it
    /// would turn "3 to review" into "unknown" — and the review line off every
    /// surface — each time an unrelated agent notified, which is a worse answer
    /// than a count that is a poll or two old. It is latched, so what it says
    /// stays true; what it can miss is a diff that moved since.
    ///
    /// `capturedAt` moves to `now` and keeps meaning what it has always meant —
    /// when this file was last assembled, which is what the widgets' "as of"
    /// footer reports and what tells a real snapshot from `empty`. It is NOT
    /// what vouches for an agent any more: `confidence(in:at:)` asks each agent
    /// for its own timestamp first, so folding in news about one agent no longer
    /// silently re-dates the rest.
    ///
    /// **This function is where `observedAt` comes from on the push path, and
    /// it is the only place that can know it.** It is handed exactly one agent
    /// and it keeps the rest, so it — and nothing downstream of it — knows
    /// which row came from fresh news and which was carried over. Stamping the
    /// fold-in here and leaving the others alone is the entire distinction the
    /// field exists to draw, and it needs nothing from the wire to draw it.
    public func merging(_ agent: Agent, at now: Date) -> FleetSnapshot {
        var incoming = agent
        // The agent this push is about IS current, whatever else in here is not.
        // Stamped here rather than left to each caller so every path through
        // this function leaves the fold-in vouched for and the others alone.
        //
        // Both dates, and they are not the same claim. `activityChangedAt` is
        // filled in only when the caller had nothing, because a push that DID
        // date the state began when it says it began. `observedAt` is
        // overwritten unconditionally: the news arrived now, whenever the state
        // it describes started, and this is the moment we heard it.
        if incoming.activityChangedAt == nil { incoming.activityChangedAt = now }
        incoming.observedAt = now

        var merged = agents
        if let index = merged.firstIndex(where: { $0.id == incoming.id }) {
            merged[index] = incoming
        } else {
            merged.append(incoming)
        }
        return FleetSnapshot(
            agents: merged, capturedAt: now, complete: complete,
            reviewsWaiting: reviewsWaiting,
            // Carried, not recomputed and not cleared, for `reviewsWaiting`'s
            // reason sharpened by the arithmetic at the field: a push is about
            // one agent and carries no trace of its own, and the fleet sum is
            // summable only at one width chosen across every ring on the runner
            // — which this process cannot do. Carried means the compact Island
            // keeps drawing the last summed history rather than blanking each
            // time an unrelated agent notifies; the trace is history, and
            // history does not stop being true.
            fleetTrace: fleetTrace)
    }

    /// Whether two fleets say the same thing about the same agents, in the same
    /// order — everything a surface draws, and not when any of it was heard.
    ///
    /// `Agent.saysTheSame(as:)` per row, and the whole of the reasoning is
    /// there. What this adds is the count and the order, both of which change
    /// what a list renders.
    ///
    /// Nil is never the same as something. A caller that has sent nothing yet
    /// has to send this one.
    public func agentsSayTheSame(as other: FleetSnapshot?) -> Bool {
        guard let other, agents.count == other.agents.count else { return false }
        return zip(agents, other.agents).allSatisfy { $0.saysTheSame(as: $1) }
    }
}

// MARK: - The activity trace, as the wire carries it

/// §04's thirteen buckets, read out of the wire's bytes and not out of anything
/// this process worked out for itself.
///
/// **A reader over `Data`, never a decode into arrays.** It holds the 66 bytes
/// it was handed and pulls a bucket out on demand; there is no `[UInt16]`
/// anywhere in this type. That is not a micro-optimisation, it is the entire
/// reason the field is bytes on the wire: `proto/farcooler.proto` sets out the
/// arithmetic — buckets x series x agents x entries, on the DECODED size, in a
/// widget extension holding a snapshot per timeline entry — and a struct that
/// materialised three arrays at init would give that saving back the moment a
/// snapshot was held rather than drawn. So construct one where you draw, let it
/// go, and keep the bytes.
///
/// # The layout
///
/// `farcooler_core::trace::Trace::encode`, which is the only producer:
///
/// ```text
///  0        version << 4 | width code (0 = 5min, 1 = 30min, 2 = 2h)
///  1..27    13 x u16 LITTLE-endian, code lines, oldest first
/// 27..53    13 x u16 little-endian, output lines, oldest first
/// 53..66    13 x u8, commits, oldest first
/// ```
///
/// # What this refuses, and why refusing is the feature
///
/// `init?` returns nil for anything that is not a version-1 trace, and every
/// surface then draws NOTHING rather than a trace of thirteen zeroes. There are
/// two separate cases and they are worth naming apart:
///
///   - **Empty is absent.** The producer sends no bytes at all when a terminal
///     has done nothing the trace can see, deliberately rather than 66 zeroes,
///     "so a fleet at rest costs nothing". A surface that drew that as a flat
///     line at zero would be asserting thirteen buckets of observed silence
///     about a terminal nobody has observed. Note that 66 zeroes IS a valid
///     trace and does draw — thirteen quiet buckets, "Drawn, not omitted" per
///     §04 — so the two cannot be collapsed.
///   - **An unknown version is refused whole.** The proto: "A client that does
///     not recognize the version must draw an empty trace rather than read the
///     rest." A partial parse of an encoding we do not know is a bar chart of
///     somebody else's bytes, and it would look entirely plausible.
///
/// The width code is checked on the same terms. Within version 1 it is one of
/// three values because `WIDTHS` has three entries, so a fourth is not a
/// version-1 trace — it is a later encoding whose author forgot the nibble
/// above it, and guessing at its span is the same mistake one paragraph up.
public struct ActivityTrace: Sendable, Equatable {
    /// §04: "Buckets 13. Fixed count at every size." Matches
    /// `farcooler_core::trace::BUCKETS`.
    public static let buckets = 13

    /// `1 + 13*2 + 13*2 + 13`. Matches `farcooler_core::trace::ENCODED_LEN`.
    public static let encodedLength = 1 + buckets * 2 + buckets * 2 + buckets

    /// The encoding this build knows. `farcooler_core::trace::ENCODING_VERSION`.
    static let version: UInt8 = 1

    /// Which of the three windows this row snapped to.
    ///
    /// §04: "Snaps to 1h / 6h / 24h — the shortest window containing the
    /// activity — with the span printed as two mono characters beside the
    /// trace."
    ///
    /// **The buckets are five minutes, thirty minutes and two hours, so the
    /// windows are 65 minutes, six and a half hours and twenty-six.** That is
    /// the producer's own departure from the round numbers and its reason is
    /// the one property everything here rests on: 3600 does not divide by 13, so
    /// an exactly-one-hour window of thirteen buckets cannot be aligned to
    /// wall-clock at all, and the alignment is what makes two polls inside one
    /// bucket produce identical bytes. `farcooler_core::trace`'s header states
    /// it and tells clients to print the round number anyway, "because that is
    /// what the span means to a person, and nobody can see the extra five
    /// minutes on a bar 12pt wide". So `label` is the spec's word and the
    /// comment here is the truth behind it.
    public enum Span: UInt8, Sendable, Equatable, CaseIterable {
        case hour = 0
        case sixHours = 1
        case day = 2

        /// The span beside the trace. §04 says two mono characters; `24h` is
        /// three, which is the spec's own count being one short rather than a
        /// third window being available to pick.
        public var label: String {
            switch self {
            case .hour: "1h"
            case .sixHours: "6h"
            case .day: "24h"
            }
        }
    }

    /// The 66 bytes, held whole. Indexing goes through `byte(_:)` because a
    /// `Data` sliced out of a larger buffer does not start at zero.
    private let bytes: Data
    public let span: Span

    /// The wire's bytes, or nil for anything this build may not draw.
    ///
    /// Nil for: no bytes (a terminal with nothing to show), the wrong length, a
    /// version this build does not know, a width code version 1 does not have.
    public init?(_ encoded: Data?) {
        guard let encoded, encoded.count == Self.encodedLength else { return nil }
        let header = encoded[encoded.startIndex]
        guard header >> 4 == Self.version, let span = Span(rawValue: header & 0x0F) else {
            return nil
        }
        self.bytes = encoded
        self.span = span
    }

    private func byte(_ offset: Int) -> UInt8 { bytes[bytes.startIndex + offset] }

    /// One little-endian `u16` out of a series. `bucket` is 0…12, oldest first.
    private func pair(_ base: Int, _ bucket: Int) -> UInt16 {
        let at = base + bucket * 2
        return UInt16(byte(at)) | (UInt16(byte(at + 1)) << 8)
    }

    /// Lines of code touched in this bucket. **The upper half.**
    ///
    /// §04: "up is code". The growth of the worktree's insertions plus
    /// deletions, split among the panes the daemon saw working while it grew —
    /// see `proto/farcooler.proto`, which is careful that this is a texture and
    /// not an accounting figure.
    public func code(_ bucket: Int) -> UInt16 { pair(1, bucket) }

    /// Lines the terminal put in front of the person. **The lower half.**
    public func output(_ bucket: Int) -> UInt16 { pair(1 + Self.buckets * 2, bucket) }

    /// Commits landed in this bucket. A mark on the axis, never a bar.
    public func commits(_ bucket: Int) -> UInt8 { byte(1 + Self.buckets * 4 + bucket) }

    /// The busiest bucket of one half, which is what full height means there.
    ///
    /// §04, and the wording is the spec's own correction of an earlier draft
    /// that "never said what full height meant": "Tallest bar = that row's
    /// busiest bucket, per half. Each half therefore reaches full height, so the
    /// two halves are not comparable to one another — only each against its own
    /// past. Never cross-row."
    ///
    /// Zero when the half is silent, which is the case §05 draws: "docs-sweep
    /// has an empty upper half — it has talked and touched nothing, drawn rather
    /// than omitted."
    public var tallestCode: UInt16 { (0..<Self.buckets).reduce(0) { max($0, code($1)) } }
    public var tallestOutput: UInt16 { (0..<Self.buckets).reduce(0) { max($0, output($1)) } }
}

// MARK: - What the OTHER runners had

/// One runner's worktrees, as this app last saw them.
///
/// `FleetSnapshot` above cannot answer this and should not be made to. It is
/// AGENTS-ONLY by design — "a widget listing every terminal on every runner
/// would be a list nobody can find anything in" — it is a SINGLE file, and it
/// is rewritten whole on every poll by whichever connection is live. Ask it
/// "what worktrees does `gpu-box-2` have" and it answers about whatever runner
/// polled last.
///
/// So this is a second, smaller thing with a different shape: keyed by runner,
/// merged rather than clobbered, and about worktrees rather than agents. It
/// exists for one screen — the overview's grid, which the owner asked to list
/// all worktrees across all servers — and for the reason a second live
/// connection cannot do that job: `Connection.start` claims four process-wide
/// slots (`Connection.current`, `WatchLinkHost.shared.adopt`,
/// `Reachability.shared.onShouldRetry`, and the one `fleet.json`), so two live
/// connections would not cost twice as much, they would fight.
///
/// **Everything in here is a claim about the past, and `seenAt` is part of the
/// value.** Nothing that reads it may draw it as current: see `decayed`, which
/// applies this app's existing staleness rule rather than inventing a second
/// one.
struct RunnerDirectory: Codable, Sendable, Equatable {
    /// One worktree, with just enough to draw a card and nothing more.
    ///
    /// No terminal ids, no pane state, no scrollback. A card shows a name, a
    /// ribbon and a tail, and a cache that held more would be a cache somebody
    /// eventually tried to open a pane from — which is the one thing a
    /// workspace on a runner you are not connected to cannot do.
    struct Workspace: Codable, Sendable, Equatable {
        var id: String
        var name: String
        var isHidden: Bool
        /// The tabs, in the order the runner gave them, as a title and a mark
        /// each. `Diff` leads, exactly as `ShellFleetMap.of` builds them, so a
        /// cached ribbon and a live one read the same way round.
        var tabs: [Tab]
        /// What this worktree's most recent agent last said.
        var tail: [String]

        init(
            id: String, name: String, isHidden: Bool, tabs: [Tab], tail: [String]
        ) {
            self.id = id
            self.name = name
            self.isHidden = isHidden
            self.tabs = tabs
            self.tail = tail
        }
    }

    /// A tab as the cache holds it.
    ///
    /// The mark is a String and not a `ShellMark`, for the reason
    /// `FleetSnapshot.Agent.status` is one: a value written by a later build
    /// must not fail to decode and take the whole cache down with it. An
    /// unrecognized word reads as `working`, which is the rank that claims the
    /// least — and which `decayed` then turns into `stale` anyway.
    struct Tab: Codable, Sendable, Equatable {
        var title: String
        var mark: String

        init(title: String, mark: String) {
            self.title = title
            self.mark = mark
        }
    }

    /// The runner's id — `Runner.id.uuidString` — because that is what a tap
    /// has to name to select it. Not the address: two runners can share a box.
    var runner: String
    /// The runner's label, for the header.
    var label: String
    /// When this app last actually heard from it.
    var seenAt: Date
    var workspaces: [Workspace]

    init(runner: String, label: String, seenAt: Date, workspaces: [Workspace]) {
        self.runner = runner
        self.label = label
        self.seenAt = seenAt
        self.workspaces = workspaces
    }
}

/// Every runner's directory, on disk, merged rather than clobbered.
///
/// `UserDefaults` and not the App Group container, deliberately: this is for
/// the app's own overview and no other surface renders it, so it does not need
/// to cross a process boundary — and `SnapshotStore`'s container is where the
/// things that DO cross one live. `RunnerStore` keeps the runners themselves
/// in plain `UserDefaults` for the same reason: none of this is secret.
///
/// One key holding one dictionary, written whole. A key per runner would be
/// cheaper to write and impossible to enumerate without knowing every runner's
/// id first, which is exactly the question this answers.
enum RunnerDirectoryStore {
    /// Kept spelled out, because it names a slot on disk that installs already
    /// have — the rule `RunnerStore`'s own keys follow.
    private static let key = "runnerDirectories"

    static func read(from defaults: UserDefaults = .standard) -> [RunnerDirectory] {
        guard let data = defaults.data(forKey: key),
            let decoded = try? decoder.decode([RunnerDirectory].self, from: data)
        else { return [] }
        return decoded
    }

    /// Record what one runner has, leaving every other runner's entry alone.
    ///
    /// Merged and not appended: a runner polls every three seconds, and a list
    /// that grew by one entry per poll would be a preferences file that grew
    /// without bound. The newest answer about a runner replaces the previous
    /// one entirely — a worktree that has gone is gone, and a directory that
    /// unioned old entries in would keep resurrecting removed worktrees.
    static func record(
        _ directory: RunnerDirectory, in defaults: UserDefaults = .standard
    ) {
        var all = read(from: defaults).filter { $0.runner != directory.runner }
        all.append(directory)
        guard let data = try? encoder.encode(all) else { return }
        defaults.set(data, forKey: key)
    }

    /// Forget a runner — for the moment somebody removes one, so its worktrees
    /// stop turning up in a grid for a machine the app can no longer reach.
    static func forget(
        runners kept: Set<String>, in defaults: UserDefaults = .standard
    ) {
        let all = read(from: defaults)
        let survivors = all.filter { kept.contains($0.runner) }
        guard survivors.count != all.count, let data = try? encoder.encode(survivors)
        else { return }
        defaults.set(data, forKey: key)
    }

    /// Seconds since 1970 on both sides, pinned for `SnapshotStore`'s reason:
    /// a reader and a writer built with different strategies write files
    /// neither can read, and the symptom is a permanently empty surface.
    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        return encoder
    }()

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return decoder
    }()
}
