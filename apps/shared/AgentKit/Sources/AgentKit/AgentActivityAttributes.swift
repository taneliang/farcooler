import Foundation

#if os(iOS)
    import ActivityKit

    /// What the lock screen shows while an agent is working, and the contract the
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
    /// Guarded because AgentKit is also the Mac app's package. `os(iOS)` and not
    /// `canImport(ActivityKit)`, which is the guard that looks right and is not:
    /// ActivityKit does import on macOS, and `ActivityAttributes` is then marked
    /// unavailable there, so the Mac build fails on a protocol it can see and
    /// cannot conform to. The guard, not a separate module — splitting one
    /// struct into its own target buys nothing and gives the two clients a
    /// second place to drift apart.
    public struct AgentActivityAttributes: ActivityAttributes {
        public struct ContentState: Codable, Hashable {
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
            public var detail: String

            public init(status: String, detail: String) {
                self.status = status
                self.detail = detail
            }
        }

        /// The terminal's id, so tapping the card can open the one it is about
        /// rather than the fleet.
        public var terminal: String

        /// The agent's name, shown as the card's title. Sent on its own rather
        /// than read out of the notification title, which is a whole sentence
        /// ("orchard needs you") and cannot be split back apart reliably.
        public var label: String

        /// Which runner it is running on.
        ///
        /// The field keeps the old spelling because the relay encodes this
        /// payload by field name — see the note on the type above.
        public var machine: String

        /// When the turn started, so the card can show its own clock.
        ///
        /// On the ATTRIBUTES rather than the state, because it does not change
        /// for the life of the card and because ActivityKit renders a timer
        /// from a date natively — no push per tick, and it keeps counting while
        /// the phone is off the network entirely.
        ///
        /// Optional, which here is also what makes it safe to add: an absent
        /// key decodes to nil rather than throwing, so an activity started by a
        /// relay that sends no such key still matches this type. A card for an
        /// agent whose turn clock the runner could not read shows no timer,
        /// which is better than showing a wrong one.
        ///
        /// On the wire it is a NUMBER, not a date string — see `init(from:)`,
        /// which is where the one part of this contract that cannot be read off
        /// the field names is written down.
        public var startedAt: Date?

        public init(terminal: String, label: String, machine: String, startedAt: Date? = nil) {
            self.terminal = terminal
            self.label = label
            self.machine = machine
            self.startedAt = startedAt
        }

        private enum CodingKeys: String, CodingKey {
            case terminal, label, machine, startedAt
        }

        /// Hand-written for one field, and this is why.
        ///
        /// `Date` has no wire format of its own. Swift's default is seconds
        /// since 2001, which nothing on the other side of this seam produces:
        /// the daemon's turn clock is Unix MILLISECONDS (the phone's
        /// `Terminal.turnStartedAt`) while every other timestamp in
        /// `services/relay/src/push.ts` is Unix SECONDS, so this is exactly
        /// where the two conventions meet. Left to the default, a plausible
        /// number decodes to a date decades or millennia out and the card
        /// counts nonsense — a wrong timer, which is the one thing `startedAt`
        /// being optional was meant to avoid. So both are accepted and told
        /// apart by magnitude: no second count this side of the year 5000
        /// reaches 1e11, and no millisecond count since 1973 falls below it.
        ///
        /// `try?` on that one field rather than `try`: a value in a shape this
        /// does not expect — a date STRING, say — must cost the timer and not
        /// the card. Attributes that fail to decode do not start the activity
        /// at all, so a strict read here would answer a wrong timestamp format
        /// with no lock screen card whatsoever.
        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            terminal = try container.decode(String.self, forKey: .terminal)
            label = try container.decode(String.self, forKey: .label)
            machine = try container.decode(String.self, forKey: .machine)
            let stamp = (try? container.decodeIfPresent(Double.self, forKey: .startedAt)) ?? nil
            startedAt = stamp.flatMap { value in
                guard value > 0 else { return nil }
                return Date(timeIntervalSince1970: value > 1e11 ? value / 1000 : value)
            }
        }

        /// The other half of the same decision. Nothing in the app encodes
        /// these today — attributes only ever arrive from a push — but a
        /// `Codable` that reads milliseconds and writes seconds-since-2001 is
        /// a trap set for whoever first round-trips one.
        public func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(terminal, forKey: .terminal)
            try container.encode(label, forKey: .label)
            try container.encode(machine, forKey: .machine)
            try container.encodeIfPresent(
                startedAt.map { $0.timeIntervalSince1970 * 1000 }, forKey: .startedAt)
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
