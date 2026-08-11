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

            /// The line under the title. May be empty — `Done` has nothing to
            /// add that the title has not already said.
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

        /// Which machine it is running on.
        public var machine: String

        public init(terminal: String, label: String, machine: String) {
            self.terminal = terminal
            self.label = label
            self.machine = machine
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
