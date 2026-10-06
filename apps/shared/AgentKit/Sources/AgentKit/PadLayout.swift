import Foundation

// The iPad's workspace (ov-348; .claude/agent/reports/phones-tree/design.md
// §4): the Mac's layout in columns, the One tree as an outline, the plan as
// the canvas's home and the orchestrator's chat beside it, always shown.
// Compact width (a Split View, Slide Over, or a phone) keeps the phone's
// segments, strip and sheet.
//
// The rules are here, so `swift test` reads them; `PadWorkspace.swift` draws.

enum PadLayout: Equatable, Sendable {
    /// The phone's screen: Orchestrator, Themes and Board, one at a time.
    case phone
    /// The plan and the chat side by side, and the tree a sidebar shown on
    /// demand over the plan: an 11-inch iPad in portrait, or two thirds of a
    /// Split View, where three columns would leave the chat too narrow to
    /// read a terminal in.
    case twoColumns
    /// The tree, the plan and the chat.
    case threeColumns

    /// The narrowest width that holds three columns: the tree's least, the
    /// plan's least, and a chat of about 60 terminal columns. A 13-inch iPad
    /// in portrait (1,032 points) clears it; an 11-inch one (834) doesn't.
    static let threeColumnWidth: Double = 1000
    /// The narrowest width that holds two: below it a regular-width window
    /// is drawn as a phone.
    static let twoColumnWidth: Double = 700

    /// The layout for a window `width` points wide. Only an iPad gets
    /// columns, and only at regular width: a large iPhone in landscape is
    /// regular too, and stays a phone. A runner without workspaces has no
    /// orchestrator and no plan, so its board keeps the phone's screen.
    static func of(isPad: Bool, regularWidth: Bool, width: Double, implicit: Bool) -> PadLayout {
        guard isPad, regularWidth, !implicit else { return .phone }
        if width >= threeColumnWidth { return .threeColumns }
        if width >= twoColumnWidth { return .twoColumns }
        return .phone
    }

    /// Whether the layout draws columns rather than the phone's segments.
    var hasColumns: Bool { self != .phone }

    /// Each column's width in a window `width` points wide. The tree is
    /// zero when it isn't a column (`twoColumns` shows it on demand, at
    /// `sidebar`); the chat takes what's left.
    func widths(_ width: Double) -> (tree: Double, plan: Double, chat: Double) {
        switch self {
        case .phone:
            return (0, 0, width)
        case .twoColumns:
            let plan = Self.clamp(width * 0.45, 340, 480)
            return (0, plan, width - plan)
        case .threeColumns:
            let tree = Self.treeColumn(width)
            let plan = Self.clamp(width * 0.35, 340, 520)
            return (tree, plan, width - tree - plan)
        }
    }

    /// The tree's width as a column: a card's key and a few words of its
    /// title on one line, four levels in.
    static func treeColumn(_ width: Double) -> Double { clamp(width * 0.24, 260, 340) }

    /// The tree's width shown on demand: over the plan, it takes no room
    /// from the columns, so it's as wide as a column's best.
    static func sidebar(_ width: Double) -> Double { clamp(width * 0.4, 300, 360) }

    private static func clamp(_ value: Double, _ low: Double, _ high: Double) -> Double {
        min(max(value, low), high)
    }
}

/// What the iPad's plan column shows: the canvas's home, or what was picked
/// in the tree or the toolbar.
enum PadCanvas: Hashable, Sendable {
    /// The plan's sections, as the phone's Plan sheet draws them.
    case plan
    /// The board, the toolbar's item (design §4).
    case board
    /// This workspace's Needs You, answerable here, as the Mac's canvas
    /// shows it (ruling R-25).
    case needsYou
    case page(PhonePlanPage)
    case task(String)
}

/// What a pick in the iPad's tree does.
enum PadPick: Equatable, Sendable {
    /// Show it in the plan column.
    case canvas(PadCanvas)
    /// Open it as the phone does: a worktree, or a terminal in one, covers
    /// the stack, the shell being a screen of its own.
    case open(PhoneRoute)
    /// Nothing to show: a subagent, which runs inside the orchestrator, and
    /// the chat column is already that.
    case none
}

extension PadPick {
    /// Where `target` goes on an iPad, in `place`'s workspace: a page, a
    /// task or the workspace's Needs You in the plan column, beside the tree
    /// it was picked from; a worktree or terminal over everything, as on the
    /// phone.
    static func of(_ target: OneTreeTarget, in place: PhoneWorkspace) -> PadPick {
        switch target {
        case .plan: .canvas(.plan)
        case .theme(let id): .canvas(.page(.theme(id)))
        case .lane(let id): .canvas(.page(.lane(id)))
        case .page(let slot): .canvas(.page(.page(slot)))
        case .task(let id): .canvas(.task(id))
        case .needsYou: .canvas(.needsYou)
        case .orchestrator: .none
        case .worktree, .terminal: PhoneTree.route(target, in: place).map(PadPick.open) ?? .none
        }
    }

    /// The tree's target that `canvas` stands for, to mark its row chosen.
    static func target(of canvas: PadCanvas) -> OneTreeTarget? {
        switch canvas {
        case .plan: .plan
        case .board: nil
        case .needsYou: .needsYou
        case .page(.theme(let id)): .theme(id)
        case .page(.lane(let id)): .lane(id)
        case .page(.page(let slot)): .page(slot)
        case .task(let id): .task(id)
        }
    }
}

/// The iPad tree's open and closed rows, kept per workspace on this device,
/// as the Mac keeps a window's.
enum PadTreeExpansion {
    static func key(_ place: PhoneWorkspace) -> String { "pad.tree.expansion.\(place.runner).\(place.workspace)" }

    static func remembered(_ place: PhoneWorkspace, in defaults: UserDefaults = .standard) -> OneTreeExpansion {
        OneTreeExpansion(encoded: defaults.string(forKey: key(place)) ?? "")
    }

    static func remember(_ expansion: OneTreeExpansion, for place: PhoneWorkspace, in defaults: UserDefaults = .standard) {
        defaults.set(expansion.encoded, forKey: key(place))
    }
}
