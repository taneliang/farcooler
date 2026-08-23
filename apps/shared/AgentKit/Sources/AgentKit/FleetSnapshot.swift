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

        public init(
            id: String, label: String, machine: String, status: String,
            glyph: String, headline: String, line: String, feed: [String],
            rank: UInt32, turnFailed: Bool, activityChangedAt: Date?,
            observedAt: Date? = nil,
            planDone: UInt32? = nil, planTotal: UInt32? = nil
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

    public init(
        agents: [Agent], capturedAt: Date, complete: Bool, reviewsWaiting: Int? = nil
    ) {
        self.agents = agents
        self.capturedAt = capturedAt
        self.complete = complete
        self.reviewsWaiting = reviewsWaiting
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
            reviewsWaiting: reviewsWaiting)
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
