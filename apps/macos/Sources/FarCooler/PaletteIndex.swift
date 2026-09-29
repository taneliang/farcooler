import AgentKit
import Foundation

/// What running a palette row does.
///
/// A value, not a closure. The panel decides WHAT was chosen and the window
/// decides HOW to carry it out — the same split `AppCommand` already uses for
/// menu items, and for the same reason: the palette then has no opinion about
/// selection, layouts or the daemon, and the window keeps exactly one
/// implementation of "go to that terminal" no matter who asked.
enum PaletteAction: Hashable {
    case openTerminal(worktree: String, terminal: String)
    case openWorktree(String)
    case newTerminal(worktree: String)
    /// Carries what was typed, because in this panel the query IS the
    /// description far more often than it is a search for one. Its title is
    /// "New Worktree…": it makes a worktree and files no task.
    case newWorktree(String)
    /// A workspace: its conversation beside its board. Its orchestrator's
    /// row lands here too.
    case openWorkspace(host: String, id: String)
    /// A task, in its workspace's task column.
    case openTask(host: String, workspace: String, id: String)
    /// Terminal ⟷ chat, for one specific pane. The palette is one of the two
    /// places the plan names for this toggle — the other is `⌃B a` — because
    /// the pane itself grew no button for it.
    case togglePaneMode(worktree: String, terminal: String)
}

/// One row, already resolved into the handful of things a row can draw.
///
/// Flattened rather than kept as `(Worktree, Terminal)` pairs so that the grid
/// and the list render from the same value: they differ in shape, not in what
/// they know, and two views deriving a subtitle separately is how they end up
/// disagreeing about what a terminal is called.
struct PaletteEntry: Identifiable, Equatable {
    let id: String
    let action: PaletteAction
    let title: String
    let detail: String
    /// Set only for a terminal — it is the only kind of row with a status to
    /// show and a screen worth previewing.
    var terminal: Terminal?
    /// For everything that is not a terminal. A terminal gets a `StatusGlyph`
    /// instead, so that the palette speaks the same indicator vocabulary as the
    /// sidebar rather than inventing a second one.
    var symbol: String?
    /// The quiet word at the right of a search row.
    ///
    /// The filtered list deliberately mixes three kinds of thing, and at a
    /// glance they are indistinguishable — "auth" is as plausibly a terminal as
    /// a worktree as the task you are about to start. One word per row is
    /// cheaper than three headed sections, and sections would freeze the order
    /// of the results, which is the one thing ranking exists to decide.
    var kind: String
}

/// Everything the palette can offer, and in what order.
///
/// Pure: no `DaemonClient`, no view state, nothing to await. That is what makes
/// the ordering rules — which are the whole product here — readable in one
/// place instead of spread through a view body.
enum PaletteIndex {
    /// How many tiles the switcher shows before it stops being a switcher.
    ///
    /// A grid is for recognizing something, and recognition does not scale: past
    /// a dozen tiles the eye starts reading rather than seeing, which is what
    /// typing is for. The cap is not a limit on what you can reach — everything
    /// is one keystroke away in the filtered list — it is a limit on what is
    /// worth showing without being asked.
    static let switcherLimit = 12

    /// Terminals whose screen is worth previewing.
    ///
    /// A terminal is its process: once that has gone there is no screen to show
    /// and a tile of stale text would claim otherwise. Exited and lost terminals
    /// are still findable by typing, where they are a row of facts rather than a
    /// picture of something that is not happening.
    private static func isLive(_ terminal: Terminal) -> Bool {
        let kind = StateKind.parse(terminal.state)
        return kind == .running || kind == .starting
    }

    /// The switcher's order: where you have most recently been.
    ///
    /// Alt-Tab's rule, and the reason for it: the pane you are in now is not the
    /// first entry, because you are already there. The most recent one you are
    /// NOT in comes first, so ⌘P then Return puts you back where you just were
    /// and holding the panel open walks further back. Without that, the single
    /// most useful thing a switcher does — bounce between two agents — needs you
    /// to aim at a tile every time.
    ///
    /// It used to order by `activitySince`, which is when an AGENT last changed
    /// what it was doing. That is a real signal but a different question: it puts
    /// a busy agent you have never opened above the pane you left ten seconds
    /// ago. Agent activity is still the tie-break for panes you have not visited,
    /// where "something happened here" is the only ordering available.
    @MainActor
    static func recent(in worktrees: [Worktree], limit: Int = switcherLimit) -> [PaletteEntry] {
        var live: [(worktree: Worktree, terminal: Terminal)] = []
        for worktree in worktrees {
            for terminal in worktree.terminals where isLive(terminal) {
                live.append((worktree, terminal))
            }
        }

        let log = VisitLog.shared
        // Sorted through the original index, because Swift's sort is not stable
        // and a fleet where several terminals tie would otherwise reshuffle its
        // tiles on every refresh — a grid that rearranges itself under the hand
        // is worse than one in the wrong order.
        let ordered = live.enumerated().sorted { left, right in
            let a = left.element.terminal, b = right.element.terminal

            // The pane you are in sinks to the bottom rather than vanishing: it
            // is still somewhere you can go, just never the default.
            let aCurrent = log.isCurrent(a.id), bCurrent = log.isCurrent(b.id)
            if aCurrent != bCurrent { return bCurrent }

            let aVisit = log.rank(a.id), bVisit = log.rank(b.id)
            if aVisit != bVisit { return aVisit > bVisit }

            // Neither has been visited this session. Fall back to which agent
            // moved most recently, which is the only thing left that means
            // anything.
            let aActive = a.activitySince ?? 0, bActive = b.activitySince ?? 0
            if aActive != bActive { return aActive > bActive }
            return left.offset < right.offset
        }

        return ordered.prefix(limit).map { entry(for: $0.element.terminal, in: $0.element.worktree) }
    }

    /// Everything the query could mean, best first.
    ///
    /// `current` is the worktree being looked at, used only when nothing else
    /// answers the question: with no worktree matched, "new terminal" still has
    /// an obvious place to go, and offering it there beats offering nothing.
    static func matching(
        _ query: String, in worktrees: [Worktree], current: String? = nil,
        currentTerminal: Terminal? = nil, workspaces: [PaletteWorkspace] = [], tasks: [PaletteTask] = [],
        limit: Int = 20
    ) -> [PaletteEntry] {
        var scored: [(entry: PaletteEntry, score: Int)] = []
        var bestWorktree: (worktree: Worktree, score: Int)?

        // Workspaces first among equals, and each kept on its own: a
        // workspace with no matching worktree is still found by its name.
        for place in workspaces {
            if let score = Fuzzy.score(place.name, query) {
                scored.append((
                    PaletteEntry(
                        id: "workspace:\(place.host)|\(place.id)", action: .openWorkspace(host: place.host, id: place.id),
                        title: place.name, detail: "Workspace · \(place.repository)",
                        symbol: "square.stack.3d.up", kind: "workspace"),
                    score))
            }
            if place.hasOrchestrator,
                let score = ["\(place.name) Orchestrator", "Orchestrator"].compactMap({ Fuzzy.score($0, query) }).max()
            {
                scored.append((
                    PaletteEntry(
                        id: "orchestrator:\(place.host)|\(place.id)",
                        action: .openWorkspace(host: place.host, id: place.id),
                        title: "\(place.name) Orchestrator", detail: "Orchestrator · \(place.repository)",
                        symbol: "person.wave.2", kind: "orchestrator"),
                    score))
            }
        }
        // Tasks from the boards already read, by key and by title.
        for task in tasks {
            guard let score = [task.key, task.title, "\(task.key) \(task.title)"].compactMap({ Fuzzy.score($0, query) }).max()
            else { continue }
            scored.append((
                PaletteEntry(
                    id: "task:\(task.host)|\(task.id)",
                    action: .openTask(host: task.host, workspace: task.workspace, id: task.id),
                    title: "\(task.key) \(task.title)", detail: "\(task.workspaceName) · \(task.status.title)",
                    symbol: "checklist", kind: "task"),
                score))
        }

        for worktree in worktrees {
            let fields = [worktree.task, worktree.branch, worktree.repository ?? ""]
            if let score = fields.compactMap({ Fuzzy.score($0, query) }).max() {
                scored.append((entry(for: worktree), score))
                if score > (bestWorktree?.score ?? Int.min) {
                    bestWorktree = (worktree, score)
                }
            }

            for terminal in worktree.terminals {
                // Matched against where it lives as well as what it is called.
                // Nobody remembers that the agent in "refactor api" is named
                // `claude` — there are four of those — and everybody remembers
                // "refactor api".
                let fields = [terminal.label, worktree.task]
                guard let score = fields.compactMap({ Fuzzy.score($0, query) }).max() else {
                    continue
                }
                scored.append((entry(for: terminal, in: worktree), score))
            }
        }

        let navigation = scored.enumerated().sorted { left, right in
            left.element.score == right.element.score
                ? left.offset < right.offset
                : left.element.score > right.element.score
        }
        .prefix(limit).map(\.element.entry)

        // Creation always sits BELOW everything you could merely go to, however
        // well it scores. Return with the highlight untouched is the most common
        // keystroke in a palette, and it must never be the one that launches a
        // process — going somewhere is free and undoable, starting an agent is
        // neither.
        var actions: [PaletteEntry] = []

        // Scoped to the pane actually being looked at, so it never floats
        // free of the terminal it would act on — unlike "new terminal" and
        // "new worktree" below, which have an obvious home even with nothing
        // selected, this one has none without a terminal to name.
        if let currentTerminal, let owner = worktrees.first(where: { worktree in
            worktree.terminals.contains { $0.id == currentTerminal.id }
        }) {
            actions.append(
                PaletteEntry(
                    id: "toggle-pane-mode:\(currentTerminal.id)",
                    action: .togglePaneMode(worktree: owner.id, terminal: currentTerminal.id),
                    title: currentTerminal.isAgentPane
                        ? "Switch \(currentTerminal.label) to Terminal"
                        : "Switch \(currentTerminal.label) to Chat",
                    detail: owner.task,
                    symbol: currentTerminal.isAgentPane
                        ? "terminal" : "bubble.left.and.bubble.right",
                    kind: "action"))
        }

        let target = bestWorktree?.worktree
            ?? worktrees.first { $0.id == current }
        if let target {
            actions.append(
                PaletteEntry(
                    id: "new-terminal:\(target.id)",
                    action: .newTerminal(worktree: target.id),
                    title: "New Terminal in \(target.task)",
                    // The target is a worktree, not a project, because that is
                    // what a terminal is actually created in — a shell has to
                    // open somewhere, and a project is several somewheres.
                    detail: target.windowSubtitle,
                    symbol: "plus",
                    kind: "action"))
        }
        let described = query.trimmingCharacters(in: .whitespacesAndNewlines)
        actions.append(
            PaletteEntry(
                id: "new-task",
                action: .newWorktree(described),
                title: described.isEmpty ? "New Worktree…" : "New Worktree “\(described)”",
                detail: "Describe it and go",
                symbol: "sparkle",
                kind: "action"))

        return navigation + actions
    }

    static func entry(for terminal: Terminal, in worktree: Worktree) -> PaletteEntry {
        PaletteEntry(
            id: "terminal:\(terminal.id)",
            action: .openTerminal(worktree: worktree.id, terminal: terminal.id),
            title: terminal.label,
            detail: [worktree.task, worktree.repository ?? ""]
                .filter { !$0.isEmpty }
                .joined(separator: " · "),
            terminal: terminal,
            kind: "terminal")
    }

    static func entry(for worktree: Worktree) -> PaletteEntry {
        PaletteEntry(
            id: "worktree:\(worktree.id)",
            action: .openWorktree(worktree.id),
            title: worktree.task,
            detail: worktree.windowSubtitle,
            symbol: "arrow.triangle.branch",
            kind: "worktree")
    }
}

/// Subsequence matching, scored.
///
/// Deliberately not a real fuzzy matcher. The corpus is at most a few hundred
/// short strings, every one of them named by the person typing, so the ranking
/// only has to separate "matched at the start of a word" from "matched four
/// letters scattered through a sentence". Anything more sophisticated would be
/// tuned against inputs nobody has yet produced.
///
/// It takes the FIRST occurrence of each character rather than the best one,
/// which is not optimal scoring — "ttt" against "tit for tat" scores lower than
/// it could. The optimal version is a dynamic program over both strings, and
/// buying a better answer to a question users are not asking is not worth
/// running it on every keystroke.
enum Fuzzy {
    /// A score, or nil when the query is not a subsequence at all. Higher is
    /// better; the absolute value means nothing outside a single comparison.
    static func score(_ candidate: String, _ query: String) -> Int? {
        // Whitespace is dropped from the query so that "ref api" still finds
        // "refactor-api" — people type the spaces they see, not the ones the
        // string has.
        let needle = Array(query.lowercased().filter { !$0.isWhitespace })
        guard !needle.isEmpty else { return 0 }
        let haystack = Array(candidate.lowercased())
        guard !haystack.isEmpty else { return nil }

        var total = 0
        var cursor = 0
        var previous = -2
        var first = -1

        for wanted in needle {
            guard let at = haystack[cursor...].firstIndex(of: wanted) else { return nil }
            total += 4
            // A run of adjacent characters is what someone typing a prefix
            // produces, and it is by far the strongest signal that this is the
            // string they meant.
            if at == previous + 1 { total += 8 }
            if at == 0 || !(haystack[at - 1].isLetter || haystack[at - 1].isNumber) {
                total += 6
            }
            // Scatter is the strongest evidence AGAINST a match, and it has to
            // be priced or the ranking gets it backwards: "hai" is a
            // subsequence of the branch `handoff-from-another-machine`, and
            // without this that outranks the worktree literally called "write a
            // haiku", because it happens to start one character earlier.
            if first >= 0 { total -= min(at - previous - 1, 6) }
            if first < 0 { first = at }
            previous = at
            cursor = at + 1
        }

        // Two mild preferences, in the order they matter: matching near the
        // front, and matching a short name. Between "api" and "the api client
        // rewrite" the first is nearly always what was meant.
        total -= min(first, 10)
        total -= min(haystack.count / 8, 6)
        return total
    }
}

/// A workspace, as the palette finds it.
struct PaletteWorkspace: Equatable {
    var host: String
    var id: String
    var name: String
    /// Its repository's display name.
    var repository: String
    var hasOrchestrator: Bool
}

/// A task on a board this window has read, as the palette finds it.
struct PaletteTask: Equatable {
    var host: String
    var workspace: String
    var workspaceName: String
    var id: String
    var key: String
    var title: String
    var status: TaskStatus
}
