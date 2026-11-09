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

/// The view model: `{schema, view}`'s view. The types are generated from the Rust
/// ones (`WireTypes.swift`, ov-1035); this file names them the way the apps do and adds
/// what only a screen needs.
public typealias PlanView = WirePlanView

extension WirePlanView {
    public typealias SectionKind = WireSectionKind
    public typealias Section = WireSection
    public typealias Mark = WireMark
    public typealias NoteKind = WireNoteKind
    public typealias Note = WireNote
    public typealias AgentRef = WireAgentRef
    public typealias Gate = WireGate
    public typealias RunStep = WireRunStep
    public typealias TicketCard = WireTicketCard
    public typealias WaitingKind = WireWaitingKind
    public typealias WaitingGroup = WireWaitingGroup
    public typealias LadderStep = WireLadderStep
    public typealias Ladder = WireLadder
    public typealias LandingCard = WireLandingCard
    public typealias AgentRow = WireAgentRow
    public typealias Agents = WireAgents
    public typealias Glyph = WireGlyph
    public typealias TileCard = WireTileCard
    public typealias ThemeTile = WireThemeTile
    public typealias DetailStage = WireDetailStage
    public typealias TicketDetail = WireTicketDetail
    public typealias TicketDetails = WireTicketDetails
    public typealias Themes = WireThemes

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
        return (try? JSONDecoder().decode(Answer.self, from: answer))?.view
    }
}

extension WireSection: Identifiable {
    public var id: WireSectionKind { kind }
}

extension WireTicketCard: Identifiable {}
extension WireLandingCard: Identifiable {}
extension WireAgentRow: Identifiable {}
extension WireTileCard: Identifiable {}
extension WireThemeTile: Identifiable {}

extension WireWaitingGroup: Identifiable {
    /// The probe id the views and the UI tests use.
    public var id: String {
        switch kind {
        case .next: "plan-next-up"
        case .notRanked: "plan-not-ranked"
        case .onYou: "plan-waiting-on-you"
        }
    }
}

extension WireTicketDetails {
    public func ticket(_ task: String) -> WireTicketDetail? { tickets.first { $0.id == task } }
}

extension WireNote {
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

    /// Any request to the same bridge, for the shared phrases (`SharedWords`).
    static func ask(_ request: String) -> String? {
        state.lock.lock()
        let present = state.present ?? linked
        state.lock.unlock()
        return present?(request)
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
