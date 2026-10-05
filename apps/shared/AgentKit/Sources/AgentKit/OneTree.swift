import Foundation

// One tree (ov-321, Concept 1 of .claude/agent/reports/ov-321/ia.md, 3.1 and
// 4): the workspace's sidebar as one outline that follows containment,
//
//   Theme › Task › Lane › Terminals
//
// where a lane IS its worktree. The board has two hierarchies that meet at the
// task: the why (theme › task) and the how (lane › worktree › terminals). The
// old sidebar listed kinds (Themes, Pages, Tasks, Terminals, Worktrees) and
// drew none of the joins; this draws them.
//
// Every rule is here, as values, so the Mac's sidebar and (later) the phones'
// push navigation read one model: what's under what, which lane shows under
// two cards ("also ov-303"), where a Needs You dot rolls up to, which theme
// opens by default, and the path the jump bar shows. The views only lay out
// `OneTree.rows`.

// MARK: - What the tree is built from

/// A card, as the tree needs it.
public struct OneTreeTask: Equatable, Sendable {
    public var id: String
    public var key: String
    public var title: String
    public var status: TaskStatus
    /// When anything last moved on it, in ms: what picks the theme with the
    /// newest activity.
    public var activityMs: Int64
    /// Its own worktree (`tasks.worktree_id`), when it has one.
    public var worktreeID: String?
    /// The subagents recorded on it (`task_workers`).
    public var workers: [TaskWorker]

    public init(
        id: String, key: String, title: String, status: TaskStatus, activityMs: Int64 = 0, worktreeID: String? = nil,
        workers: [TaskWorker] = []
    ) {
        self.id = id
        self.key = key
        self.title = title
        self.status = status
        self.activityMs = activityMs
        self.worktreeID = worktreeID
        self.workers = workers
    }

    /// A card as the board read it.
    public init(row: TaskRow) {
        self.init(
            id: row.id, key: row.key, title: row.title, status: row.status,
            activityMs: Int64(row.lastMoved.timeIntervalSince1970 * 1000), worktreeID: row.worktreeID,
            workers: row.workers)
    }

    /// A card only the plan read (a theme names it, the board doesn't list
    /// it): its status from the CLI's word for it. Nil for a word this build
    /// doesn't know, which is never guessed.
    public init?(card: PlanCard) {
        guard let status = OneTreeTask.status(word: card.status) else { return nil }
        self.init(id: card.task, key: card.key, title: card.title, status: status)
    }

    /// "In Review" (the CLI's word in `plan --json`), or the wire's
    /// `in_review`, as a status.
    static func status(word: String) -> TaskStatus? {
        if let status = TaskStatus(rawValue: word) { return status }
        let folded = word.lowercased().replacingOccurrences(of: " ", with: "")
        return TaskStatus.allCases.first {
            $0.title.lowercased().replacingOccurrences(of: " ", with: "") == folded
                || $0.rawValue.replacingOccurrences(of: "_", with: "") == folded
        } ?? (folded == "cancelled" ? .cancelled : nil)
    }
}

/// A pane in a worktree.
public struct OneTreeTerminal: Equatable, Sendable {
    public var id: String
    public var title: String
    /// An agent's pane; else a shell's.
    public var isAgent: Bool
    /// The orchestrator's own seat, which only its conversation shows.
    public var isOrchestrator: Bool

    public init(id: String, title: String, isAgent: Bool, isOrchestrator: Bool = false) {
        self.id = id
        self.title = title
        self.isAgent = isAgent
        self.isOrchestrator = isOrchestrator
    }
}

/// A worktree: a lane's, a task's own, the main checkout or a loose one.
public struct OneTreeWorktree: Equatable, Sendable {
    public var id: String
    /// Its name as the fleet says it (`.claude/worktrees/<name>`).
    public var name: String
    /// Its directory on its runner.
    public var path: String
    public var branch: String
    public var isMainCheckout: Bool
    public var isHidden: Bool
    public var terminals: [OneTreeTerminal]
    /// The tasks whose panes it holds (`terminals.task_id`) or that it is
    /// the lane of (`open_tasks`), by id.
    public var taskIDs: [String]

    public init(
        id: String, name: String, path: String = "", branch: String = "", isMainCheckout: Bool = false,
        isHidden: Bool = false, terminals: [OneTreeTerminal] = [], taskIDs: [String] = []
    ) {
        self.id = id
        self.name = name
        self.path = path
        self.branch = branch
        self.isMainCheckout = isMainCheckout
        self.isHidden = isHidden
        self.terminals = terminals
        self.taskIDs = taskIDs
    }
}

/// An orchestrator's page, and the theme it's anchored to.
public struct OneTreePage: Equatable, Sendable {
    public var slot: String
    public var title: String
    /// The theme's id, or nil for a page anchored to none.
    public var themeID: String?

    public init(slot: String, title: String, themeID: String? = nil) {
        self.slot = slot
        self.title = title
        self.themeID = themeID
    }

    public init(_ page: BoardPage) {
        self.init(slot: page.slot, title: page.title, themeID: page.anchorKind == "theme" && !page.anchor.isEmpty ? page.anchor : nil)
    }
}

/// What asks for the owner, by what it's on: the items of the workspace's
/// Needs You, and the themes with an `owner_ask`.
public struct OneTreeAsks: Equatable, Sendable {
    public var tasks: Set<String> = []
    public var themes: Set<String> = []
    public var terminals: Set<String> = []
    /// An item on the orchestrator's own pane.
    public var orchestrator = false

    public init(tasks: Set<String> = [], themes: Set<String> = [], terminals: Set<String> = [], orchestrator: Bool = false) {
        self.tasks = tasks
        self.themes = themes
        self.terminals = terminals
        self.orchestrator = orchestrator
    }

    /// The asks in `items` (the workspace's own Needs You) and `plan`'s
    /// themes: each item on the task it names, else on the pane it names.
    public init(items: [NeedsYouItem], plan: PlanModel) {
        for item in items {
            if let task = item.task {
                tasks.insert(task.id)
            } else if let terminal = item.terminal {
                if terminal.isOrchestrator { orchestrator = true } else { terminals.insert(terminal.id) }
            }
        }
        themes = Set(plan.themes.filter { !$0.ownerAsk.isEmpty }.map(\.id))
    }
}

/// Which cards the tree shows: status is a filter on the tree, not a
/// sidebar axis of its own.
public enum OneTreeFilter: String, CaseIterable, Sendable {
    case open
    case inReview
    case all

    public var title: String {
        switch self {
        case .open: "Open"
        case .inReview: "In Review"
        case .all: "All"
        }
    }

    /// Whether a card in `status` is listed.
    public func shows(_ status: TaskStatus) -> Bool {
        switch self {
        case .open: !status.isFinished
        case .inReview: status == .inReview
        case .all: true
        }
    }
}

/// Everything the tree is built from.
public struct OneTreeInput: Equatable, Sendable {
    /// The board's cards, in the board's order.
    public var tasks: [OneTreeTask]
    public var plan: PlanModel
    /// The workspace's worktrees: lanes', tasks' and loose ones. Not the
    /// main checkout, which is `mainCheckout`.
    public var worktrees: [OneTreeWorktree]
    /// The repository's own checkout, holding the project terminals.
    public var mainCheckout: OneTreeWorktree?
    public var pages: [OneTreePage]
    public var asks: OneTreeAsks
    public var filter: OneTreeFilter
    /// The pinned places' trailing words: the orchestrator's state, and
    /// the Needs You count (the title bar's number, the same).
    public var orchestratorWord: String?
    public var needsYouCount: Int
    /// Whether the workspace has an orchestrator to focus.
    public var hasOrchestrator: Bool

    public init(
        tasks: [OneTreeTask], plan: PlanModel = .empty, worktrees: [OneTreeWorktree] = [],
        mainCheckout: OneTreeWorktree? = nil, pages: [OneTreePage] = [], asks: OneTreeAsks = OneTreeAsks(),
        filter: OneTreeFilter = .open, orchestratorWord: String? = nil, needsYouCount: Int = 0,
        hasOrchestrator: Bool = true
    ) {
        self.tasks = tasks
        self.plan = plan
        self.worktrees = worktrees
        self.mainCheckout = mainCheckout
        self.pages = pages
        self.asks = asks
        self.filter = filter
        self.orchestratorWord = orchestratorWord
        self.needsYouCount = needsYouCount
        self.hasOrchestrator = hasOrchestrator
    }
}

// MARK: - The tree

/// Where a node goes when it's chosen.
public enum OneTreeTarget: Hashable, Sendable {
    /// The orchestrator's chat column, focused.
    case orchestrator
    /// The workspace's Needs You, answerable, in the canvas.
    case needsYou
    /// The canvas's home.
    case plan
    case theme(String)
    case task(String)
    case lane(String)
    /// A worktree opened whole: a task's own with no lane, or a loose one.
    case worktree(String)
    case terminal(worktree: String, terminal: String)
    case page(String)
}

/// One node of the outline.
public struct OneTreeNode: Equatable, Sendable, Identifiable {
    public enum Kind: Equatable, Sendable {
        case place
        case theme
        case task
        /// "4 done": a theme's finished cards, folded.
        case doneFold
        /// A lane: a plan lane (with its worktree, if any), or a task's own
        /// worktree with no lane recorded.
        case lane
        case terminal
        /// An agent with no pane: it runs inside the orchestrator.
        case subagent
        case page
        /// No Theme, Main Checkout, Loose Worktrees.
        case group
        case worktree
    }

    /// Unique in the tree: its path from the root. A lane under two cards
    /// is two nodes, with one target.
    public var id: String
    public var kind: Kind
    public var title: String
    /// A task's key, before its title.
    public var key: String = ""
    /// Trailing, quiet: a theme's progress, a lane's state, a count.
    public var detail: String = ""
    /// A second, quieter line: a subagent's "No terminal; …".
    public var caption: String = ""
    /// "also ov-303": the lane works other cards too.
    public var also: String = ""
    /// An SF Symbol name for the row's glyph.
    public var glyph: String = ""
    public var target: OneTreeTarget?
    public var children: [OneTreeNode] = []
    /// This node itself asks for the owner.
    public var asks = false
    /// It or anything under it asks: what a collapsed node's dot rolls up.
    public var holdsAsk = false
    /// Open until someone closes it.
    public var expandedByDefault = false
    /// Drawn quieter: a finished card, a landed lane.
    public var quiet = false
    /// For a lane: its state, so the view can tint a stale or waiting one.
    public var laneState: LaneState?

    public var hasChildren: Bool { !children.isEmpty }

    /// Whether choosing `target` lands on this node: its own target, or,
    /// for a lane, its worktree opened whole.
    public func stands(for target: OneTreeTarget?) -> Bool {
        guard let target else { return false }
        if self.target == target { return true }
        guard kind == .lane, case .worktree(let id) = target else { return false }
        return children.contains {
            if case .terminal(id, _)? = $0.target { return true }
            return false
        }
    }

    /// Whether the row draws the amber dot: it asks, or it's closed over
    /// something that does (the roll-up, as Xcode's issue badges do).
    public func showsDot(expanded: Bool) -> Bool { asks || (!expanded && holdsAsk) }
}

/// The sidebar: three pinned places, the tree, and under a divider the main
/// checkout and the loose worktrees.
public struct OneTree: Equatable, Sendable {
    public var places: [OneTreeNode]
    public var tree: [OneTreeNode]
    public var below: [OneTreeNode]

    /// Every root, top to bottom.
    public var roots: [OneTreeNode] { places + tree + below }

    public static func build(_ input: OneTreeInput) -> OneTree {
        OneTreeBuilder(input).build()
    }
}

// MARK: - Words

public enum OneTreeWords {
    public static let orchestrator = "Orchestrator"
    public static let needsYou = "Needs You"
    public static let plan = "Plan"
    public static let noTheme = "No Theme"
    public static let mainCheckout = "Main Checkout"
    public static let looseWorktrees = "Loose Worktrees"
    public static let subagentCaption = "No terminal; runs inside the orchestrator"

    /// "4 done".
    public static func done(_ n: Int) -> String { "\(n) done" }

    /// "3/10": a theme's cards done of those counted.
    public static func progress(_ counts: PlanCounts) -> String { "\(counts.done)/\(PlanWords.total(counts))" }

    /// "also ov-303", "also ov-303 +2": the other cards a lane works.
    public static func also(_ keys: [String]) -> String {
        guard let first = keys.first else { return "" }
        return keys.count == 1 ? "also \(first)" : "also \(first) +\(keys.count - 1)"
    }

    /// "1 shell", "2 shells", "No shells".
    public static func shells(_ n: Int) -> String {
        switch n {
        case 0: "No shells"
        case 1: "1 shell"
        default: "\(n) shells"
        }
    }

    /// A subagent's name: "Builder · Opus", "Subagent · Sonnet".
    public static func subagent(role: String, model: String) -> String {
        let model = PlanWords.model(model)
        return model.isEmpty ? role : "\(role) · \(model)"
    }

    /// A lane agent's role, as people say it.
    public static func role(_ raw: String) -> String {
        switch raw {
        case "review": "Reviewer"
        case "fix": "Fixer"
        default: "Builder"
        }
    }

    /// A lane state's glyph.
    public static func glyph(_ state: LaneState) -> String {
        switch state {
        case .queued: "clock"
        case .building: "hammer"
        case .review: "eye"
        case .fixing: "wrench.adjustable"
        case .landing: "arrow.down.to.line"
        case .landed: "checkmark.circle"
        case .dropped: "xmark.circle"
        case .unknown: "questionmark.circle"
        }
    }
}

// MARK: - Building

/// Builds a `OneTree` from its input. Each rule is a method, named for it.
struct OneTreeBuilder {
    let input: OneTreeInput
    let tasksByID: [String: OneTreeTask]
    let themes: [PlanTheme]
    let lanes: [PlanLane]
    /// Each lane's worktree, by lane id.
    let laneWorktrees: [String: OneTreeWorktree]
    /// The theme expanded by default.
    let newestTheme: String?

    init(_ input: OneTreeInput) {
        self.input = input
        var byID: [String: OneTreeTask] = [:]
        for task in input.tasks where byID[task.id] == nil { byID[task.id] = task }
        for card in input.plan.cards where byID[card.task] == nil {
            if let task = OneTreeTask(card: card) { byID[card.task] = task }
        }
        tasksByID = byID
        themes = input.plan.shownThemes
        lanes = input.plan.lanes.filter { $0.state != .dropped }
        var joined: [String: OneTreeWorktree] = [:]
        for lane in lanes {
            if let worktree = OneTreeBuilder.worktree(of: lane, in: input.worktrees) { joined[lane.id] = worktree }
        }
        laneWorktrees = joined
        newestTheme = OneTreeBuilder.newestTheme(
            themes.filter { $0.state == "active" }.isEmpty ? themes : themes.filter { $0.state == "active" },
            tasks: byID, lanes: lanes)
    }

    func build() -> OneTree {
        let tree = themeNodes() + [noThemeNode()].compactMap { $0 }
        return OneTree(places: places(), tree: tree, below: [mainCheckoutNode(), looseNode()].compactMap { $0 })
    }

    // MARK: Joins

    /// The worktree a lane works in: the one at its path, else (a path
    /// recorded relative to the repository) the one whose path ends with
    /// it, else the one on its branch. Nil for a lane with no worktree yet.
    static func worktree(of lane: PlanLane, in worktrees: [OneTreeWorktree]) -> OneTreeWorktree? {
        let path = lane.worktreePath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if !path.isEmpty {
            if let exact = worktrees.first(where: { $0.path == lane.worktreePath }) { return exact }
            if let suffix = worktrees.first(where: { $0.path.hasSuffix("/" + path) }) { return suffix }
        }
        if !lane.branch.isEmpty, let branch = worktrees.first(where: { $0.branch == lane.branch }) { return branch }
        return nil
    }

    /// The theme with the newest activity: its story, its cards' moves and
    /// its lanes' state changes. The earlier on the board when two tie.
    static func newestTheme(_ themes: [PlanTheme], tasks: [String: OneTreeTask], lanes: [PlanLane]) -> String? {
        var best: (id: String, at: Int64)?
        for theme in themes {
            let ids = Set(theme.cards.map(\.task))
            var at = theme.storyAt
            for id in ids { at = max(at, tasks[id]?.activityMs ?? 0) }
            for lane in lanes where lane.cards.contains(where: { ids.contains($0.task) }) {
                at = max(at, lane.stateSince)
            }
            if best == nil || at > best!.at { best = (theme.id, at) }
        }
        return best?.id
    }

    /// The lanes working `task`, in the plan's order: a fix round's lane
    /// after the lane it fixes.
    func lanes(of task: String) -> [PlanLane] {
        lanes.filter { lane in lane.cards.contains { $0.task == task } }
    }

    /// Every worktree a lane reaches.
    var laneWorktreeIDs: Set<String> { Set(laneWorktrees.values.map(\.id)) }

    /// The worktrees `task` works in with no lane recorded: its own, and
    /// any holding its panes, that no lane already stands for.
    func ownWorktrees(of task: OneTreeTask) -> [OneTreeWorktree] {
        let taken = Set(lanes(of: task.id).compactMap { laneWorktrees[$0.id]?.id })
        var out: [OneTreeWorktree] = []
        if let own = task.worktreeID, let found = input.worktrees.first(where: { $0.id == own }), !taken.contains(own) {
            out.append(found)
        }
        for worktree in input.worktrees where worktree.taskIDs.contains(task.id) {
            if !taken.contains(worktree.id), !out.contains(where: { $0.id == worktree.id }) { out.append(worktree) }
        }
        return out
    }

    // MARK: Places

    func places() -> [OneTreeNode] {
        var out: [OneTreeNode] = []
        if input.hasOrchestrator {
            var node = OneTreeNode(
                id: "place:orchestrator", kind: .place, title: OneTreeWords.orchestrator,
                detail: input.orchestratorWord ?? "", glyph: "bubble.left.and.text.bubble.right", target: .orchestrator)
            node.asks = input.asks.orchestrator
            node.holdsAsk = node.asks
            out.append(node)
        }
        out.append(
            OneTreeNode(
                id: "place:needs-you", kind: .place, title: OneTreeWords.needsYou,
                detail: input.needsYouCount > 0 ? "\(input.needsYouCount)" : "", glyph: "flag", target: .needsYou))
        let shownThemes = Set(themes.map(\.id))
        let unanchored = input.pages.filter { page in page.themeID.map { !shownThemes.contains($0) } ?? true }
        out.append(
            finish(
                OneTreeNode(
                    id: "place:plan", kind: .place, title: OneTreeWords.plan, glyph: "map", target: .plan,
                    children: unanchored.map { pageNode($0, under: "place:plan") })))
        return out
    }

    func pageNode(_ page: OneTreePage, under parent: String) -> OneTreeNode {
        OneTreeNode(id: "\(parent)/page:\(page.slot)", kind: .page, title: page.title, glyph: "doc.text", target: .page(page.slot))
    }

    // MARK: Themes

    /// The themes the filter leaves: active and paused ones always under
    /// Open, a finished one only while it still has open cards; under In
    /// Review, those holding a card in review; every one under All.
    func themeNodes() -> [OneTreeNode] {
        themes.compactMap { theme -> OneTreeNode? in
            let ids = theme.cards.map(\.task)
            let shown = ids.compactMap { tasksByID[$0] }.filter { input.filter.shows($0.status) }
            switch input.filter {
            case .open where theme.state == "done" && shown.isEmpty: return nil
            case .inReview where shown.isEmpty: return nil
            default: break
            }
            let id = "theme:\(theme.id)"
            var children = shown.map { taskNode($0, under: id, theme: theme.id) }
            if input.filter == .open {
                let done = ids.compactMap { tasksByID[$0] }.filter { $0.status == .done }
                if !done.isEmpty {
                    let fold = "\(id)/done"
                    children.append(
                        finish(
                            OneTreeNode(
                                id: fold, kind: .doneFold, title: OneTreeWords.done(done.count), glyph: "checkmark",
                                children: done.map { taskNode($0, under: fold, theme: theme.id) }, quiet: true)))
                }
            }
            children += input.pages.filter { $0.themeID == theme.id }.map { pageNode($0, under: id) }
            var node = OneTreeNode(
                id: id, kind: .theme, title: theme.name, detail: OneTreeWords.progress(theme.counts),
                glyph: theme.state == "paused" ? "pause.circle" : theme.state == "done" ? "checkmark.circle" : "map",
                target: .theme(theme.id), children: children)
            node.asks = input.asks.themes.contains(theme.id)
            node.expandedByDefault = theme.id == newestTheme
            return finish(node)
        }
    }

    /// No Theme: the open cards in no theme the tree shows. On a board
    /// with no plan it's every card, open from the start.
    func noThemeNode() -> OneTreeNode? {
        let themed = Set(themes.flatMap { $0.cards.map(\.task) })
        let order = Dictionary(TaskBoardModel.order.enumerated().map { ($1, $0) }, uniquingKeysWith: { a, _ in a })
        let mine = input.tasks.filter { !themed.contains($0.id) && input.filter.shows($0.status) }
            .enumerated().sorted { l, r in
                let (a, b) = (order[l.element.status] ?? 99, order[r.element.status] ?? 99)
                if a != b { return a < b }
                if l.element.activityMs != r.element.activityMs { return l.element.activityMs > r.element.activityMs }
                return l.offset < r.offset
            }.map(\.element)
        guard !mine.isEmpty else { return nil }
        let id = "group:no-theme"
        var node = OneTreeNode(
            id: id, kind: .group, title: OneTreeWords.noTheme, detail: "\(mine.count)", glyph: "tray",
            children: mine.map { taskNode($0, under: id, theme: nil) })
        node.expandedByDefault = themes.isEmpty
        return finish(node)
    }

    // MARK: Tasks and lanes

    func taskNode(_ task: OneTreeTask, under parent: String, theme: String?) -> OneTreeNode {
        let id = "\(parent)/task:\(task.id)"
        let mine = lanes(of: task.id)
        var children = mine.map { laneNode($0, for: task, under: id) }
        children += ownWorktrees(of: task).map { worktreeLaneNode($0, for: task, under: id) }
        // Subagents with no lane to sit under sit under the card.
        if mine.isEmpty {
            children += subagents(lane: nil, task: task, under: id)
        }
        var node = OneTreeNode(
            id: id, kind: .task, title: task.title, key: task.key, glyph: TaskStatusGlyph.name(task.status),
            target: .task(task.id), children: children, quiet: task.status.isFinished)
        node.asks = input.asks.tasks.contains(task.id)
        // A card being worked opens on its lanes inside the theme that's
        // open by default; the rest stay closed.
        node.expandedByDefault = theme != nil && theme == newestTheme
            && mine.contains { $0.state.isLive && $0.state != .queued }
        return finish(node)
    }

    func laneNode(_ lane: PlanLane, for task: OneTreeTask, under parent: String) -> OneTreeNode {
        let id = "\(parent)/lane:\(lane.id)"
        let others = lane.cards.filter { $0.task != task.id }.map(\.key)
        var children: [OneTreeNode] = []
        if let worktree = laneWorktrees[lane.id] { children += terminalNodes(worktree, under: id) }
        children += subagents(lane: lane, task: task, under: id)
        var node = OneTreeNode(
            id: id, kind: .lane, title: lane.name, detail: PlanWords.state(lane.state),
            also: OneTreeWords.also(others), glyph: OneTreeWords.glyph(lane.state), target: .lane(lane.id),
            children: children, quiet: !lane.state.isLive)
        node.laneState = lane.state
        return finish(node)
    }

    /// A task's own worktree with no lane recorded: a lane all the same,
    /// with no state to say.
    func worktreeLaneNode(_ worktree: OneTreeWorktree, for task: OneTreeTask, under parent: String) -> OneTreeNode {
        let id = "\(parent)/worktree:\(worktree.id)"
        return finish(
            OneTreeNode(
                id: id, kind: .lane, title: worktree.name, glyph: "arrow.triangle.branch",
                target: .worktree(worktree.id), children: terminalNodes(worktree, under: id)))
    }

    func terminalNodes(_ worktree: OneTreeWorktree, under parent: String) -> [OneTreeNode] {
        worktree.terminals.filter { !$0.isOrchestrator }.map { terminal in
            var node = OneTreeNode(
                id: "\(parent)/terminal:\(terminal.id)", kind: .terminal, title: terminal.title,
                detail: terminal.isAgent ? "Agent" : "Shell", glyph: terminal.isAgent ? "sparkles" : "terminal",
                target: .terminal(worktree: worktree.id, terminal: terminal.id))
            node.asks = input.asks.terminals.contains(terminal.id)
            node.holdsAsk = node.asks
            return node
        }
    }

    /// The agents with no pane of their own: a lane's open agents
    /// (`lane_agents`), or, where the lane records none, the card's open
    /// subagents (`task_workers`). The same agent is often recorded in
    /// both; the lane's record wins, so it shows once.
    func subagents(lane: PlanLane?, task: OneTreeTask, under parent: String) -> [OneTreeNode] {
        if let lane {
            guard lane.state.isLive else { return [] }
            let open = lane.agents.filter { $0.endedAt == nil }
            if !open.isEmpty {
                return open.enumerated().map { index, agent in
                    subagentNode(
                        id: "\(parent)/agent:\(agent.agentId.isEmpty ? "\(index)" : agent.agentId)",
                        title: OneTreeWords.subagent(role: OneTreeWords.role(agent.role), model: agent.model))
                }
            }
        }
        return task.workers.filter(\.state.isOpen).enumerated().map { index, worker in
            subagentNode(
                id: "\(parent)/worker:\(index)",
                title: OneTreeWords.subagent(role: "Subagent", model: worker.model.isEmpty ? worker.harness : worker.model))
        }
    }

    func subagentNode(id: String, title: String) -> OneTreeNode {
        OneTreeNode(
            id: id, kind: .subagent, title: title, caption: OneTreeWords.subagentCaption, glyph: "person.crop.circle.dashed",
            target: .orchestrator)
    }

    // MARK: Below the divider

    func mainCheckoutNode() -> OneTreeNode? {
        guard let checkout = input.mainCheckout else { return nil }
        let id = "group:main"
        let terminals = terminalNodes(checkout, under: id)
        return finish(
            OneTreeNode(
                id: id, kind: .group, title: OneTreeWords.mainCheckout, detail: OneTreeWords.shells(terminals.count),
                glyph: "house", target: .worktree(checkout.id), children: terminals))
    }

    /// Loose Worktrees: the workspace's worktrees no lane and no card
    /// reaches, whatever the filter shows. Leftovers, for cleanup: closed.
    func looseNode() -> OneTreeNode? {
        let reached = laneWorktreeIDs.union(input.tasks.compactMap(\.worktreeID))
        let loose = input.worktrees.filter { worktree in
            !worktree.isMainCheckout && !worktree.isHidden && worktree.id != input.mainCheckout?.id
                && !reached.contains(worktree.id)
                && !worktree.taskIDs.contains { tasksByID[$0] != nil }
        }
        guard !loose.isEmpty else { return nil }
        let id = "group:loose"
        return finish(
            OneTreeNode(
                id: id, kind: .group, title: OneTreeWords.looseWorktrees, detail: "\(loose.count)",
                glyph: "archivebox",
                children: loose.map { worktree in
                    let node = "\(id)/worktree:\(worktree.id)"
                    return finish(
                        OneTreeNode(
                            id: node, kind: .worktree, title: worktree.name, glyph: "arrow.triangle.branch",
                            target: .worktree(worktree.id), children: terminalNodes(worktree, under: node)))
                }))
    }

    /// Roll a node's asks up from its children.
    func finish(_ node: OneTreeNode) -> OneTreeNode {
        var node = node
        node.holdsAsk = node.asks || node.children.contains(where: \.holdsAsk)
        return node
    }
}

/// A card's status as a glyph, shape before color.
public enum TaskStatusGlyph {
    public static func name(_ status: TaskStatus) -> String {
        switch status {
        case .backlog: "circle.dotted"
        case .todo: "circle"
        case .needsDecision: "questionmark.circle"
        case .inProgress: "circle.lefthalf.filled"
        case .inReview: "eye.circle"
        case .done: "checkmark.circle"
        case .cancelled: "xmark.circle"
        }
    }
}

// MARK: - Expansion, rows and paths

/// Which nodes are open: the person's choices, by node id, over each
/// node's default. Kept per window.
public struct OneTreeExpansion: Equatable, Sendable, Codable {
    public var choices: [String: Bool]

    public init(choices: [String: Bool] = [:]) { self.choices = choices }

    public func isExpanded(_ node: OneTreeNode) -> Bool { choices[node.id] ?? node.expandedByDefault }

    public mutating func toggle(_ node: OneTreeNode) { choices[node.id] = !isExpanded(node) }

    /// Open every node in `ids` (a path's ancestors): what revealing a
    /// selection does.
    public mutating func open(_ ids: [String]) {
        for id in ids { choices[id] = true }
    }

    /// Kept as text, for a window's own storage.
    public var encoded: String {
        (try? String(decoding: JSONEncoder().encode(choices), as: UTF8.self)) ?? ""
    }

    public init(encoded: String) {
        choices = (try? JSONDecoder().decode([String: Bool].self, from: Data(encoded.utf8))) ?? [:]
    }
}

/// One row of the outline as drawn.
public struct OneTreeRow: Equatable, Sendable, Identifiable {
    public var node: OneTreeNode
    public var depth: Int
    public var expanded: Bool
    /// The amber dot (`OneTreeNode.showsDot`).
    public var dot: Bool

    public var id: String { node.id }
}

extension OneTree {
    /// `nodes` as rows, top to bottom: each node, then, while it's open,
    /// its children one level in.
    public static func rows(_ nodes: [OneTreeNode], expansion: OneTreeExpansion, depth: Int = 0) -> [OneTreeRow] {
        var out: [OneTreeRow] = []
        for node in nodes {
            let open = node.hasChildren && expansion.isExpanded(node)
            out.append(OneTreeRow(node: node, depth: depth, expanded: open, dot: node.showsDot(expanded: open)))
            if open { out += rows(node.children, expansion: expansion, depth: depth + 1) }
        }
        return out
    }

    /// The nodes from a root down to the node for `target`: the one named
    /// `hint` when it is one (a lane chosen under its second card), else
    /// the first in the tree. Nil when nothing goes there.
    public func path(to target: OneTreeTarget, hint: String? = nil) -> [OneTreeNode]? {
        var first: [OneTreeNode]?
        var hinted: [OneTreeNode]?
        func walk(_ nodes: [OneTreeNode], _ trail: [OneTreeNode]) {
            for node in nodes {
                let here = trail + [node]
                if node.target == target {
                    if first == nil { first = here }
                    if node.id == hint { hinted = here }
                }
                walk(node.children, here)
            }
        }
        walk(roots, [])
        if let found = hinted ?? first { return found }
        // A lane's worktree opened whole is its lane: the node whose panes
        // are that worktree's.
        guard case .worktree(let id) = target else { return nil }
        var lane: [OneTreeNode]?
        func find(_ nodes: [OneTreeNode], _ trail: [OneTreeNode]) {
            for node in nodes where lane == nil {
                let here = trail + [node]
                let holds = node.kind == .lane && node.children.contains {
                    if case .terminal(id, _)? = $0.target { return true }
                    return false
                }
                if holds { lane = here } else { find(node.children, here) }
            }
        }
        find(roots, [])
        return lane
    }

    /// The breadcrumb for `target`: the targets on its path, each named,
    /// the pinned places left out (the jump bar's own first crumb is the
    /// workspace).
    public func crumbs(to target: OneTreeTarget, hint: String? = nil) -> [(title: String, target: OneTreeTarget?)] {
        guard let path = path(to: target, hint: hint) else { return [] }
        return path.filter { $0.kind != .place && $0.kind != .doneFold }.map { node in
            (node.key.isEmpty ? node.title : "\(node.key) \(node.title)", node.target)
        }
    }

    /// ⌘↑: the nearest node above `target` that goes somewhere, or the plan
    /// at the top.
    public func parent(of target: OneTreeTarget, hint: String? = nil) -> OneTreeTarget? {
        guard let path = path(to: target, hint: hint) else { return nil }
        return path.dropLast().reversed().compactMap(\.target).first ?? (target == .plan ? nil : .plan)
    }

    /// The ids of the nodes above `target`'s: opened to reveal it.
    public func ancestors(of target: OneTreeTarget, hint: String? = nil) -> [String] {
        guard let path = path(to: target, hint: hint) else { return [] }
        return path.dropLast().map(\.id)
    }

    /// Every node, depth first.
    public var allNodes: [OneTreeNode] {
        var out: [OneTreeNode] = []
        func walk(_ nodes: [OneTreeNode]) {
            for node in nodes {
                out.append(node)
                walk(node.children)
            }
        }
        walk(roots)
        return out
    }
}

// MARK: - Narrowing

extension OneTree {
    /// `nodes` narrowed to those whose key or title holds `text`, with the
    /// ancestors that lead to them: what the navigator's filter (⌘F) shows.
    /// Nothing is cut with no text.
    public static func narrowed(_ nodes: [OneTreeNode], to text: String) -> [OneTreeNode] {
        guard !BoardFilter.isEmpty(text) else { return nodes }
        return nodes.compactMap { node in
            // The board's own rule, every word somewhere in the key or title.
            if BoardFilter.matches(key: node.key, title: node.title, text) { return node }
            let kept = narrowed(node.children, to: text)
            guard !kept.isEmpty else { return nil }
            var trimmed = node
            trimmed.children = kept
            trimmed.holdsAsk = trimmed.asks || kept.contains(where: \.holdsAsk)
            return trimmed
        }
    }

    /// Every node in `nodes` open: how a narrowed tree is drawn, so each
    /// match is in sight.
    public static func allOpen(_ nodes: [OneTreeNode]) -> OneTreeExpansion {
        var choices: [String: Bool] = [:]
        func walk(_ nodes: [OneTreeNode]) {
            for node in nodes where node.hasChildren {
                choices[node.id] = true
                walk(node.children)
            }
        }
        walk(nodes)
        return OneTreeExpansion(choices: choices)
    }
}
