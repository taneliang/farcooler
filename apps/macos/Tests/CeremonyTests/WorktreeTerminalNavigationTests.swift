import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A terminal opened in a worktree is where it lives (ov-267): under its
/// worktree in the sidebar, the jump bar's last segment after its worktree,
/// and one click from its worktree; and every jump bar segment is two
/// controls, its label going to what it names, its caret opening the menu of
/// what's beside it.
@MainActor
struct WorktreeTerminalNavigationTests {
    typealias Selection = ContentView.Selection
    static let billing = "billing"

    static func terminal(_ id: String, title: String = "shell", role: String? = nil) -> Terminal {
        var t = Terminal(id: id, short: id, title: title, preset: "shell", state: "running", epoch: 0)
        t.role = role
        return t
    }

    static let worktree = Worktree(
        id: "pdf", short: "pdf", task: "invoice pdf", branch: "feat/pdf", repository: "overnight", host: "",
        path: "/tmp/pdf", state: "active",
        terminals: [terminal("t1"), terminal("t2", title: "server"), terminal("o", title: "claude", role: "orchestrator")])
    static let fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [worktree], branchPrefix: nil)

    static func at(_ terminal: String?) -> Selection {
        .workspace(host: "", workspace: billing, focus: .worktree(worktree.id, terminal: terminal))
    }

    // MARK: - Drawing and clicking, offscreen

    /// Where each probed view was drawn, by its id (`identified`).
    @MainActor final class Seen { var views: [String: CGRect] = [:] }

    struct Probed<Content: View>: View {
        let seen: Seen
        let content: Content

        var body: some View {
            content
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = seen.views = Dictionary(
                            probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                }
        }
    }

    final class ClickWindow: NSWindow {
        override var canBecomeKey: Bool { true }
    }

    /// `content` in an offscreen window, and what it probed.
    static func show<V: View>(_ content: V, size: CGSize) async throws -> (NSWindow, Seen) {
        let seen = Seen()
        let window = ClickWindow(
            contentRect: NSRect(x: -6000, y: -6000, width: size.width, height: size.height), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(
            rootView: Probed(seen: seen, content: content.frame(width: size.width, height: size.height, alignment: .topLeading)))
        window.orderFrontRegardless()
        try await settle(window)
        return (window, seen)
    }

    static func settle(_ window: NSWindow) async throws {
        for _ in 0..<10 {
            window.contentView?.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    /// A click in the middle of `id`, sent to the window itself: no event
    /// reaches the system, or the owner's screen.
    static func click(_ id: String, in window: NSWindow, _ seen: Seen) async throws {
        let frame = try #require(seen.views[id], "nothing drawn as \(id): \(seen.views.keys.sorted())")
        let at = NSPoint(x: frame.midX, y: window.frame.height - frame.midY)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try #require(
                NSEvent.mouseEvent(
                    with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1))
            window.sendEvent(event)
        }
        try await settle(window)
    }

    @Test("The sidebar lists a worktree's terminals under it, the open one selected, and a click opens one")
    func sidebarListsTerminals() async throws {
        #expect(BoardWorktrees.terminals(of: Self.worktree, in: Self.fleet).map(\.id) == ["t1", "t2"])
        final class Opened { var terminals: [String] = [] }
        let opened = Opened()
        let worktrees = BoardWorktrees(
            shown: [Self.worktree], selected: Self.worktree.id,
            worktreeTerminals: [Self.worktree.id: BoardWorktrees.terminals(of: Self.worktree, in: Self.fleet)],
            selectedTerminal: "t2", onOpenTerminal: { _, t in opened.terminals.append(t.id) })
        #expect(!worktrees.rowSelected(Self.worktree), "the worktree's row lit beside its open terminal")

        let (window, seen) = try await Self.show(BoardWorktreesSection(worktrees: worktrees, keyed: false), size: CGSize(width: 300, height: 300))
        defer { window.close() }
        #expect(seen.views["board-worktree-terminal-t2"] != nil, "no row for the open terminal")
        #expect(seen.views["board-worktree-terminal-o"] == nil, "an orchestrator listed as a shell")
        try await Self.click("board-worktree-terminal-t1", in: window, seen)
        #expect(opened.terminals == ["t1"])
    }

    // MARK: - The jump bar

    @Test("The open terminal is the jump bar's last segment, after its worktree, the one you're at")
    func terminalSegment() throws {
        #expect(ContentView.openTerminal(in: Self.worktree, place: Self.at("t2"), keyPane: nil) == "t2")
        let key = PaneRef(host: "", worktree: Self.worktree.id, terminal: "t1")
        #expect(ContentView.openTerminal(in: Self.worktree, place: Self.at(nil), keyPane: key) == "t1")
        #expect(ContentView.openTerminal(in: Self.worktree, place: Self.at(nil), keyPane: nil) == nil)

        let terminal = try #require(TerminalCrumb.of("t2", in: Self.worktree, selection: Self.at("t2"), fleet: Self.fleet))
        #expect(terminal.title == "server")
        #expect(terminal.jumpMenu.items.map(\.title) == ["shell", "server"])
        #expect(terminal.jumpMenu.current?.title == "server")
        #expect(TerminalCrumb.of("o", in: Self.worktree, selection: nil, fleet: Self.fleet) == nil)

        let crumbs = [WorkspaceNavigation.Crumb(title: "Billing", target: .workspace(host: "", workspace: Self.billing, focus: nil))]
        let segment = WorktreeCrumb(title: "invoice pdf", isHere: true, tasks: [], loose: [], terminal: terminal)
        let pieces = DrillBreadcrumb.pieces(crumbs, worktrees: segment, menus: 1).map(\.kind)
        #expect(pieces == [.crumb(0), .crumbCaret(0), .separator, .menuIcon, .menuTitle, .menuChevron, .separator,
                           .terminalTitle, .terminalChevron])
        let styles = DrillBreadcrumb.pieces(crumbs, worktrees: segment, menus: 1)
        #expect(styles.filter { $0.style.tone == .primary }.map(\.kind) == [.terminalTitle], "one segment is where you are")
        let menus = DrillBreadcrumb.segmentMenus(crumbs: 1, menus: [JumpMenu([])], worktrees: segment)
        #expect(menus.count == 3 && menus[2].items.map(\.title) == ["shell", "server"])
    }

    @Test("From one of its terminals, the worktree segment's label goes back to the worktree")
    func backToTheWorktree() {
        let back = ContentView.backToWorktree(Self.worktree, from: Self.at("t2"), in: Self.fleet)
        #expect(back == .go(ContentView.opening(Self.worktree, terminal: nil, in: Self.fleet)))
        #expect(WorkspaceScreen.namedTerminal(back?.place) == nil && ContentView.openedWhole(back?.place)?.worktree == "pdf")
        #expect(ContentView.backToWorktree(Self.worktree, from: Self.at(nil), in: Self.fleet) == nil, "already there")
    }

    // MARK: - Label and caret

    static let workspaceCrumb = WorkspaceNavigation.Crumb(
        title: "Billing", target: .workspace(host: "", workspace: billing, focus: nil))

    @Test("Each label goes to what it names; each caret opens its own segment's menu")
    func clickPaths() {
        let terminal = TerminalCrumb(title: "server", siblings: [])
        let fromTerminal = WorktreeCrumb(
            title: "invoice pdf", isHere: true, tasks: [], loose: [], target: .go(Self.at(nil)), terminal: terminal)
        let crumbs = [Self.workspaceCrumb]
        func click(_ kind: DrillBreadcrumb.Piece.Kind, _ worktrees: WorktreeCrumb?) -> DrillBreadcrumb.Click {
            DrillBreadcrumb.click(kind, crumbs: crumbs, worktrees: worktrees)
        }
        // The workspace: its label goes there, its caret lists the others.
        #expect(click(.crumb(0), fromTerminal) == .go(.go(.workspace(host: "", workspace: Self.billing, focus: nil))))
        #expect(click(.crumbCaret(0), fromTerminal) == .menu(0))
        // The worktree: up from its terminal by the label, sideways by the caret.
        #expect(click(.menuIcon, fromTerminal) == .go(.go(Self.at(nil))))
        #expect(click(.menuChevron, fromTerminal) == .menu(1))
        // The terminal: where you are; its caret lists the worktree's others.
        #expect(click(.terminalTitle, fromTerminal) == .none)
        #expect(click(.terminalChevron, fromTerminal) == .menu(2))
        // The worktree open whole is where you are: its label goes nowhere.
        let whole = WorktreeCrumb(title: "invoice pdf", isHere: true, tasks: [], loose: [])
        #expect(click(.menuIcon, whole) == .none)
        // "Worktrees" beside a task with several names no one place: its menu.
        let several = WorktreeCrumb(title: "Worktrees", isHere: false, tasks: [], loose: [])
        #expect(click(.menuIcon, several) == .menu(1))
        // A task's one worktree: its label goes in, keeping the task as the way back.
        let task: Selection = .workspace(host: "", workspace: Self.billing, focus: .task("p1"))
        let one = WorktreeCrumb(
            title: "pdf", isHere: false, tasks: [], loose: [],
            opens: WorkspaceWorktrees.MenuItem(title: "pdf", target: Self.at(nil), current: false, trail: task))
        #expect(click(.menuIcon, one) == .go(.open(Self.at(nil), from: task)))
    }

    @Test("A caret says what its menu holds, and a label where it goes")
    func names() {
        #expect(DrillBreadcrumb.labelName("Main") == "Go to Main")
        #expect(DrillBreadcrumb.caretName(0, crumbs: 2) == "Show other workspaces")
        #expect(DrillBreadcrumb.caretName(1, crumbs: 2) == "Show other places in this workspace")
        #expect(DrillBreadcrumb.caretName(2, crumbs: 2) == "Show other worktrees")
        #expect(DrillBreadcrumb.caretName(3, crumbs: 2) == "Show other terminals")
    }

    @Test("In a real bar, the labels go and the carets are their own controls, at least 20 pt wide")
    func inARealBar() async throws {
        final class Heard { var jumps: [JumpTarget] = [] }
        let heard = Heard()
        let terminal = TerminalCrumb(title: "server", siblings: [])
        let bar = DrillBreadcrumb(
            crumbs: [Self.workspaceCrumb],
            worktrees: WorktreeCrumb(
                title: "invoice pdf", isHere: true, tasks: [], loose: [], target: .go(Self.at(nil)), terminal: terminal),
            onGo: { _ in Issue.record("a label went by the plain way back") }, onClose: nil,
            menus: JumpMenuSource(count: 1, build: { [JumpMenu([])] }), onJump: { heard.jumps.append($0) })
        let (window, seen) = try await Self.show(bar, size: CGSize(width: 600, height: 60))
        defer { window.close() }
        try await Self.click("jump-label-0", in: window, seen)
        try await Self.click("breadcrumb-worktrees", in: window, seen)
        #expect(heard.jumps == [.go(.workspace(host: "", workspace: Self.billing, focus: nil)), .go(Self.at(nil))])
        for index in 0...2 {
            let caret = try #require(seen.views["jump-caret-\(index)"], "no caret for segment \(index)")
            #expect(caret.width >= JumpCaretButton.minWidth, "segment \(index)'s caret is \(caret.width) wide")
            let label = index == 0 ? seen.views["jump-label-0"] : index == 1 ? seen.views["breadcrumb-worktrees"] : nil
            if let label { #expect(!label.intersects(caret), "segment \(index)'s caret overlaps its label") }
        }
        try await Self.click("jump-caret-1", in: window, seen)
        #expect(heard.jumps.count == 2, "a caret navigated")
        #expect(window.childWindows?.isEmpty == false || NSApp.windows.contains { $0.className.contains("Popover") && $0.isVisible },
                "the caret opened no menu")
    }

    // MARK: - ⌘K

    @Test("⌘K's terminal rows say where the terminal is, as the jump bar reads down to it")
    func paletteSaysThePath() {
        let entry = PaletteIndex.entry(for: Self.terminal("t2", title: "server"), in: Self.worktree)
        #expect(entry.title == "server")
        #expect(entry.detail == "overnight › invoice pdf")
    }
}
