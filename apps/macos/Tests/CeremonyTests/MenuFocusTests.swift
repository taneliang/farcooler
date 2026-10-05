import Foundation
import Testing

@testable import Far_Cooler

/// The menu bar's items are dimmed when what's on screen gives them nothing
/// to act on (ov-211).
///
/// The owner's report: with a terminal focused and no diff shown, the Diff
/// menu's Next Hunk, Read Commit by Commit and Mark as Reviewed were all
/// enabled. HIG, The menu bar: "If a menu bar item isn't actionable, disable
/// the action instead of hiding it from the menu."
struct DiffMenuFocusTests {
    private func commit(_ sha: String) -> ChangeCommit {
        try! JSONDecoder().decode(
            ChangeCommit.self,
            from: Data(#"{"sha":"\#(sha)","subject":"s","author":"a","timestamp":0}"#.utf8))
    }

    private var key: MainWindowFocus { MainWindowFocus(overlayOpen: false) }

    private var everything: [KeyPath<DiffMenuFocus, Bool>] {
        [\.nextHunk, \.previousHunk, \.nextFile, \.previousFile, \.readsCommits, \.nextCommit, \.previousCommit,
         \.marksReviewed]
    }

    @Test("With no diff focused, every Diff item is dimmed")
    func noDiffDimsEveryItem() {
        // A terminal focused: no `ChangesPane` publishes, so `diff` is nil.
        for item in everything {
            #expect(!DiffMenuFocus.allows(item, nil, in: key))
        }
    }

    @Test("A focused diff that can do everything enables every Diff item, in the key window only")
    func aFocusedDiffEnablesItsItems() {
        let all = DiffMenuFocus(
            nextHunk: true, previousHunk: true, nextFile: true, previousFile: true, readsCommits: true,
            nextCommit: true, previousCommit: true, marksReviewed: true)
        var palette = key
        palette.overlayOpen = true
        for item in everything {
            #expect(DiffMenuFocus.allows(item, all, in: key))
            // Settings key, or the palette over the window: the diff is behind.
            #expect(!DiffMenuFocus.allows(item, all, in: nil))
            #expect(!DiffMenuFocus.allows(item, all, in: palette))
        }
        #expect(!DiffMenuFocus.allows(\.nextHunk, DiffMenuFocus(), in: key))
    }

    @Test("An empty diff moves nowhere and has nothing to mark")
    func anEmptyDiffDoesNothing() {
        let diff = DiffMenuFocus.make(
            scope: .branch, files: 0, at: nil, hunks: [], lastHunk: nil, next: nil, previous: nil, commits: 0,
            unreviewed: nil)
        #expect(diff == DiffMenuFocus())
    }

    @Test("A branch diff wraps, so its moves always go somewhere")
    func aBranchDiffMoves() {
        let diff = DiffMenuFocus.make(
            scope: .branch, files: 3, at: 2, hunks: [], lastHunk: nil, next: nil, previous: nil, commits: 2,
            unreviewed: true)
        #expect(diff.nextFile && diff.previousFile && diff.nextHunk && diff.previousHunk)
        #expect(diff.readsCommits && diff.marksReviewed)
        // Next Commit only while reading commit by commit.
        #expect(!diff.nextCommit && !diff.previousCommit)
    }

    @Test("At the end of the last commit, Next stops; Previous still goes")
    func theLastCommitStops() {
        let diff = DiffMenuFocus.make(
            scope: .commit, files: 2, at: 1, hunks: ["h1", "h2"], lastHunk: "h2", next: nil,
            previous: commit("a"), commits: 2, unreviewed: false)
        #expect(!diff.nextFile && !diff.nextHunk && !diff.nextCommit)
        #expect(diff.previousFile && diff.previousHunk && diff.previousCommit)
        // Reviewed already: nothing to mark.
        #expect(!diff.marksReviewed)
        // A hunk still ahead in the file: Next Hunk goes to it.
        let earlier = DiffMenuFocus.make(
            scope: .commit, files: 2, at: 1, hunks: ["h1", "h2"], lastHunk: "h1", next: nil,
            previous: commit("a"), commits: 2, unreviewed: false)
        #expect(earlier.nextHunk && !earlier.nextFile)
    }
}

struct LayoutMenuFocusTests {
    private func pane(_ id: String, left: Int, focused: Bool = false, zoomed: Bool = false) -> PaneRect {
        PaneRect(id: id, short: id, title: nil, left: left, top: 0, columns: 40, rows: 24, focused: focused, zoomed: zoomed)
    }

    private func group(_ id: String, _ panes: [PaneRect]) -> PaneGroup {
        PaneGroup(id: id, name: id, active: true, columns: 81, rows: 24, layout: "", panes: panes)
    }

    @Test("One pane: nothing to zoom, arrange, step between or move out; no neighbors")
    func onePane() {
        let only = pane("a", left: 0, focused: true)
        let layout = LayoutMenuFocus.make(
            group: group("@1", [only]), here: only, layouts: [group("@1", [only])], switchesMode: false)
        #expect(layout.splits)
        #expect(!layout.zooms && !layout.arranges && !layout.stepsPanes && !layout.movesOut)
        #expect(!layout.left && !layout.right && !layout.above && !layout.below)
        #expect(!layout.stepsLayouts && !layout.switchesMode && !layout.zoomed)
    }

    @Test("Two panes side by side, two layouts: the left pane has a neighbor to its right only")
    func twoPanes() {
        let left = pane("a", left: 0, focused: true)
        let right = pane("b", left: 41, zoomed: true)
        let shown = group("@1", [left, right])
        let layout = LayoutMenuFocus.make(
            group: shown, here: left, layouts: [shown, group("@2", [pane("c", left: 0)])], switchesMode: true)
        #expect(layout.zooms && layout.arranges && layout.stepsPanes && layout.movesOut)
        #expect(layout.right && !layout.left && !layout.above && !layout.below)
        #expect(layout.stepsLayouts && layout.switchesMode && layout.zoomed)
    }

    @Test("A Layout item is dimmed with no layout on screen, another window key, or an overlay up")
    func layoutItemsNeedALayout() {
        var focus = MainWindowFocus(overlayOpen: false)
        #expect(!MainWindowFocus.lays(\.splits, focus))
        focus.layout = LayoutMenuFocus(panes: 1)
        #expect(MainWindowFocus.lays(\.splits, focus))
        #expect(!MainWindowFocus.lays(\.zooms, focus))
        #expect(!MainWindowFocus.lays(\.splits, nil))
        focus.overlayOpen = true
        #expect(!MainWindowFocus.lays(\.splits, focus))
    }
}

struct MainWindowMenuTests {
    @Test("Workspace and Terminal items go only where there's somewhere to go")
    func goingNeedsSomewhere() {
        var focus = MainWindowFocus(overlayOpen: false)
        for item: KeyPath<MainWindowFocus, Bool> in [
            \.goesBack, \.focuses, \.inWorkspace, \.nextWorktree, \.previousWorktree, \.stepsTerminals,
        ] {
            #expect(!MainWindowFocus.goes(item, focus))
        }
        focus.goesBack = true
        #expect(MainWindowFocus.goes(\.goesBack, focus))
        #expect(!MainWindowFocus.goes(\.goesBack, nil))
        focus.overlayOpen = true
        #expect(!MainWindowFocus.goes(\.goesBack, focus))
    }

    @Test("⌃⌘n and ⌘n pick only a terminal or a workspace that's there")
    func numbersPickOnlyWhatsThere() {
        var focus = MainWindowFocus(overlayOpen: false)
        focus.terminals = 2
        focus.workspaces = 3
        #expect(MainWindowFocus.picksTerminal(2, focus) && !MainWindowFocus.picksTerminal(3, focus))
        #expect(MainWindowFocus.picksWorkspace(3, focus) && !MainWindowFocus.picksWorkspace(4, focus))
        #expect(!MainWindowFocus.picksTerminal(1, nil) && !MainWindowFocus.picksWorkspace(1, nil))
    }

    @Test("The sidebar item says what it will do")
    func sidebarTitleSaysWhatItWillDo() {
        var focus = MainWindowFocus(overlayOpen: false)
        #expect(MainWindowFocus.sidebarTitle(focus) == "Hide Sidebar")
        focus.sidebarShown = false
        #expect(MainWindowFocus.sidebarTitle(focus) == "Show Sidebar")
    }

    @Test("Back goes somewhere only from a level, Focus, or a loose worktree beside a board")
    func backNeedsSomewhere() {
        typealias S = ContentView.Selection
        let board = S.workspace(host: "", workspace: "w", focus: nil)
        let task = S.workspace(host: "", workspace: "w", focus: .task("t"))
        let loose = S.looseWorktree(host: "", worktree: "l", terminal: nil)
        #expect(!ContentView.goesBack(focus: false, from: board, trail: nil, board: "w"))
        #expect(!ContentView.goesBack(focus: false, from: nil, trail: nil, board: nil))
        #expect(ContentView.goesBack(focus: true, from: board, trail: nil, board: "w"))
        #expect(ContentView.goesBack(focus: false, from: task, trail: nil, board: "w"))
        #expect(ContentView.goesBack(focus: false, from: loose, trail: nil, board: "w"))
        #expect(!ContentView.goesBack(focus: false, from: loose, trail: nil, board: nil))
    }
}

/// The call sites: every menu item in `Commands.swift` has its enabled state
/// wired to what it acts on.
///
/// The tests above check the rules; this checks the menu bar reads them.
/// Read as text, the way `ShortcutSheetTests` reads the chords, because a
/// `Commands` body can't be rendered in a test. Take one `.disabled(` off an
/// item, or point it at the wrong rule, and this goes red.
struct MenuWiringTests {
    private static let commands: String = {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/FarCooler/Commands.swift")
        return (try? String(contentsOf: url, encoding: .utf8)) ?? ""
    }()

    /// Each item's title, as written, and what its `.disabled(` must read.
    static let rules: [String: String] = [
        "Check for Updates…": "Updates.shared.isEnabled",
        "New Terminal": "hasWorktree",
        "New Worktree…": "MainWindowFocus.isKey(",
        "New Workspace…": "makesWorkspaces",
        "Add Repository…": "MainWindowFocus.isKey(",
        "Show Board": "MainWindowFocus.isKey(",
        "Show Plan": "MainWindowFocus.goes(\\.inWorkspace",
        "close.title": "CloseCommand.closes(",
        "Close Window": "MainWindowFocus.isKey(",
        "Close All": "CloseCommand.closes(",
        "Open in Editor": "hasWorktree",
        "Back": "\\.goesBack",
        "Forward": "\\.goesForward",
        "Go to Jump Bar": "\\.hasJumpBar",
        "Go to Line…": "findsInFile",
        "Focus": "\\.focuses",
        "Orchestrator": "\\.inWorkspace",
        "Navigator": "\\.inWorkspace",
        "Main Area": "\\.inWorkspace",
        "Next Task Tab": "stepsTaskTabs",
        "Previous Task Tab": "stepsTaskTabs",
        "Next Worktree": "\\.nextWorktree",
        "Previous Worktree": "\\.previousWorktree",
        "Switch Workspace…": "navigates",
        "Workspace \\(n)": "picksWorkspace(n",
        "Mark All as Read": "MainWindowFocus.marksRead(",
        "Next Terminal": "\\.stepsTerminals",
        "Previous Terminal": "\\.stepsTerminals",
        "Next Item That Needs You": "stepsToAttention",
        "Terminal \\(n)": "picksTerminal(n",
        "Split Right": "\\.splits",
        "Split Down": "\\.splits",
        "Move Pane Out": "\\.movesOut",
        "Zoom Pane": "zoomsPane",
        "Next Arrangement": "\\.arranges",
        "Even Out Panes": "\\.arranges",
        "preset.label": "\\.arranges",
        "Pane Left": "\\.left",
        "Pane Right": "\\.right",
        "Pane Above": "\\.above",
        "Pane Below": "\\.below",
        "Next Pane": "\\.stepsPanes",
        "Previous Pane": "\\.stepsPanes",
        "New Layout": "\\.splits",
        "Next Layout": "\\.stepsLayouts",
        "Previous Layout": "\\.stepsLayouts",
        "Switch Between Terminal and Chat": "\\.switchesMode",
        "Next Hunk": "DiffMenuFocus.allows(\\.nextHunk, diff",
        "Previous Hunk": "DiffMenuFocus.allows(\\.previousHunk, diff",
        "Next File": "DiffMenuFocus.allows(\\.nextFile, diff",
        "Previous File": "DiffMenuFocus.allows(\\.previousFile, diff",
        "Read Commit by Commit": "DiffMenuFocus.allows(\\.readsCommits, diff",
        "Next Commit": "DiffMenuFocus.allows(\\.nextCommit, diff",
        "Previous Commit": "DiffMenuFocus.allows(\\.previousCommit, diff",
        "Mark as Reviewed": "DiffMenuFocus.allows(\\.marksReviewed, diff",
        "MainWindowFocus.sidebarTitle(mainWindow)": "MainWindowFocus.togglesSidebar(",
        "Go to Anything…": "MainWindowFocus.isKey(",
        "Show Activity": "MainWindowFocus.isKey(",
        "Reload Fleet": "MainWindowFocus.isKey(",
        "MainWindowFocus.findTitle(mainWindow)": "MainWindowFocus.isKey(",
        "Keyboard Shortcuts": "MainWindowFocus.isKey(",
    ]

    /// Always actionable, so never dimmed.
    static let alwaysActs: Set<String> = ["About Far Cooler"]

    /// Every `Button(` or `Toggle(` in the menu bar, with the lines up to the
    /// next item or menu: its modifiers.
    static func items(_ source: String) -> [(title: String, body: String)] {
        let stops = ["Divider()", "Section {", "Menu(", "CommandMenu(", "CommandGroup(", "ForEach("]
        var out: [(title: String, body: String)] = []
        for line in source.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("Button(") || trimmed.hasPrefix("Toggle(") {
                let open = trimmed.dropFirst("Button(".count)
                let title: String
                if open.hasPrefix("\"") {
                    title = String(open.dropFirst().prefix { $0 != "\"" })
                } else {
                    title = open.range(of: ") {").map { String(open[..<$0.lowerBound]) } ?? String(open)
                }
                out.append((title, line))
            } else if stops.contains(where: { trimmed.hasPrefix($0) }) || trimmed.hasPrefix("//") {
                // A comment or a menu ends the item, so a later one's
                // `.disabled(` can't be credited to it.
                if !out.isEmpty, !trimmed.hasPrefix("//") { out[out.count - 1].body += "\n<end>" }
            } else if !out.isEmpty, !out[out.count - 1].body.hasSuffix("<end>") {
                out[out.count - 1].body += "\n" + line
            }
        }
        return out
    }

    @Test("Every menu item's enabled state reads what it acts on")
    func everyItemIsWired() {
        let found = Self.items(Self.commands)
        #expect(found.count > 50, "only \(found.count) items read from Commands.swift")
        for (title, body) in found where !Self.alwaysActs.contains(title) {
            guard let rule = Self.rules[title] else {
                Issue.record("“\(title)” has no rule here: say what it acts on, and dim it when it can't")
                continue
            }
            let disabled = body.split(separator: "\n").first { $0.contains(".disabled(") }
            #expect(disabled != nil, "“\(title)” is never dimmed")
            #expect(disabled?.contains(rule) == true, "“\(title)” is dimmed by \(disabled ?? "nothing"), not \(rule)")
        }
        let titles = Set(found.map(\.title))
        #expect(Set(Self.rules.keys).subtracting(titles).isEmpty, "rules for items that are gone")
    }

    @Test("The sidebar item has no fixed Toggle Sidebar title")
    func noToggleSidebar() {
        #expect(!Self.commands.contains("\"Toggle Sidebar\""))
    }
}
