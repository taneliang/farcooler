import AgentKit
import Combine
import Foundation
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

/// ov-598: a project terminal's navigator row says whose login it uses (a lone
/// pane draws no header), and the viewer it compares against reaches a redraw
/// when the runner's people arrive.
@MainActor
@Suite(.serialized)
struct LonePaneLoginTests {
    init() { UserDefaults.standard.removeObject(forKey: "talk.you.box") }

    @Test("A project terminal's row says whose login it uses, as the person it is or as yours")
    func rowSaysIt() {
        var terminal = Terminal(id: "t1", short: "t1", title: "t", preset: "shell", state: "running", epoch: 0)
        var terminals = ProjectTerminals(claudeLoginViewer: "")
        #expect(terminals.claudeLogin(of: terminal) == nil)
        terminal.claudeLogin = PaneClaudeLogin(handle: "alex", name: "Alex Rivera", owner: false)
        #expect(terminals.claudeLogin(of: terminal) == "Uses Alex Rivera’s Claude login")
        terminals.claudeLoginViewer = "alex"
        #expect(terminals.claudeLogin(of: terminal) == "Uses your Claude login")
    }

    @Test("The row the section builds carries the login sentence")
    func rowCarriesIt() {
        var terminal = Terminal(id: "t1", short: "t1", title: "t", preset: "shell", state: "running", epoch: 0)
        terminal.claudeLogin = PaneClaudeLogin(handle: "alex", name: "Alex Rivera", owner: false)
        let terminals = ProjectTerminals(terminals: [terminal], claudeLoginViewer: "alex")
        #expect(ProjectTerminalRow(terminal, in: terminals, keyed: false).claudeLogin == "Uses your Claude login")
    }

    @Test("The viewer is nil until the people are read, then the redraw is asked for and it says who")
    func viewerArrivesWithARedraw() async {
        let client = DaemonClient(target: "box")
        client.daemonBuild = DaemonBuild(version: "0.1.0", matches: true, platform: "macos", capabilities: ["representatives"])
        client.commandRunnerForTesting = { args in
            if args == ["member", "list", "--json"] {
                return (Data(#"{"members":[],"claude_account":"team@example.com","you":"bob"}"#.utf8), nil)
            }
            return (Data(#"{"worktrees":[]}"#.utf8), nil)
        }
        var redraws = 0
        let watch = client.objectWillChange.sink { _ in redraws += 1 }
        #expect(client.claudeLoginViewer == nil)
        await client.talk.load(client)
        #expect(redraws > 0, "the window was not told the people arrived")
        #expect(client.claudeLoginViewer == "bob")
        watch.cancel()
    }
}
