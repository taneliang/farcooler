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
// 3. a unique agent roster with runner-reported activity, then tickets by
//    their own stage. Assignment and stage age are ticket context; neither
//    diagnoses an agent's execution or attention;
// 4. the footer counts, which open the ticket lists.
//
// Needs You sits between the facts and Initiatives, but its rows are the
// runner's own list, which the plan doesn't carry, so the views add them.

/// The plan column as the Rust core sends it; the types are generated from
/// `column.rs` (`WireTypes.swift`, ov-1035) and this names them the way the apps do.

extension WireColumn {

    /// The column of `plan` as read from the runner, or nil when it has no
    /// view (a plan made in code, or no bridge).
    /// - Parameters:
    ///   - needsYou: the workspace's Needs You count, the theme asks included.
    ///   - statuses: the board's card statuses by task id.
    ///   - titles: the board's card titles by task id, for a card the plan
    ///     doesn't carry.
    public static func of(
        _ plan: PlanModel, needsYou: Int = 0, statuses: [String: TaskStatus] = [:], titles: [String: String] = [:]
    ) -> WireColumn? {
        WirePlanView.build(plan: plan, context: PlanViewContext(needsYou: needsYou, statuses: statuses, titles: titles))?.column
    }

    /// What a Needs You row puts in the orchestrator's input when clicked:
    /// a reference to it, for the owner to finish (unsent).
    public static func quote(key: String?, title: String) -> String {
        if let key, !key.isEmpty { return AskAboutTask.draft(key: key, title: title) }
        return "About \u{201C}\(AskAboutTask.oneLine(title))\u{201D}: "
    }
}

/// "not_started" as "notStarted": the name the apps' accessibility ids and navigation carry.
private func camelName(_ word: String) -> String {
    let parts = word.split(separator: "_")
    return (parts.first.map(String.init) ?? "") + parts.dropFirst().map { $0.prefix(1).uppercased() + $0.dropFirst() }.joined()
}

extension WireFactKind {
    /// The app's own name, which accessibility ids carry.
    public var name: String { camelName(rawValue) }
}

extension WireFact: Identifiable {
    public var id: String { kind.name }
}

extension WireCountKind {
    /// The app's own name, which a footer's list and its accessibility id carry.
    public var name: String { camelName(rawValue) }
    public init?(name: String) {
        guard let kind = [WireCountKind.waiting, .notStarted, .done].first(where: { $0.name == name }) else { return nil }
        self = kind
    }
}

extension WireCount: Identifiable {
    public var id: String { kind.name }
    /// The board's list it opens.
    public var taskStatus: TaskStatus { TaskStatus(rawValue: status) ?? .todo }
}

extension WireInitiative: Identifiable {}
extension WireColumnTicket: Identifiable {
    public var id: String { task }
}
extension WireColumnAgent: Identifiable {}
extension WireWaitingTicket: Identifiable {
    public var id: String { task }
}
extension WireGroup: Identifiable {
    public var id: String { stage.rawValue }
}

extension WirePipeStage: CaseIterable {
    public static var allCases: [WirePipeStage] { [.stuck, .building, .reviewing, .landing] }

    /// A stage's list's title, for a list opened by name when its stage
    /// has emptied; a group in the column says its own (`Group.title`).
    public var title: String {
        switch self {
        case .stuck: "Needs You"
        case .building: "Building"
        case .reviewing: "Reviewing"
        case .landing: "Landing"
        }
    }
}

extension WireRole {
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

// A board with only listed workers still has a roster to show. Keep the
// broader `PlanModel.isEmpty` rule (which chooses board layout) unchanged.
extension PlanModel {
    public var hasColumnAgents: Bool { agents.contains { $0.role != .orchestrator } }
}
