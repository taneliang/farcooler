import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Two terminals with one name in a worktree (ov-267 review, L15): every new
/// terminal is called "shell", so the default case showed two identical lines
/// in the sidebar, the jump bar and ⌘K. They are told apart the way Terminal.app
/// does it, by an ordinal from the second on: "shell", "shell 2".
@MainActor
struct DuplicateTerminalNamesTests {
    typealias Nav = WorktreeTerminalNavigationTests

    static let worktree = Worktree(
        id: "pdf", short: "pdf", task: "invoice pdf", branch: "feat/pdf", repository: "overnight", host: "",
        path: "/tmp/pdf", state: "active",
        terminals: [Nav.terminal("t1"), Nav.terminal("t2"), Nav.terminal("t3", title: "server"), Nav.terminal("t4")])
    static let fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [worktree], branchPrefix: nil)

    @Test("The second and later terminals of one name take an ordinal; the first and a lone one don't")
    func names() {
        let w = Self.worktree
        #expect(w.terminals.map { w.name(of: $0) } == ["shell", "shell 2", "server", "shell 3"])
        let lone = Worktree(
            id: "x", short: "x", task: "x", branch: "x", repository: "r", host: "", path: "/x", state: "active",
            terminals: [Nav.terminal("a"), Nav.terminal("b", title: "server")])
        #expect(lone.terminals.map { lone.name(of: $0) } == ["shell", "server"], "a lone shell was numbered")
    }

    @Test("The jump bar names them apart, in its last segment and its menu")
    func jumpBar() throws {
        let crumb = try #require(TerminalCrumb.of("t2", in: Self.worktree, selection: nil, fleet: Self.fleet))
        #expect(crumb.title == "shell 2")
        #expect(crumb.jumpMenu.items.map(\.title) == ["shell", "shell 2", "server", "shell 3"])
    }

    @Test("⌘K's rows name them apart")
    func palette() {
        let titles = Self.worktree.terminals.map { PaletteIndex.entry(for: $0, in: Self.worktree).title }
        #expect(titles == ["shell", "shell 2", "server", "shell 3"])
    }

    @Test("The sidebar's rows name them apart")
    func sidebar() async throws {
        let worktrees = BoardWorktrees(
            shown: [Self.worktree], selected: Self.worktree.id,
            worktreeTerminals: [Self.worktree.id: BoardWorktrees.terminals(of: Self.worktree, in: Self.fleet)],
            selectedTerminal: "t2")
        let (window, seen) = try await Nav.show(
            BoardWorktreesSection(worktrees: worktrees, keyed: false), size: CGSize(width: 300, height: 300))
        defer { window.close() }
        // A name's drawn width says which name it is: "shell 2" is wider than
        // "shell", and "server" differs from both.
        func width(_ id: String) throws -> CGFloat {
            try #require(seen.views["board-worktree-terminal-name-\(id)"], "no name drawn for \(id)").width
        }
        let (first, second, third, fourth) = (try width("t1"), try width("t2"), try width("t4"), try width("t3"))
        #expect(second > first, "the second shell reads like the first: \(first), \(second)")
        #expect(abs(second - third) <= 0.5, "'shell 2' and 'shell 3' differ by a digit: \(second), \(third)")
        #expect(fourth != first, "server drew as wide as shell")
    }
}
