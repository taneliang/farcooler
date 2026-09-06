import Foundation

/// What the lock screen card says, as the relay encodes it.
///
/// **At file scope and outside the `os(iOS)` guard below, deliberately.** It
/// lived inside `AgentActivityAttributes`, which cannot compile on macOS —
/// `ActivityAttributes` is unavailable there — so this type was invisible to
/// `swift test`, which runs on macOS, and had no tests at all for its whole
/// life. That is not an acceptable place for a wire contract three programs
/// have to agree on down to the field names, and it is the one part of that
/// contract with real logic in it: a hand-written decoder that has to tell
/// Unix seconds from milliseconds by magnitude, default every string rather
/// than throw, and tell an absent count from a zero one.
///
/// `AgentActivityAttributes.ContentState` is a typealias to this, so the wire
/// is unchanged and so is every reader: ActivityKit matches a push to a running
/// activity by the `attributes-type` string and decodes `content-state` into
/// the associated type, and a typealias satisfies that exactly as a nested
/// struct did. Nothing about the payload moved.

/// The card's whole content: which agent it is leading with, and what
/// that agent is doing.
///
/// Every field here was on the ATTRIBUTES until the card became
/// per-install. That move is the substance of this type's change and it
/// is not cosmetic: attributes are an activity's identity, APNs rejects
/// a push that repeats them after the start, and a card whose leader
/// lived in its identity could therefore never change leader. It would
/// have had to be ended and restarted, which is two cards on the lock
/// screen for the moment in between — the exact failure one card per
/// install exists to remove.
///
/// **Size.** ActivityKit caps a content state at 4KB encoded, and every
/// field here is bounded well below that by the sender rather than by
/// hope: `detail` is composed and cut on the host at
/// `farcooler_core::feed::WIDTH` (40) or `SAID_WIDTH` (120), `terminal`
/// is a UUID, and `label` and `machine` are a preset name and a runner
/// name. `services/relay/test/relay.test.ts` measures the encoded state
/// of a deliberately oversized push rather than trusting that
/// arithmetic, because the failure mode is not a truncated card — it is
/// APNs refusing the push, which looks from every side like a relay that
/// sent nothing.
public struct AgentCardState: Codable, Hashable, Sendable {
    /// The leading agent's terminal id, so tapping the card opens the
    /// one it is about rather than the fleet.
    ///
    /// On the state rather than the attributes now, and that is what
    /// makes `.widgetURL` correct across a change of leader: the URL is
    /// rebuilt from this on every render, so a card that started
    /// leading with one agent and now leads with another opens the
    /// second one.
    ///
    /// May be empty. A card started by a build older than this one has
    /// no such key in its persisted state — see `init(from:)` — and a
    /// card with no terminal simply gets no tap target, which is better
    /// than a link to `://terminal/`, which opens nothing and cannot be
    /// told apart from a card that ignored the tap.
    public var terminal: String

    /// The leading agent's name, shown as the card's title. Sent on its
    /// own rather than read out of the notification title, which is a
    /// whole sentence ("orchard needs you") and cannot be split back
    /// apart reliably.
    public var label: String

    /// Which runner the leading agent is running on.
    ///
    /// The field keeps the old spelling because the relay encodes this
    /// payload by field name — see the note on the type above. It is
    /// **runner** everywhere a person reads it and `machine` everywhere
    /// a machine does, and those two facts have to be held apart
    /// deliberately or the rename that looks like tidying takes push
    /// down.
    public var machine: String

    /// `working`, `blocked`, or `done`.
    ///
    /// A String rather than an enum, deliberately. The daemon and the
    /// relay are not Swift, so an enum here would be a fourth place that
    /// has to be taught every new state, and a value none of them knew
    /// about would fail to decode — taking the whole card down rather
    /// than one word of it. `AgentStatus` below folds anything
    /// unrecognized into "Working", so an older phone meeting a newer
    /// daemon shows something sensible instead of nothing.
    public var status: String

    /// The line under the title, in every presentation.
    ///
    /// Whatever the runner composed: the question for a blocked card,
    /// the signal line for a working one — both arrive as the notice's
    /// `subtitle` and the relay puts both here. May be empty; `Done` has
    /// nothing to add that the title has not already said.
    ///
    /// There is exactly ONE such field on purpose. A second one, `line`,
    /// stood beside this for a while: read by the widget, declared by
    /// nothing in `services/relay/src/push.ts`, and therefore never
    /// written — its own doc comment claimed older relays omitted it,
    /// which implied newer ones sent it, which was never true. The card
    /// has room for one line and the runner composes one line, so a
    /// second field could only ever have been a copy of this one that
    /// two senders could disagree about.
    ///
    /// **That precedent is why there were no fleet counts here**, and
    /// why the ones below took a change on the other side of the wire
    /// rather than a change of mind. The objection was never about the
    /// shape of the field: it was that nothing could write one honestly.
    /// The relay stored no fleet and a daemon knows only its own runner,
    /// so a count here would have been `line` again, in a form that is
    /// harder to notice because a plausible number is indistinguishable
    /// from a true one.
    ///
    /// The relay accumulates a row per agent now, from the notices every
    /// runner on an account already sends it — which is a thing only it
    /// can see. So the counts have a writer, and it is the one place in
    /// the system that has ever seen a whole fleet awake. See
    /// `services/relay/migrations/0008_fleet_rows.sql`.
    public var detail: String

    /// When the LEADER's turn started, so the card can show its own
    /// clock.
    ///
    /// It moved here from the attributes with the rest of the leader,
    /// and the reason it lived there is worth keeping straight because
    /// it is no longer true: it was on the attributes because it did not
    /// change for the life of the card. It changes now — a new leader is
    /// a new turn — and it has to, or the card would count a second
    /// agent's work from the first agent's start.
    ///
    /// What did NOT change is why it is a date rather than a string:
    /// ActivityKit renders a timer from a date natively, so there is no
    /// push per tick and it keeps counting while the phone is off the
    /// network entirely. A string computed in the extension freezes at
    /// the last push, because an extension gets no per-second wake-up.
    ///
    /// Optional, which here is also what makes it safe: an absent key
    /// decodes to nil rather than throwing, so a state sent by a relay
    /// that omits it still decodes. A card for an agent whose turn clock
    /// the runner could not read shows no timer, which is better than
    /// showing a wrong one.
    ///
    /// On the wire it is a NUMBER, not a date string — see `init(from:)`.
    public var startedAt: Date?

    /// How many agents on this account are in each tier, and how many
    /// the card has no line for.
    ///
    /// **Over the whole fleet, not over what fits.** A count that only
    /// counted the drawn rows would say "2 need you" while three agents
    /// were waiting, which is the specific lie the tail exists to
    /// prevent — and it is the same lie the App Group snapshot tells
    /// whenever `complete` is false, which is why every surface reading
    /// that has had to qualify itself.
    ///
    /// `-1` for "the relay did not say", which is what an absent key
    /// decodes to and what every card started before this carries. It
    /// has to be distinguishable from zero: a fleet with nothing blocked
    /// and a relay that never mentioned blocking are different facts, and
    /// only one of them entitles the card to state a number. See
    /// `knowsFleet`.
    ///
    /// The relay also sends `more` — the fleet minus the ROWS it sent — and
    /// this type deliberately does not decode it. This card draws one agent
    /// and counts the rest, so a number computed against four rows it never
    /// draws would say "+3 more" over six agents. `more` belongs to the card
    /// that draws a line each, and a field with no reader is the mistake this
    /// type has been burned by once already: see the note on `detail`.
    public var blocked: Int
    public var review: Int
    public var working: Int

    /// What the whole fleet has changed: `+391 −112`, and the commits
    /// under it.
    ///
    /// `nil` where nobody measured, which is not zero and must not be
    /// drawn as one. A worktree the runner has not probed and one with no
    /// base to compare against have both said nothing, and the relay
    /// carries that absence all the way from `review::Counts` rather than
    /// flattening it on the way — so a card with no numbers draws none
    /// rather than claiming a diff of nothing.
    public var insertions: Int?
    public var deletions: Int?
    public var commits: Int?

    /// Whether the relay told this card anything about its fleet.
    ///
    /// The one guard every count below needs. A card started by a relay
    /// older than the roster carries none of these keys, decodes them to
    /// `-1`, and must draw the tail the way it always did — off the App
    /// Group snapshot — rather than announce a fleet of minus one.
    public var knowsFleet: Bool { blocked >= 0 && working >= 0 }

    public init(
        terminal: String = "",
        label: String = "",
        machine: String = "",
        status: String,
        detail: String,
        startedAt: Date? = nil,
        blocked: Int = -1,
        review: Int = -1,
        working: Int = -1,
        insertions: Int? = nil,
        deletions: Int? = nil,
        commits: Int? = nil
    ) {
        self.terminal = terminal
        self.label = label
        self.machine = machine
        self.status = status
        self.detail = detail
        self.startedAt = startedAt
        self.blocked = blocked
        self.review = review
        self.working = working
        self.insertions = insertions
        self.deletions = deletions
        self.commits = commits
    }

    private enum CodingKeys: String, CodingKey {
        case terminal, label, machine, status, detail, startedAt
        case blocked, review, working, insertions, deletions, commits
    }

    /// Hand-written for two reasons, and neither is the timestamp alone.
    ///
    /// **Every string defaults rather than throws.** A card started by a
    /// build older than this one has a persisted content state of
    /// `{status, detail}` and nothing else, and ActivityKit decodes that
    /// state back into THIS type when the upgraded app enumerates
    /// `Activity.activities`. A strict decode throws there, the activity
    /// is missing from the list, and `LiveActivities.reapDuplicates`
    /// cannot end a card it is never shown — leaving a terminal-scoped
    /// card from the old build sitting beside the new fleet-scoped one
    /// for the life of its stale date. That is precisely the upgrade
    /// this restructure has to survive, so the leniency is the feature.
    ///
    /// **`Date` has no wire format of its own.** Swift's default is
    /// seconds since 2001, which nothing on the other side of this seam
    /// produces: the daemon's turn clock is Unix MILLISECONDS (the
    /// phone's `Terminal.turnStartedAt`) while every other timestamp in
    /// `services/relay/src/push.ts` is Unix SECONDS, so this is exactly
    /// where the two conventions meet. Left to the default, a plausible
    /// number decodes to a date decades or millennia out and the card
    /// counts nonsense — a wrong timer, which is the one thing
    /// `startedAt` being optional was meant to avoid. So both are
    /// accepted and told apart by magnitude: no second count this side
    /// of the year 5000 reaches 1e11, and no millisecond count since
    /// 1973 falls below it.
    ///
    /// `try?` on that one field rather than `try`: a value in a shape
    /// this does not expect — a date STRING, say — must cost the timer
    /// and not the card.
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        terminal = (try? container.decodeIfPresent(String.self, forKey: .terminal)) ?? ""
        label = (try? container.decodeIfPresent(String.self, forKey: .label)) ?? ""
        machine = (try? container.decodeIfPresent(String.self, forKey: .machine)) ?? ""
        status = (try? container.decodeIfPresent(String.self, forKey: .status)) ?? ""
        detail = (try? container.decodeIfPresent(String.self, forKey: .detail)) ?? ""
        let stamp = (try? container.decodeIfPresent(Double.self, forKey: .startedAt)) ?? nil
        startedAt = stamp.flatMap { value in
            guard value > 0 else { return nil }
            return Date(timeIntervalSince1970: value > 1e11 ? value / 1000 : value)
        }
        // `-1` and not `0` for an absent count, which is the whole of
        // `knowsFleet`: a relay too old to have a roster says nothing
        // here, and a card that read that as "nobody is blocked" would
        // state a fact about a fleet nothing had counted. Zero is a real
        // answer and absence is not, exactly as `insertions` below.
        func count(_ key: CodingKeys) -> Int {
            ((try? container.decodeIfPresent(Int.self, forKey: key)) ?? nil) ?? -1
        }
        blocked = count(.blocked)
        review = count(.review)
        working = count(.working)
        insertions = (try? container.decodeIfPresent(Int.self, forKey: .insertions)) ?? nil
        deletions = (try? container.decodeIfPresent(Int.self, forKey: .deletions)) ?? nil
        commits = (try? container.decodeIfPresent(Int.self, forKey: .commits)) ?? nil
    }

    /// The other half of the same decision, and it is not decorative
    /// here the way it was on the attributes: ActivityKit persists a
    /// content state by encoding it and reads it back by decoding it, so
    /// a pair that wrote seconds-since-2001 and read milliseconds would
    /// move every card's timer by decades across a single app launch.
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(terminal, forKey: .terminal)
        try container.encode(label, forKey: .label)
        try container.encode(machine, forKey: .machine)
        try container.encode(status, forKey: .status)
        try container.encode(detail, forKey: .detail)
        try container.encodeIfPresent(
            startedAt.map { $0.timeIntervalSince1970 * 1000 }, forKey: .startedAt)
        // Written only when they are real, so that a persisted card
        // round-trips to the same `knowsFleet` it was decoded with.
        // Encoding `-1` would turn "the relay said nothing" into a stored
        // number, and the next decode would read it back as an answer.
        if knowsFleet {
            try container.encode(blocked, forKey: .blocked)
            try container.encode(review, forKey: .review)
            try container.encode(working, forKey: .working)
        }
        try container.encodeIfPresent(insertions, forKey: .insertions)
        try container.encodeIfPresent(deletions, forKey: .deletions)
        try container.encodeIfPresent(commits, forKey: .commits)
    }
}


#if os(iOS)
    import ActivityKit

    /// What the lock screen shows while agents are working, and the contract the
    /// relay encodes against.
    ///
    /// Shared between the app and the widget extension because those are two
    /// separate binaries that have to agree on this type down to the field
    /// names. ActivityKit matches an incoming push to a running activity by the
    /// `attributes-type` string in the payload and decodes `content-state`
    /// straight into `ContentState`, so a field renamed on one side and not the
    /// other does not fail to build — the push simply stops arriving. That is
    /// the hardest kind of break to notice, which is why this lives in one file
    /// that both targets compile rather than in each of them.
    ///
    /// `services/relay/src/push.ts` writes the other end of this contract.
    ///
    /// **There is ONE card per app install**, and that is the shape everything
    /// below follows from. It used to be one card per terminal, which put four
    /// stacked cards on the lock screen for four running agents while the
    /// Dynamic Island — which can present exactly one — picked whichever it
    /// liked. So the card now LEADS with one agent and counts the rest, and
    /// because the leader changes over the card's life as different agents block
    /// and finish, the leader is part of the CONTENT STATE. The attributes are
    /// what is fixed for the install's card, which is very nearly nothing.
    ///
    /// Guarded because AgentKit is also the Mac app's package. `os(iOS)` and not
    /// `canImport(ActivityKit)`, which is the guard that looks right and is not:
    /// ActivityKit does import on macOS, and `ActivityAttributes` is then marked
    /// unavailable there, so the Mac build fails on a protocol it can see and
    /// cannot conform to. The guard, not a separate module — splitting one
    /// struct into its own target buys nothing and gives the two clients a
    /// second place to drift apart.
    public struct AgentActivityAttributes: ActivityAttributes {
        /// The card's whole content, defined above this guard so that it can be
        /// TESTED.
        ///
        /// A typealias and not a nested struct, and that is the only thing about
        /// it that changed: ActivityKit needs an associated type of this name on
        /// this protocol, and it does not care whether the name is a declaration
        /// or an alias. The payload, the field names and every reader are
        /// untouched. See `AgentCardState`, which says why it moved.
        public typealias ContentState = AgentCardState

        /// Which SHAPE this card was started in.
        ///
        /// The one thing that is genuinely fixed for the life of an install's
        /// card, and it exists for exactly one job: telling a card started by an
        /// older build apart from one started by this one.
        ///
        /// It has to be told rather than worked out. Both shapes are the same
        /// ActivityKit type under the same `attributes-type` string — they have
        /// to be, or a phone mid-upgrade would stop matching pushes to the cards
        /// it already has — and `ContentState` above decodes an old card
        /// leniently, so "the label is empty" is a guess and not an answer. A
        /// version is an answer, and `LiveActivities.precedence` uses it to make
        /// sure the card that survives an upgrade is the fleet-shaped one rather
        /// than a terminal-scoped leftover.
        ///
        /// Absent decodes to `legacy`, which is what every in-flight card from
        /// before this field is.
        public var version: Int

        /// What a card started before the fleet restructure reads as.
        public static let legacyVersion = 1

        /// A card that headlines one agent and counts the rest off this phone's
        /// own snapshot.
        public static let fleetVersion = 2

        /// A card whose counts come from the relay's roster.
        ///
        /// What this build starts, and what the relay sends. It is the third
        /// value of a RANK rather than a second flag, which is the whole reason
        /// a third shape needed no new field: higher is a card this build
        /// understands better, and `precedence` compares the number.
        ///
        /// Must equal `ACTIVITY_VERSION` in `services/relay/src/push.ts`.
        public static let rowsVersion = 3

        // There was an `isFleetShaped` here, and its one reader — the card
        // `LiveActivities.precedence` keeps when two are in flight — now compares
        // `version` itself. A boolean over a rank of three could only say
        // "second shape or later", which would rank a card counting the fleet
        // off a stale local file equal to one the relay counted; and a property
        // nothing reads is the shape of mistake this type has been burned by
        // once already. See the note on `detail`.

        public init(version: Int = AgentActivityAttributes.rowsVersion) {
            self.version = version
        }

        private enum CodingKeys: String, CodingKey {
            case version
        }

        /// Lenient for the same reason `ContentState.init(from:)` is, and the
        /// stakes are higher here: attributes that fail to decode do not merely
        /// lose a field, they keep the whole activity out of
        /// `Activity.activities` — so a strict read of a key that older cards do
        /// not carry would make exactly the cards this field exists to retire
        /// the ones nothing can reach.
        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            version =
                ((try? container.decodeIfPresent(Int.self, forKey: .version)) ?? nil)
                ?? Self.legacyVersion
        }

        public func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(version, forKey: .version)
        }
    }

    /// The three states a card can be in, and the words and color for each.
    ///
    /// Here rather than in the widget so the app and the extension describe a
    /// state identically. The widget draws it; the app uses it to decide whether
    /// an activity is finished and should stop being tracked.
    public enum AgentStatus: String, Sendable {
        case working
        case blocked
        case done

        /// Anything unrecognized is treated as working — see `ContentState.status`.
        public init(_ raw: String) {
            self = AgentStatus(rawValue: raw) ?? .working
        }

        public var title: String {
            switch self {
            case .working: "Working"
            case .blocked: "Needs You"
            case .done: "Finished"
            }
        }

        /// Whether the run is over. The relay ends the activity itself, but a
        /// card can also arrive already finished if the phone was off the
        /// network when the agent stopped.
        public var isFinished: Bool { self == .done }
    }
#endif
