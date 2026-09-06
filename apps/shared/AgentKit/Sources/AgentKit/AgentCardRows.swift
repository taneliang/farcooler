import Foundation

/// One agent's line on the lock screen card, and the arithmetic that decides
/// which lines there are.
///
/// **This is the reader for a field the relay has been sending for a while.**
/// `services/relay/src/push.ts` puts a row per agent on the content state —
/// name, runner, status, detail, the diff counts and the thirteen buckets — and
/// `AgentCardState` decoded none of it, so the card drew one agent and counted
/// the rest. Everything here is on the wire already; nothing below asks the
/// relay for anything new.
///
/// **Why it is here and not in the widget.** The split between what a row says
/// and how a row is drawn is the whole of what can be checked without a device:
/// `swift test --package-path apps/shared/AgentKit` runs on every push and the
/// iOS UI suite is compiled and never executed, so a sentence composed in a
/// `View.body` is a sentence nothing reads back. The same reasoning put
/// `GlanceTraceLayout` beside the trace it draws.

// MARK: - The wire's timestamps

/// Unix seconds and Unix milliseconds, told apart by magnitude.
///
/// **One copy of a rule that was written twice.** `AgentCardState.init(from:)`
/// had it inline for the headline's `startedAt`, and a row carries two more
/// dates of exactly the same kind — so the second and third copies would have
/// been three places to notice that the daemon's turn clock is milliseconds
/// while everything else in `push.ts` is seconds. Left to Swift's own default a
/// plausible number decodes to a date decades out and the card counts nonsense.
///
/// No second count this side of the year 5000 reaches 1e11, and no millisecond
/// count since 1973 falls below it.
enum AgentCardClock {
    /// The wire's number as a date, or nil for an absent or nonsense one.
    static func date(_ value: Double?) -> Date? {
        guard let value, value > 0 else { return nil }
        return Date(timeIntervalSince1970: value > 1e11 ? value / 1000 : value)
    }

    /// The other direction, for the encoder ActivityKit persists cards with.
    /// Milliseconds, because that is what this side reads back.
    static func number(_ date: Date?) -> Double? {
        date.map { $0.timeIntervalSince1970 * 1000 }
    }
}

// MARK: - A row

/// One agent, as the relay stored its own last notice.
///
/// `ActivityRow` in `services/relay/src/push.ts` is the other end of this, field
/// for field. A row is one runner's word about one agent, carried rather than
/// composed — the relay adds nothing to it but the ordering.
///
/// **Lenient in exactly the way `AgentCardState` is**, and for the harder
/// version of the same reason: ActivityKit decodes a persisted state back into
/// this type after an upgrade, and a strict decode that throws does not lose a
/// row, it keeps the whole activity out of `Activity.activities` — where
/// `LiveActivities.reapDuplicates` can never end it. Every field defaults.
public struct AgentCardRow: Codable, Hashable, Sendable, Identifiable {
    /// This row's own terminal, so a tap on the row opens the pane it names
    /// rather than the one the card happens to be headlining.
    public var terminal: String
    public var label: String
    public var machine: String
    /// `working`, `blocked` or `done`. A String for `AgentCardState.status`'s
    /// reason: the daemon and the relay are not Swift, and a word none of them
    /// knew about must cost one row's ring rather than the card.
    public var status: String
    public var detail: String
    /// Absent where the runner measured nothing — a worktree it has not probed,
    /// or one with no base to compare against. `nil` is not zero and the row
    /// draws no figures at all rather than `+0 −0` over a measurement nobody
    /// made.
    public var insertions: Int?
    public var deletions: Int?
    public var commits: Int?
    /// When this agent's turn began.
    public var startedAt: Date?
    /// When this agent last said anything, which is what decides whether the
    /// card may still assert `working` about it. See `confidence(at:)`.
    public var updatedAt: Date?
    /// The thirteen buckets, as the wire's 66 bytes.
    ///
    /// Base64 on the wire and `Data` here, which is the same encoding
    /// `FleetSnapshot.Agent.trace` carries and the same bytes `ActivityTrace`
    /// reads. **Absent is not thirteen quiet buckets** — a terminal with
    /// nothing to show sends no field at all, deliberately, and `ActivityTrace`
    /// refuses to build from nil so the surface draws nothing rather than a
    /// flat line at zero.
    public var trace: Data?

    public var id: String { terminal }

    public init(
        terminal: String = "",
        label: String = "",
        machine: String = "",
        status: String = "",
        detail: String = "",
        insertions: Int? = nil,
        deletions: Int? = nil,
        commits: Int? = nil,
        startedAt: Date? = nil,
        updatedAt: Date? = nil,
        trace: Data? = nil
    ) {
        self.terminal = terminal
        self.label = label
        self.machine = machine
        self.status = status
        self.detail = detail
        self.insertions = insertions
        self.deletions = deletions
        self.commits = commits
        self.startedAt = startedAt
        self.updatedAt = updatedAt
        self.trace = trace
    }

    private enum CodingKeys: String, CodingKey {
        case terminal, label, machine, status, detail
        case insertions, deletions, commits, startedAt, updatedAt, trace
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        func text(_ key: CodingKeys) -> String {
            ((try? container.decodeIfPresent(String.self, forKey: key)) ?? nil) ?? ""
        }
        func number(_ key: CodingKeys) -> Int? {
            (try? container.decodeIfPresent(Int.self, forKey: key)) ?? nil
        }
        terminal = text(.terminal)
        label = text(.label)
        machine = text(.machine)
        status = text(.status)
        detail = text(.detail)
        insertions = number(.insertions)
        deletions = number(.deletions)
        commits = number(.commits)
        startedAt = AgentCardClock.date(
            (try? container.decodeIfPresent(Double.self, forKey: .startedAt)) ?? nil)
        updatedAt = AgentCardClock.date(
            (try? container.decodeIfPresent(Double.self, forKey: .updatedAt)) ?? nil)
        // Base64, decoded here rather than at the draw site. A field that is not
        // base64, or is base64 of the wrong number of bytes, becomes no trace —
        // `ActivityTrace.init?` refuses anything that is not 66 bytes of a
        // version it knows, so the only thing this has to get right is not
        // throwing.
        trace = ((try? container.decodeIfPresent(String.self, forKey: .trace)) ?? nil)
            .flatMap { Data(base64Encoded: $0) }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(terminal, forKey: .terminal)
        try container.encode(label, forKey: .label)
        try container.encode(machine, forKey: .machine)
        try container.encode(status, forKey: .status)
        try container.encode(detail, forKey: .detail)
        try container.encodeIfPresent(insertions, forKey: .insertions)
        try container.encodeIfPresent(deletions, forKey: .deletions)
        try container.encodeIfPresent(commits, forKey: .commits)
        try container.encodeIfPresent(AgentCardClock.number(startedAt), forKey: .startedAt)
        try container.encodeIfPresent(AgentCardClock.number(updatedAt), forKey: .updatedAt)
        try container.encodeIfPresent(trace?.base64EncodedString(), forKey: .trace)
    }

    /// How long ago this row last said anything.
    ///
    /// Zero when it never said — which reads as "just heard" rather than "heard
    /// nothing for ever", and that is the safe direction here: a row with no
    /// `updatedAt` came from a relay that did not send one, and dashing its ring
    /// on that basis would be this card inventing an outage out of a missing
    /// field.
    public func age(at now: Date) -> TimeInterval {
        guard let updatedAt else { return 0 }
        return max(0, now.timeIntervalSince(updatedAt))
    }

    /// Whether the card may still state this row's status as current.
    ///
    /// `FleetSnapshot`'s rule, applied to a pushed row rather than to a stored
    /// agent — the same function, not a second opinion. Blocked and done are
    /// latched at any age; working stops being vouched for after
    /// `FleetSnapshot.staleAfter`.
    public func confidence(at now: Date) -> FleetSnapshot.Confidence {
        FleetSnapshot.confidence(status: status, heard: age(at: now))
    }
}

// MARK: - The card, laid out

/// What the card draws: a header, two rows and a tail.
///
/// **Two rows, and the ceiling the wire measured is a different number.**
/// `ROWS_SHOWN` in `services/relay/src/index.ts` is four and the byte
/// arithmetic beside it allows eight; both are about what a 4KB push can
/// carry. This is about what a presentation roughly 160 points tall can draw,
/// and the design answers that with `+N more` rather than by growing. The two
/// limits are independent and the smaller one is here.
///
/// Every string on the card is composed in this type and read back by a test.
/// The relay sends three numbers rather than a sentence for exactly that
/// reason — see `ActivityState.blocked` in `push.ts` — so the card owns its own
/// wording and the Dynamic Island can word the same three numbers shorter.
public struct AgentCardLayout: Sendable, Equatable {
    /// How many agents get a line.
    public static let rowsDrawn = 2

    /// How many rings the tail draws before it stops.
    ///
    /// **A width limit, and the only figure here that is not the design's.** The
    /// card is 360 points wide with 14 of padding either side, the tail's line
    /// wants about 130 of that for `+6 more · +391 −112`, and a ribbon mark plus
    /// its gap is 12 — so twelve rings is what is left, with room to spare for a
    /// wider total. A fleet larger than that says so in the header and in `+N
    /// more`, both of which count every agent; the rings are a texture and stop.
    public static let ringsDrawn = 12

    /// One drawn line, with every string it shows already composed.
    public struct Row: Sendable, Equatable, Identifiable {
        public let row: AgentCardRow
        /// The state, as the one mark. **This is where state lives** — the
        /// trace beside it never carries amber or blue, because history is not
        /// urgent.
        public let mark: GlanceMark
        /// The agent's name. Its runner is deliberately not on the line: the
        /// design gives the row a name and a detail, and the width it would
        /// take belongs to the trace.
        public let name: String
        public let detail: String
        /// `+142 −37`, or nil where the runner measured nothing.
        public let diff: String?
        /// `4 commits`, or `as of 12m` for a row that has gone quiet. See
        /// `AgentCardLayout.footnote`.
        public let footnote: String?

        public var id: String { row.terminal }
        /// The thirteen buckets, straight off the push.
        public var trace: Data? { row.trace }
        public var terminal: String { row.terminal }
        public var startedAt: Date? { row.startedAt }
    }

    public let rows: [Row]
    /// Agents with no line: `+6 more`. See `init?`, which is where the two
    /// separate reasons a row has no line are added together.
    public let hidden: Int
    /// The header's own sentence: `2 need you`.
    public let title: String
    /// What is left of it, in mono on the right: `3 to review · 3 in flight`.
    public let counts: String?
    /// The header's ring, on the same precedence every other surface uses.
    public let mark: GlanceMark
    /// One ring per agent in the fleet, in tier order.
    ///
    /// **From the tier counts, not from the rows**, because there are only ever
    /// a handful of rows and the fleet is what this is about. That is also its
    /// limit: the counts say how many agents are in each tier and nothing about
    /// any one of them, so every ring here declines to state a core and none of
    /// them is ever dashed. A per-agent fact about an agent the card has no row
    /// for is not on the wire.
    public let rings: [GlanceMark]
    /// The tail's own line: `+6 more · +391 −112`.
    public let line: String?

    /// The card, from a push that carried rows.
    ///
    /// **Nil when there are none**, which is the compatibility path and not a
    /// failure: an app updated ahead of its relay, or a card started by one, has
    /// a state with no `rows` key at all and must keep drawing the headline the
    /// way it always did. `knowsFleet` is required for the same reason the
    /// header needs it — a card may not write "2 need you" from counts nothing
    /// sent.
    public init?(state: AgentCardState, now: Date = Date()) {
        guard !state.rows.isEmpty, state.knowsFleet else { return nil }

        let drawn = state.rows.prefix(Self.rowsDrawn)
        rows = drawn.map { row in
            Row(
                row: row,
                mark: GlanceMark(status: row.status, confidence: row.confidence(at: now)),
                name: Self.name(of: row),
                detail: row.detail,
                diff: Self.diff(insertions: row.insertions, deletions: row.deletions),
                footnote: Self.footnote(row, at: now))
        }

        // Two separate populations, added once. `more` is the fleet minus the
        // rows the relay SENT — it already accounts for a row dropped for the
        // byte budget and for one that has gone quiet — and the rest is the
        // rows it sent that this card has no room for. Neither one alone is the
        // number a person is owed: `more` under a card drawing two of four rows
        // would undercount by two, and the leftover alone would ignore the fleet
        // entirely.
        hidden = max(0, state.more) + max(0, state.rows.count - drawn.count)

        // The header's clauses, in the order the fleet is urgent in. An empty
        // tier is dropped rather than written as zero: "0 need you" is worse
        // than silence on a lock screen.
        var clauses: [String] = []
        if state.blocked > 0 {
            clauses.append("\(state.blocked) need\(state.blocked == 1 ? "s" : "") you")
        }
        if state.review > 0 { clauses.append("\(state.review) to review") }
        if state.working > 0 { clauses.append("\(state.working) in flight") }
        // A fleet with nothing in any tier still needs a title — the card is on
        // screen either way, and a blank header reads as a card that failed to
        // load. The relay's `fleetHeader` falls back to the same two words.
        title = clauses.first ?? "Your agents"
        counts = clauses.count > 1 ? clauses.dropFirst().joined(separator: " · ") : nil
        mark = GlanceMark(status: Self.tier(state))
        rings = Self.rings(state)

        var tail: [String] = []
        if hidden > 0 { tail.append("+\(hidden) more") }
        if let totals = Self.diff(insertions: state.insertions, deletions: state.deletions) {
            tail.append(totals)
        }
        line = tail.isEmpty ? nil : tail.joined(separator: " · ")
    }

    /// What to call this agent.
    ///
    /// The runner, then the terminal, when the daemon sent no name. A blank line
    /// where a name goes is the one thing worse than an ugly one, and a card
    /// started by an older build carries neither — see `AgentCardRow.init(from:)`,
    /// which defaults every string rather than throwing.
    static func name(of row: AgentCardRow) -> String {
        if !row.label.isEmpty { return row.label }
        if !row.machine.isEmpty { return row.machine }
        return row.terminal
    }

    /// `+142 −37`, or nil.
    ///
    /// **Both halves or neither.** An absent count is not zero — the runner
    /// measured nothing — and `+142 −0` over a deletion nobody counted is a
    /// figure this card would be making up. The minus is U+2212, the character
    /// every other diff count in this product uses.
    static func diff(insertions: Int?, deletions: Int?) -> String? {
        guard let insertions, let deletions else { return nil }
        return "+\(insertions) −\(deletions)"
    }

    /// The second line of a row's figures.
    ///
    /// Commits when there are any, and how long ago the row last spoke when
    /// there are not. Two different facts in one slot because the slot is 64
    /// points wide and both are worth more than an empty line — and because
    /// they do not compete: a row with commits has plainly been working, and
    /// the staleness a person actually needs is in the ring, which dashes on
    /// exactly the rule `FleetSnapshot.confidence` states.
    ///
    /// `GlanceAge.fresh` is the threshold and it is quoted rather than chosen:
    /// two minutes is where this product's surfaces start saying how old a
    /// thing is. Under it a row says nothing, because `as of 0m` is noise.
    /// Zero commits is a measurement of nothing and gets the age instead.
    static func footnote(_ row: AgentCardRow, at now: Date) -> String? {
        if let commits = row.commits, commits > 0 {
            return "\(commits) commit\(commits == 1 ? "" : "s")"
        }
        guard row.updatedAt != nil else { return nil }
        let age = row.age(at: now)
        guard age >= GlanceAge.fresh else { return nil }
        return "as of \(GlanceAge.brief(age))"
    }

    /// Which tier the header's ring is about. The product's own precedence:
    /// blocked beats review beats working.
    ///
    /// A status WORD rather than a `FleetSnapshot.Glance`, which is the type
    /// that looks right here and is not: its `review` rung counts WORKSPACES
    /// whose diff moved, as its own doc says, while the relay's `review` counts
    /// agents whose turn is over — `all.filter(row => row.status === 'done')` in
    /// `services/relay/src/index.ts`. Two counts of different things, and
    /// borrowing the enum would put that confusion in a type that is about to be
    /// read by three other surfaces. The word goes through the one status→mark
    /// switch instead.
    static func tier(_ state: AgentCardState) -> String {
        if state.blocked > 0 { return "blocked" }
        if state.review > 0 { return "done" }
        return "working"
    }

    /// One ring per agent, blocked first, capped at `ringsDrawn`.
    ///
    /// `withoutCore` on every one of them, and that is §03's own vocabulary
    /// rather than a shortcut: the core is the agent's side of the mark, the
    /// counts say nothing about any single agent, and nil is a surface DECLINING
    /// to state an axis rather than stating that the agent is at a prompt.
    static func rings(_ state: AgentCardState) -> [GlanceMark] {
        let tiers = [(state.blocked, "blocked"), (state.review, "done"), (state.working, "working")]
        var marks: [GlanceMark] = []
        for (count, status) in tiers where count > 0 {
            for _ in 0..<count {
                guard marks.count < Self.ringsDrawn else { return marks }
                marks.append(GlanceMark(status: status).withoutCore)
            }
        }
        return marks
    }
}
