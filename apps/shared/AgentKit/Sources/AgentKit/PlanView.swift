import Foundation

// The Plan's sections and ticket cards as the Rust core builds them (ov-1016,
// ruling R-112): `crates/client/src/plan_view.rs` decides which ticket is in
// which section, the order of the landings, and every word a section header,
// a ticket card, a landing card, an agent row and a theme tile says. This is
// that answer decoded; the Mac and the phones only lay it out.
//
// AgentKit never links the Rust library: the watch and the widgets compile a
// subset of these sources and can't. Each app installs a presenter at launch
// (`PlanViewPresenter.install`) that calls `farcooler_client_plan_view`, which
// also keeps that symbol in the app's link. Without one, the presenter looks
// the symbol up in the running image (`dlsym`), which is how AgentKit's own
// tests reach the library their target links.

/// The view model: `{schema, view}`'s view.
public struct PlanView: Decodable, Equatable, Sendable {
    public enum SectionKind: String, Decodable, Sendable {
        case needsYou = "needs_you", landing, inReview = "in_review", building, waiting
        case onLandedLanding = "on_landed_landing", agents, themes, landedToday = "landed_today"
    }

    /// A section to draw, in the order drawn.
    public struct Section: Decodable, Equatable, Identifiable, Sendable {
        public var kind: SectionKind
        public var title: String
        /// Its rows; 0 draws no count.
        public var count: Int
        /// What it says with nothing in it; nil when it has rows or says nothing.
        public var empty: String?
        /// Drawn folded until opened.
        public var folded: Bool
        /// Said on hover over (or a long press of) the title, never its rows.
        public var help: String?
        public var id: SectionKind { kind }
    }

    /// What a ticket's mark shows: its stage, a fix round, or its place in Next Up.
    public enum Mark: String, Decodable, Sendable {
        case rank, waiting, building, fixing, reviewing, needsYou = "needs_you", landing, landed, dropped, unknown
    }

    public enum NoteKind: String, Decodable, Sendable {
        /// The pull request's stage.
        case stage
        /// The engine run's step and gate.
        case run
        /// Waits on the owner, or hasn't moved.
        case warning
        case overBudget = "over_budget"
    }

    /// A line under a ticket's second line.
    public struct Note: Decodable, Equatable, Sendable {
        public var kind: NoteKind
        public var text: String
        /// Drawn in amber.
        public var attention: Bool
        /// Every word of it, for a hover or a long press.
        public var help: String?

        /// The id a view and the UI tests know it by: the stage's (amber or not),
        /// the run's, the warning's and the budget's.
        public func probe(key: String) -> String {
            switch kind {
            case .stage: attention ? "plan-ticket-\(key)-stage-attention" : "plan-ticket-\(key)-stage"
            case .run: "plan-ticket-\(key)-run"
            case .warning: "plan-ticket-\(key)-warning"
            case .overBudget: "plan-ticket-\(key)-over-budget"
            }
        }
    }

    /// The agent working a ticket, or landing a landing.
    public struct AgentRef: Decodable, Equatable, Sendable {
        public var id: String
        /// "Builder · Sonnet".
        public var who: String
        /// "380K"; nil when no turn stated any.
        public var tokens: String?
        /// The pane, to open it.
        public var terminal: String?
        /// The model as recorded, for the screen reader; "" when unknown.
        public var model: String?
    }

    public struct Gate: Decodable, Equatable, Sendable {
        public var label: String
        public var text: String
        public var principal: String
    }

    /// The engine's run of a ticket: the step it is on and the gate it waits at.
    public struct RunStep: Decodable, Equatable, Sendable {
        public var run: String
        public var phase: String
        public var phaseLabel: String
        public var step: String?
        public var gate: Gate?
        public var attention: Bool
    }

    /// A ticket's row: in In Review, Building, Waiting or Waiting for the Next Landing.
    public struct TicketCard: Decodable, Equatable, Identifiable, Sendable {
        /// The card's task id.
        public var id: String
        public var key: String
        /// Its title, else its key.
        public var heading: String
        /// The theme it serves.
        public var theme: String?
        /// The stage, as the plan's wire spells it.
        public var stage: String
        /// "Fixing · round 1", "Waiting · 2nd", "In Review · in Landing 8".
        public var status: String
        /// The line under the title.
        public var second: String
        /// Its place in Next Up, from 1.
        public var rank: Int?
        public var mark: Mark
        /// The mark is drawn in amber.
        public var markAttention: Bool
        public var notes: [Note]
        public var waitsOnOwner: Bool
        /// The landing that landed which it still names.
        public var landedLanding: String?
        public var agent: AgentRef?
        public var run: RunStep?
        public var branch: String
        /// On a landing, the word beside the title: "Needs you" while it waits
        /// on the owner (which `notes` then leads with), "Landed · ac840108"
        /// once it has; nil when it is simply aboard.
        public var note: String?
        /// What a screen reader says for the row.
        public var accessibility: String
    }

    public enum WaitingKind: String, Decodable, Sendable {
        case next, notRanked = "not_ranked", onYou = "on_you"
    }

    /// One of Waiting's groups; only groups with cards are sent.
    public struct WaitingGroup: Decodable, Equatable, Identifiable, Sendable {
        public var kind: WaitingKind
        /// Drawn only when another group shares the section.
        public var title: String
        /// The title is drawn in amber.
        public var attention: Bool
        public var cards: [TicketCard]

        /// The probe id the views and the UI tests use.
        public var id: String {
            switch kind {
            case .next: "plan-next-up"
            case .notRanked: "plan-not-ranked"
            case .onYou: "plan-waiting-on-you"
            }
        }
    }

    public struct LadderStep: Decodable, Equatable, Sendable {
        public var label: String
        public var explanation: String
    }

    /// The ladder a landing climbs: three steps, and the one it stands on.
    public struct Ladder: Decodable, Equatable, Sendable {
        public var steps: [LadderStep]
        /// 0 to 2, or 3 once everything is done.
        public var at: Int
        /// "Checking on CI · step 2 of 3".
        public var compact: String
    }

    public struct LandingCard: Decodable, Equatable, Identifiable, Sendable {
        public var id: String
        public var number: Int
        /// "Landing 86".
        public var label: String
        public var subtitle: String?
        /// Its tickets' titles on one line.
        public var carries: String?
        /// The phase as the plan's wire spells it.
        public var phase: String
        /// "Checking on CI".
        public var phaseWord: String
        /// "checking on CI", inside a sentence.
        public var phaseSentence: String
        public var explanation: String
        /// CI failed, or nothing has moved.
        public var attention: Bool
        /// When it entered this phase, in ms.
        public var since: Int64
        public var ladder: Ladder
        public var lander: AgentRef?
        /// "Lander · Opus · 1.4M".
        public var landerWords: String?
        public var pushedSha: String?
        /// Stands in for CI's line while a pushed commit has no CI read of it yet.
        public var ciUnread: String?
        public var tickets: [TicketCard]
    }

    public struct AgentRow: Decodable, Equatable, Identifiable, Sendable {
        public var id: String
        public var who: String
        public var working: Bool
        public var worksFor: String
        /// "Working · 22 min", "Idle · active 1 h ago".
        public var state: String
        public var second: String
        public var doingNow: String
        public var tokens: String?
        /// "≈$1.10".
        public var cost: String?
        public var terminal: String?
        public var accessibility: String
    }

    public struct Agents: Decodable, Equatable, Sendable {
        /// "5 agents, including the orchestrator · 4 working · …", or "No agents running."
        public var header: String
        /// Working first, then by spend.
        public var rows: [AgentRow]
    }

    public enum Glyph: String, Decodable, Sendable {
        case done, inProgress = "in_progress", notStarted = "not_started"
    }

    public struct TileCard: Decodable, Equatable, Identifiable, Sendable {
        public var id: String
        public var key: String
        public var title: String
        public var glyph: Glyph
    }

    public struct ThemeTile: Decodable, Equatable, Identifiable, Sendable {
        public var id: String
        public var name: String
        public var done: Int
        public var total: Int
        /// "6 of 10 done".
        public var progress: String
        /// How full the meter is, 0 to 1.
        public var fraction: Double
        /// Up to three cards.
        public var cards: [TileCard]
        /// What waits on the owner, the one amber line.
        public var ownerAsk: String?
        /// A ticket of it is landing, in review or building.
        public var moving: Bool
    }

    public struct Themes: Decodable, Equatable, Sendable {
        public var tiles: [ThemeTile]
        /// Done themes, folded into one tile.
        public var finished: [ThemeTile]
        public var finishedTitle: String?
        /// "7 themes".
        public var finishedWords: String?
    }

    public var schema: Int
    /// The runner's clock.
    public var now: Int64
    /// The sections to draw, in order.
    public var sections: [Section]
    public var needsYou: Int
    /// Live landings, furthest along first.
    public var landings: [LandingCard]
    public var inReview: [TicketCard]
    public var building: [TicketCard]
    /// Next Up ranked, Not Ranked, then Waiting on You; empty groups left out.
    public var waiting: [WaitingGroup]
    public var onLandedLanding: [TicketCard]
    /// Landed on the viewer's day, newest first.
    public var landedToday: [TicketCard]
    public var agents: Agents
    public var themes: Themes
    /// The plan column beside the conversation (ov-867), from the same read.
    public var column: PlanColumn

    /// The section of `kind`, when it's drawn.
    public func section(_ kind: SectionKind) -> Section? { sections.first { $0.kind == kind } }

    /// The cards in a Waiting group, or none.
    public func waiting(_ kind: WaitingKind) -> [TicketCard] { waiting.first { $0.kind == kind }?.cards ?? [] }

    /// Every card in Waiting, in its groups' order.
    public var waitingCount: Int { waiting.reduce(0) { $0 + $1.cards.count } }

    /// Each Waiting group is named only when something else shares the section.
    public var namesWaitingGroups: Bool { waiting.count > 1 }

    /// Decode `{schema, view, error}` as the bridge answers; nil for an error.
    public static func decode(answer: Data) -> PlanView? {
        struct Answer: Decodable {
            var view: PlanView?
        }
        return (try? PlanJSON.decoder.decode(Answer.self, from: answer))?.view
    }
}

// MARK: Building one

/// What the app knows beside the plan: the board's own reads.
public struct PlanViewContext: Equatable, Sendable {
    /// The workspace's Needs You count, for the section's row.
    public var needsYou: Int
    /// The board's card statuses by task id.
    public var statuses: [String: TaskStatus]
    /// When each card last moved, in ms, by task id.
    public var activity: [String: Int64]
    /// Whether the runner lists agents (`board_line`).
    public var agentsSupported: Bool
    /// The board's card titles by task id, for a card the plan carries none
    /// for (the column's tickets).
    public var titles: [String: String]
    /// The viewer's offset from UTC in seconds, which says where their day ends.
    public var utcOffset: Int

    public init(
        needsYou: Int = 0, statuses: [String: TaskStatus] = [:], activity: [String: Int64] = [:], agentsSupported: Bool = true,
        titles: [String: String] = [:], utcOffset: Int = TimeZone.current.secondsFromGMT()
    ) {
        self.needsYou = max(0, needsYou)
        self.statuses = statuses
        self.activity = activity
        self.agentsSupported = agentsSupported
        self.titles = titles
        self.utcOffset = utcOffset
    }

    /// The request's `context`, in the Apple apps' style.
    var json: Data {
        let object: [String: Any] = [
            "needs_you": needsYou,
            "statuses": statuses.mapValues(\.rawValue),
            "activity": activity,
            "agents_supported": agentsSupported,
            "titles": titles,
            "utc_offset": utcOffset,
            "style": "apple",
        ]
        return (try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])) ?? Data("{}".utf8)
    }
}

extension PlanView {
    /// The view of `plan` as read from the runner, or nil when it has no wire
    /// form (a plan made in code) or no presenter answers.
    /// - Parameter workflow: `workflow.snapshot`'s presentation, as JSON, when
    ///   the workspace has a published workflow.
    public static func build(plan: PlanModel, context: PlanViewContext = PlanViewContext(), workflow: Data? = nil) -> PlanView? {
        guard let wire = plan.wire else { return nil }
        return PlanViewPresenter.view(wire: wire, context: context, workflow: workflow)
    }

    /// The same, from the separate reads the screens hold.
    public static func build(
        plan: PlanModel, needsYou: Int, statuses: [String: TaskStatus] = [:], activity: [String: Int64] = [:],
        agentsSupported: Bool = true
    ) -> PlanView? {
        build(
            plan: plan,
            context: PlanViewContext(needsYou: needsYou, statuses: statuses, activity: activity, agentsSupported: agentsSupported))
    }
}

/// Calls the Rust core's `farcooler_client_plan_view` (see this file's head).
public enum PlanViewPresenter {
    /// A request in, the answer out; nil when there is nothing to call.
    public typealias Present = @Sendable (String) -> String?

    private final class State: @unchecked Sendable {
        let lock = NSLock()
        var present: Present?
        /// The last few answers, so a view body that asks again doesn't cross
        /// the bridge for the same plan.
        var recent: [(key: Key, view: PlanView?)] = []
    }

    struct Key: Equatable {
        var wire: Data
        var context: PlanViewContext
        var workflow: Data?
    }

    private static let state = State()
    private static let kept = 8

    /// Install the app's call to the bridge. Each app does this at launch.
    public static func install(_ present: @escaping Present) {
        state.lock.lock()
        state.present = present
        state.recent = []
        state.lock.unlock()
    }

    static func view(wire: Data, context: PlanViewContext, workflow: Data?) -> PlanView? {
        let key = Key(wire: wire, context: context, workflow: workflow)
        state.lock.lock()
        if let hit = state.recent.firstIndex(where: { $0.key == key }) {
            let found = state.recent.remove(at: hit)
            state.recent.insert(found, at: 0)
            state.lock.unlock()
            return found.view
        }
        let present = state.present ?? linked
        state.lock.unlock()

        var request = Data(#"{"schema":1,"plan":"#.utf8)
        request.append(wire)
        request.append(Data(#","context":"#.utf8))
        request.append(context.json)
        if let workflow {
            request.append(Data(#","workflow":"#.utf8))
            request.append(workflow)
        }
        request.append(Data("}".utf8))
        let answer = present?(String(decoding: request, as: UTF8.self))
        let view = answer.flatMap { PlanView.decode(answer: Data($0.utf8)) }

        state.lock.lock()
        state.recent.insert((key, view), at: 0)
        if state.recent.count > kept { state.recent.removeLast(state.recent.count - kept) }
        state.lock.unlock()
        return view
    }

    /// The bridge's C signature.
    public typealias Bridge = @convention(c) (UnsafePointer<CChar>?, UnsafeMutablePointer<UInt8>?, Int) -> Int

    /// Call a bridge that answers into a caller's buffer, as every
    /// `farcooler_client_*` answer does: once to learn the length when the
    /// first buffer is short, then again.
    public static func call(_ bridge: Bridge, _ request: String) -> String? {
        request.withCString { input -> String? in
            var capacity = 64 * 1024
            for _ in 0..<2 {
                var buffer = [UInt8](repeating: 0, count: capacity)
                let needed = buffer.withUnsafeMutableBufferPointer { bridge(input, $0.baseAddress, capacity) }
                if needed == 0 { return nil }
                if needed <= capacity { return String(decoding: buffer[0..<needed], as: UTF8.self) }
                capacity = needed
            }
            return nil
        }
    }

    /// The bridge, looked up in the running image: there when whatever runs
    /// this links the Rust library (AgentKit's tests do), else nil.
    private static let linked: Present? = {
        // RTLD_DEFAULT: every image loaded.
        guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "farcooler_client_plan_view") else { return nil }
        let bridge = unsafeBitCast(symbol, to: Bridge.self)
        return { call(bridge, $0) }
    }()
}

extension PlanView.Mark {
    /// The ticket stage a mark stands for, for a glyph: a fix round is a build.
    public var stage: TicketStage {
        switch self {
        case .waiting: .waiting
        case .building, .fixing: .building
        case .reviewing: .reviewing
        case .needsYou: .needsYou
        case .landing: .landing
        case .landed: .landed
        case .dropped: .dropped
        case .rank, .unknown: .unknown
        }
    }
}
