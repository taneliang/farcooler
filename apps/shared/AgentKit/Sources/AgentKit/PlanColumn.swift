import Foundation

// The plan column (ov-867, C10f): what the plan side says. The Rust core
// decides it (`crates/client/src/plan_view/column.rs`, ov-1016) and sends it as
// the plan view's `column`; this is that answer as values, which the Mac and
// the phones draw. The mental model is `.claude/agent/reports/ov-867/intent.md`:
//
// 1. the facts line: what needs you, what's stuck, the next landing, and how
//    much of the board is done;
// 2. Initiatives (today's themes): the larger goals, each with its goal,
//    progress and next step;
// 3. the pipeline: status, then agent, then tickets. Each agent says what it
//    does now, and its tickets are listed under it. A ticket's initiative is
//    the ticket's, never the agent's;
// 4. the footer counts, which open the ticket lists.
//
// Needs You sits between the facts and Initiatives, but its rows are the
// runner's own list, which the plan doesn't carry, so the views add them.

public struct PlanColumn: Decodable, Equatable, Sendable {
    // MARK: The facts line

    /// One fact on the facts line, as runs of text: the number is strong.
    public struct Fact: Decodable, Equatable, Sendable, Identifiable {
        public struct Run: Decodable, Equatable, Sendable {
            public var text: String
            public var strong: Bool
        }
        /// The raw value is the app's own name, which accessibility ids
        /// carry; the view's snake-case word decodes into it.
        public enum Kind: String, Decodable, Sendable {
            case needsYou, stuck, nextLanding, done

            public init(from decoder: Decoder) throws {
                let word = try decoder.singleValueContainer().decode(String.self)
                guard let kind = Kind(rawValue: PlanColumn.camel(word)) else {
                    throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: word))
                }
                self = kind
            }
        }
        public var kind: Kind
        public var runs: [Run]
        /// Drawn in amber: it needs the owner (above 0) or is stuck.
        public var attention: Bool
        /// The fact as one string, for a label.
        public var text: String
        public var id: String { kind.rawValue }
    }

    // MARK: Initiatives

    public struct Initiative: Decodable, Equatable, Sendable, Identifiable {
        /// The theme's id: an initiative is today's theme.
        public var id: String
        public var name: String
        /// The outcome the theme was written for; empty when it has none.
        public var goal: String
        public var done: Int
        public var total: Int
        /// "20 of 29".
        public var progress: String
        /// How full its meter is, 0 to 1.
        public var fraction: Double
        /// What's next for it, or what waits on the owner (`asksOwner`).
        public var next: String?
        /// `next` is the owner's to act on: drawn in amber.
        public var asksOwner: Bool
    }

    // MARK: The pipeline

    /// Where an agent's work stands, in the order the pipeline lists them.
    /// The raw value names a stage's list in the apps' navigation.
    public enum Stage: String, CaseIterable, Decodable, Sendable {
        case stuck, building, reviewing, landing

        /// A stage's list's title, for a list opened by name when its stage
        /// has emptied; a group in the column says its own (`Group.title`).
        public var title: String {
            switch self {
            case .stuck: "Stuck"
            case .building: "Building"
            case .reviewing: "Reviewing"
            case .landing: "Landing"
            }
        }
    }

    /// What an agent does.
    public enum Role: String, Decodable, Sendable {
        case builder, reviewer, lander, agent

        /// Its glyph, an SF Symbol.
        public var symbol: String {
            switch self {
            case .builder: "hammer"
            case .reviewer: "eye"
            case .lander: "arrow.down.to.line"
            case .agent: "sparkle"
            }
        }
    }

    public struct Ticket: Decodable, Equatable, Sendable, Identifiable {
        public var task: String
        public var key: String
        /// Its title, else the board's, else its key.
        public var title: String
        /// The initiative the ticket is for, if any: the ticket's own.
        public var initiative: String?
        public var initiativeID: String?
        public var stage: Stage
        /// When it entered its stage, in ms; 0 when nothing says.
        public var since: Int64
        public var id: String { task }

        private enum CodingKeys: String, CodingKey {
            case task, key, title, initiative, stage, since
            case initiativeID = "initiativeId"
        }
    }

    /// One agent and the tickets it works.
    public struct Agent: Decodable, Equatable, Sendable, Identifiable {
        public var id: String
        public var role: Role
        /// "Builder 2": its role, numbered when there are several.
        public var name: String
        /// What it does now, in its own words; the problem when it is stuck.
        public var doing: String
        /// `doing` says why it's stuck: drawn in amber.
        public var doingIsTrouble: Bool
        /// How long it has been at it, in ms; nil when nothing says.
        public var elapsed: Int64?
        /// "47 min", or nil.
        public var time: String?
        /// Its pane, to open its chat; nil when the runner names none.
        public var terminal: String?
        /// The first ticket it works, for its page when there's no pane to
        /// open.
        public var ticket: String?
        public var tickets: [Ticket]
        /// It is working now: its glyph shows it, and its tickets never do.
        public var working: Bool
        /// Tickets in this stage nobody is on, said under the last card.
        public var more: Int
    }

    public struct Group: Decodable, Equatable, Sendable, Identifiable {
        public var stage: Stage
        public var title: String
        public var agents: [Agent]
        /// Tickets in this stage with no agent on them.
        public var unattended: [Ticket]
        /// The tickets in this stage, each once.
        public var count: Int
        /// The unattended tickets drawn in place: the first few in a stage
        /// with no agent; none in a stage with agents, whose "more" line opens
        /// them all.
        public var preview: [Ticket]
        /// The unattended tickets only the list shows.
        public var hidden: Int
        /// "12 more waiting for review": the tickets with nobody on them.
        public var unattendedWords: String?
        public var id: String { stage.rawValue }
    }

    // MARK: The footer

    /// A ticket waiting for a builder, as the footer's list draws it.
    public struct Waiting: Decodable, Equatable, Sendable, Identifiable {
        public var task: String
        public var key: String
        public var title: String
        /// When it began waiting, in ms.
        public var since: Int64
        public var id: String { task }
    }

    public struct Count: Decodable, Equatable, Sendable, Identifiable {
        /// The raw value names a footer's list in the apps' navigation and its
        /// accessibility id; the view's snake-case word decodes into it.
        public enum Kind: String, Decodable, Sendable {
            case waiting, notStarted, done

            public init(from decoder: Decoder) throws {
                let word = try decoder.singleValueContainer().decode(String.self)
                guard let kind = Kind(rawValue: PlanColumn.camel(word)) else {
                    throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: word))
                }
                self = kind
            }
        }
        public var kind: Kind
        public var count: Int
        /// The board's list it opens, as the store spells a status.
        public var statusWord: String
        /// "10 tickets waiting for a builder", "22 not started", "796 done".
        public var words: String
        public var id: String { kind.rawValue }
        /// The board's list it opens.
        public var status: TaskStatus { TaskStatus(rawValue: statusWord) ?? .todo }

        private enum CodingKeys: String, CodingKey {
            case kind, count, words
            case statusWord = "status"
        }
    }

    public var facts: [Fact]
    /// The initiatives in motion or waiting on the owner, in the board's order.
    public var initiatives: [Initiative]
    /// The rest that are open, said on one quiet line.
    public var quietInitiatives: [Initiative]
    /// "Nothing moving: Mac look and feel, 33 of 34; Marketing, 6 of 8".
    public var quietLine: String?
    /// How many initiatives are open: the section's count.
    public var initiativeCount: Int
    /// The stages with anything in them, in order.
    public var pipeline: [Group]
    /// "10 agents", "1 agent".
    public var agentWords: String
    public var footer: [Count]
    /// The tickets waiting for a builder (the footer's first list), each once,
    /// in the plan's order.
    public var waiting: [Waiting]

    /// The column of `plan` as read from the runner, or nil when it has no
    /// view (a plan made in code, or no bridge).
    /// - Parameters:
    ///   - needsYou: the workspace's Needs You count, the theme asks included.
    ///   - statuses: the board's card statuses by task id.
    ///   - titles: the board's card titles by task id, for a card the plan
    ///     doesn't carry.
    public static func of(
        _ plan: PlanModel, needsYou: Int = 0, statuses: [String: TaskStatus] = [:], titles: [String: String] = [:]
    ) -> PlanColumn? {
        PlanView.build(plan: plan, context: PlanViewContext(needsYou: needsYou, statuses: statuses, titles: titles))?.column
    }
}

extension PlanColumn {
    /// "not_started" as "notStarted".
    static func camel(_ word: String) -> String {
        let parts = word.split(separator: "_")
        return (parts.first.map(String.init) ?? "") + parts.dropFirst().map { $0.prefix(1).uppercased() + $0.dropFirst() }.joined()
    }

    /// What a Needs You row puts in the orchestrator's input when clicked:
    /// a reference to it, for the owner to finish (unsent).
    public static func quote(key: String?, title: String) -> String {
        if let key, !key.isEmpty { return AskAboutTask.draft(key: key, title: title) }
        return "About \u{201C}\(AskAboutTask.oneLine(title))\u{201D}: "
    }
}
