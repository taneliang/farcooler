import Foundation

/// The iPhone's Plan sheet's status filters (ov-616): Needs You, In Progress,
/// In Review and Done, one or none chosen. With none, the tree shows what
/// `OneTreeFilter.open` does, every card not finished, with the finished ones
/// folded under each theme.
///
/// A narrowing on top of `OneTreeFilter` rather than more cases of it: the
/// Mac's navigator offers that filter's three as a segmented control, and its
/// choice is kept by their raw values.
public enum PlanFilter: String, CaseIterable, Sendable {
    case needsYou
    case inProgress
    case inReview
    case done

    public var title: String {
        switch self {
        case .needsYou: "Needs You"
        case .inProgress: "In Progress"
        case .inReview: "In Review"
        case .done: "Done"
        }
    }

    /// The tree's own filter under this one: finished cards are only
    /// reached under All.
    public var base: OneTreeFilter { self == .done ? .all : .open }

    /// Whether `task` is listed: one that asks for you, or one in the
    /// filter's status.
    public func keeps(_ task: OneTreeTask, asks: OneTreeAsks) -> Bool {
        switch self {
        case .needsYou: asks.tasks.contains(task.id) || task.status == .needsDecision
        case .inProgress: task.status == .inProgress
        case .inReview: task.status == .inReview
        case .done: task.status == .done
        }
    }

    /// What the tree says with nothing under the filter.
    public var emptyWords: String {
        switch self {
        case .needsYou: "Nothing needs you."
        case .inProgress: "Nothing is in progress."
        case .inReview: "Nothing is in review."
        case .done: "Nothing is done yet."
        }
    }

    /// Where a phone keeps a workspace's choice.
    public static func key(runner: String, workspace: String) -> String { "plan.filter.\(runner).\(workspace)" }

    public static func remembered(runner: String, workspace: String, in defaults: UserDefaults = .standard) -> PlanFilter? {
        defaults.string(forKey: key(runner: runner, workspace: workspace)).flatMap(PlanFilter.init(rawValue:))
    }

    public static func remember(_ filter: PlanFilter?, runner: String, workspace: String, in defaults: UserDefaults = .standard) {
        defaults.set(filter?.rawValue, forKey: key(runner: runner, workspace: workspace))
    }
}
