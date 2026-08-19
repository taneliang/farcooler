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
        public var activityChangedAt: Date?

        public init(
            id: String, label: String, machine: String, status: String,
            glyph: String, headline: String, line: String, feed: [String],
            rank: UInt32, turnFailed: Bool, activityChangedAt: Date?
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
        }

        /// Whether this status stays true as the snapshot ages.
        ///
        /// Blocked and done are facts about something that already happened and
        /// that only a person un-does. Working is a claim about right now, and
        /// right now passes.
        public var isLatched: Bool { status == "blocked" || status == "done" }
    }

    public var agents: [Agent]
    public var capturedAt: Date
    /// Whether these are all the agents there are.
    ///
    /// True only after a real fleet poll. A snapshot assembled from pushes
    /// knows about the agents that happened to notify, and a surface that drew
    /// it as the fleet would be asserting that the others do not exist.
    public var complete: Bool

    public init(agents: [Agent], capturedAt: Date, complete: Bool) {
        self.agents = agents
        self.capturedAt = capturedAt
        self.complete = complete
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

    /// How old what is known about ONE agent is.
    ///
    /// The agent's own timestamp when the host sent one, and the snapshot's
    /// capture only when it did not. The two part company every time a push
    /// arrives: `merging` re-assembles the snapshot at `now`, so an age measured
    /// from `capturedAt` alone said that news about agent A was also news about
    /// B through F. An agent last actually seen six hours ago came back to
    /// `.known` and the widget asserted it again — which defeats the staleness
    /// rule these surfaces are built around.
    ///
    /// A daemon too old to send `activitySince` falls back to `capturedAt` and
    /// gets exactly the behavior it always had, including that flaw. There is no
    /// honest alternative: the only thing that could date it is the host.
    public func age(of agent: Agent, at now: Date) -> TimeInterval {
        max(0, now.timeIntervalSince(agent.activityChangedAt ?? capturedAt))
    }

    /// Whether a surface may still state this agent's status as current.
    ///
    /// Judged per agent rather than per snapshot — see `age(of:at:)`.
    ///
    /// One cost is deliberate and is in the safe direction: an agent that has
    /// held one state for longer than `staleAfter` reads as "last seen" even on
    /// a snapshot polled a second ago, because the only date the host gives for
    /// it is when that state began. Understating what is known makes a widget
    /// say less; overstating it makes a widget say something false about the one
    /// thing it exists to report.
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
    /// `age(of:at:)` — so agents go stale at different moments, and a timeline
    /// that stopped at the earliest one would render that entry from then on
    /// with every LATER agent still reading as current. Permanently, on the
    /// exact case this exists for: a working push sends no alert, so there is no
    /// extension run and no reload to rescue it.
    ///
    /// Bounded by construction, which is what makes it safe to return a list:
    /// no timestamp is in the future, so every moment here falls inside
    /// `staleAfter` of `now`.
    public func stalenessMoments(after now: Date) -> [Date] {
        let moments = agents
            .filter { !$0.isLatched }
            .map { ($0.activityChangedAt ?? capturedAt).addingTimeInterval(Self.staleAfter) }
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
    public var ranked: [Agent] {
        agents.sorted { ($0.rank, $0.id) < ($1.rank, $1.id) }
    }

    /// How many agents are waiting on a person. The number a small widget shows.
    public var needingYou: Int { agents.filter { $0.status == "blocked" }.count }

    /// Fold in the one agent a push was about, keeping every other.
    ///
    /// `complete` is carried through rather than recomputed: merging does not
    /// discover agents, so a partial snapshot stays partial however many
    /// pushes arrive, and a complete one stays complete.
    ///
    /// `capturedAt` moves to `now` and keeps meaning what it has always meant —
    /// when this file was last assembled, which is what the widgets' "as of"
    /// footer reports and what tells a real snapshot from `empty`. It is NOT
    /// what vouches for an agent any more: `confidence(in:at:)` asks each agent
    /// for its own timestamp first, so folding in news about one agent no longer
    /// silently re-dates the rest.
    public func merging(_ agent: Agent, at now: Date) -> FleetSnapshot {
        var incoming = agent
        // The agent this push is about IS current, whatever else in here is not.
        // Stamped here rather than left to each caller so every path through
        // this function leaves the fold-in vouched for and the others alone.
        if incoming.activityChangedAt == nil { incoming.activityChangedAt = now }

        var merged = agents
        if let index = merged.firstIndex(where: { $0.id == incoming.id }) {
            merged[index] = incoming
        } else {
            merged.append(incoming)
        }
        return FleetSnapshot(agents: merged, capturedAt: now, complete: complete)
    }
}
