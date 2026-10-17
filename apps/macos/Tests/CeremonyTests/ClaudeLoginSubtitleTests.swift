import AgentKit
import Testing

@testable import Far_Cooler

/// ov-597: a pane alone in its layout draws no header, so the window's
/// subtitle says whose Claude login its agent runs on.
struct ClaudeLoginSubtitleTests {
    private static let alex = PaneClaudeLogin(handle: "alex", name: "Alex Rivera", owner: false)

    private static func layout(panes ids: [String], login: PaneClaudeLogin?) -> ShownLayout {
        var terminals = ids.map { Terminal(id: $0, short: $0, title: $0, preset: "claude", state: "running", epoch: 0) }
        terminals[0].claudeLogin = login
        let worktree = Worktree(
            id: "w", short: "w", task: "lane", branch: "main", repository: "shop", host: "",
            path: "/tmp/shop", state: "active", terminals: terminals, repositoryID: nil,
            workspace: nil, openTasks: nil)
        let panes = ids.map {
            PaneRect(id: $0, short: $0, title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: false, zoomed: false)
        }
        let group = PaneGroup(id: "@1", name: "", active: true, columns: 80, rows: 24, layout: "a", panes: panes)
        return ShownLayout(column: .worktree, worktree: worktree, group: group, groups: [group])
    }

    @Test("A lone pane's subtitle says whose login it uses")
    func lonePane() {
        let shown = Self.layout(panes: ["t1"], login: Self.alex)
        #expect(ContentView.subtitle("shop / main", lone: shown, viewer: "") == "shop / main · Uses Alex Rivera’s Claude login")
        #expect(ContentView.subtitle("shop / main", lone: shown, viewer: "alex") == "shop / main · Uses your Claude login")
    }

    @Test("A tiled layout's subtitle is untouched: its headers say it")
    func tiled() {
        let shown = Self.layout(panes: ["t1", "t2"], login: Self.alex)
        #expect(ContentView.subtitle("shop / main", lone: shown, viewer: "") == "shop / main")
    }

    @Test("A lone pane the runner names no login for leaves the subtitle alone")
    func noLogin() {
        let shown = Self.layout(panes: ["t1"], login: nil)
        #expect(ContentView.subtitle("shop / main", lone: shown, viewer: "") == "shop / main")
    }
}
