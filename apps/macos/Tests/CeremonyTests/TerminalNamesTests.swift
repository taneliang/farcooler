import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Terminals for projects and reviews (ov-234, from ov-190's "just
/// terminals"): a name that shows everywhere, the port a terminal serves with
/// Open in Browser, a task's own terminals with Close, ↑ and ↓ onto the
/// project's, and a breadcrumb that names a project terminal in any
/// workspace.
@MainActor
struct TerminalNamesTests {
    private typealias Selection = ContentView.Selection

    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let board = "ws-bil"

    private static func terminal(
        _ id: String, preset: String = "zsh", state: String = "running", role: String? = "shell",
        task: String? = nil, ports: [Int]? = nil, mode: String? = nil
    ) -> Terminal {
        var t = Terminal(id: id, short: id, title: id, preset: preset, state: state, epoch: 0)
        t.role = role
        t.taskId = task
        t.ports = ports
        t.paneMode = mode
        return t
    }

    private static func worktree(
        _ id: String, terminals: [Terminal], main: Bool = false, host: String = ""
    ) -> Worktree {
        var w = Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: "shop", host: host, path: "/tmp/\(id)",
            state: "active", terminals: terminals, repositoryID: repo, workspace: nil)
        w.is_main_checkout = main
        return w
    }

    private static func fleet(_ worktrees: [Worktree]) -> Fleet {
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: worktrees, branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [
            WorkspaceSummary(id: "ws-main", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: repo),
            WorkspaceSummary(id: board, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo),
        ]
        return fleet
    }

    // MARK: - Ports

    /// The port is a field, lowest first, so a dev server that also opens a
    /// debugger port reads as the server a person started. A runner too old
    /// to send it says nothing.
    @Test("A terminal's port is its lowest, as :5173, and absent from a runner that doesn't say")
    func theLowestPortIsTheLabel() {
        #expect(Self.terminal("dev", ports: [9229, 5173]).servedPorts == [5173, 9229])
        #expect(Self.terminal("dev", ports: [9229, 5173]).portLabel == ":5173")
        #expect(Self.terminal("dev", ports: []).portLabel == nil)
        #expect(Self.terminal("dev", ports: nil).portLabel == nil)
    }

    /// It decodes from the CLI's JSON key, and an older CLI's missing key is
    /// no ports rather than a failure.
    @Test("ports decodes from worktree list, and is absent without the key")
    func portsDecode() throws {
        let base = #"{"id":"a","short":"a","title":"x","preset":"zsh","state":"running","epoch":0"#
        let with = try JSONDecoder().decode(Terminal.self, from: Data((base + #","ports":[5173,9229]}"#).utf8))
        #expect(with.servedPorts == [5173, 9229])
        let without = try JSONDecoder().decode(Terminal.self, from: Data((base + "}").utf8))
        #expect(without.ports == nil && without.portLabel == nil)
    }

    /// Open in Browser goes to localhost on the lowest port, for this Mac's
    /// own runner only: a remote runner's port isn't forwarded, so localhost
    /// would open something else.
    @Test("Open in Browser opens localhost on the lowest port, only on this Mac")
    func openInBrowserIsForThisMacOnly() {
        let dev = Self.terminal("dev", ports: [9229, 5173])
        #expect(TerminalPorts.browserURL(for: dev, host: "")?.absoluteString == "http://localhost:5173")
        #expect(TerminalPorts.browserURL(for: dev, host: "build-box") == nil)
        #expect(TerminalPorts.browserURL(for: Self.terminal("shell"), host: "") == nil)
    }

    // MARK: - A task's terminals

    private static func taskRow(_ id: String = "t3", worktree: String?) -> TaskRow {
        TaskRow(id: id, key: "bil-3", title: "Export", status: .inProgress, statusSince: .now, worktreeID: worktree)
    }

    /// Its terminals are the ones in its worktree that are the person's: not
    /// its agent, an orchestrator or a changes pane, and not another
    /// worktree's. A Done task's server is still listed: nothing closes it.
    @Test("A task's terminals are those running in its worktree, never its agent")
    func aTasksTerminalsAreThoseInItsWorktree() {
        let lane = Self.worktree(
            "tax",
            terminals: [
                Self.terminal("agent", preset: "claude", role: "agent", task: "t3"),
                Self.terminal("dev", ports: [5173]),
                Self.terminal("diff", preset: "farcooler", role: nil, mode: "changes"),
                Self.terminal("conductor", preset: "claude", role: "orchestrator"),
                Self.terminal("tests", state: "exited"),
            ])
        let elsewhere = Self.worktree("pdf", terminals: [Self.terminal("other-dev", ports: [3000])])
        let fleet = Self.fleet([elsewhere, lane])
        #expect(TaskTerminals.terminals(of: Self.taskRow(worktree: "tax"), host: "", in: fleet).map(\.id) == ["dev", "tests"])
        #expect(TaskTerminals.terminals(of: Self.taskRow(worktree: "pdf"), host: "", in: fleet).map(\.id) == ["other-dev"])
        // A task with no worktree of its own is where its agent is, as the
        // task view draws it; with neither, there's nothing to list.
        #expect(TaskTerminals.terminals(of: Self.taskRow(worktree: nil), host: "", in: fleet).map(\.id) == ["dev", "tests"])
        #expect(TaskTerminals.terminals(of: Self.taskRow("t77", worktree: nil), host: "", in: fleet).isEmpty)
        #expect(TaskTerminals.terminals(of: Self.taskRow("t77", worktree: "gone"), host: "", in: fleet).isEmpty)
    }

    // MARK: - Names

    @Test("The rename field opens on the name a person gave, and empty on an automatic one")
    func theFieldOpensOnTheGivenName() {
        var named = Self.terminal("proxy")
        named.title = "gcp proxy"
        #expect(TerminalName.initial(named) == "gcp proxy")
        var automatic = Self.terminal("shell")
        automatic.title = "Terminal 3"
        #expect(TerminalName.initial(automatic) == "")
    }

    @Test("A name the runner would refuse can't be confirmed; an empty one clears the name")
    func theSheetRefusesWhatTheRunnerWould() {
        #expect(TerminalName.isValid("gcp proxy"))
        #expect(TerminalName.isValid("   "))
        #expect(TerminalName.isValid(String(repeating: "x", count: TerminalName.limit)))
        #expect(!TerminalName.isValid(String(repeating: "x", count: TerminalName.limit + 1)))
        #expect(!TerminalName.isValid("a\nb"))
    }

    /// A name shows wherever the terminal's label does: the stored title,
    /// when it isn't the automatic one. The runner keeps it, so it comes back
    /// on every list and after a restart.
    @Test("A renamed terminal's name is its label, everywhere the label shows")
    func aRenamedTerminalIsLabeledByItsName() {
        var shell = Self.terminal("shell")
        shell.title = "gcp proxy"
        #expect(shell.label == "gcp proxy")
        #expect(shell.displayName(ordinal: 2) == "gcp proxy")
        shell.title = "Terminal"
        #expect(shell.label == "shell")
    }

    // MARK: - ↑ and ↓

    /// The project's terminals are rows of the navigator, between the tasks
    /// and the worktrees, so ↓ from the last task reaches the first one and
    /// ↑ from the first worktree reaches the last.
    @Test("↑ and ↓ walk the project's terminals, between the tasks and the worktrees")
    func arrowsReachProjectTerminals() {
        let items = Navigator.items(
            orchestrator: true, tasks: ["t3", "t9"], terminals: ["proxy", "tail"], worktrees: ["scratch"])
        #expect(
            items == [.orchestrator, .task("t3"), .task("t9"), .terminal("proxy"), .terminal("tail"), .worktree("scratch")])
        #expect(Navigator.step(from: .task("t9"), by: 1, in: items) == .terminal("proxy"))
        #expect(Navigator.step(from: .terminal("tail"), by: 1, in: items) == .worktree("scratch"))
        #expect(Navigator.step(from: .worktree("scratch"), by: -1, in: items) == .terminal("tail"))
        // With none, the walk is as it was.
        #expect(
            Navigator.items(orchestrator: false, tasks: ["t3"], worktrees: ["scratch"])
                == [.task("t3"), .worktree("scratch")])
        // A terminal row opens no task.
        #expect(NavigatorItem.terminal("proxy").taskID == nil)
    }

    /// A project terminal open lights its own row, in the workspace it was
    /// opened from; the checkout's worktree row isn't lit for it. A terminal
    /// in a worktree that isn't one of the project's still lights the worktree.
    @Test("A project terminal open lights its own row, not the checkout's")
    func aProjectTerminalLightsItsRow() {
        let open = Selection.workspace(host: "", workspace: Self.board, focus: .worktree("checkout", terminal: "proxy"))
        #expect(Navigator.current(open, trail: nil, board: Self.board, terminals: ["proxy"]) == .terminal("proxy"))
        #expect(Navigator.current(open, trail: nil, board: Self.board, terminals: []) == .worktree("checkout"))
        #expect(Navigator.current(open, trail: nil, board: "ws-other", terminals: ["proxy"]) == nil)
        let whole = Selection.workspace(host: "", workspace: Self.board, focus: .worktree("checkout", terminal: nil))
        #expect(Navigator.current(whole, trail: nil, board: Self.board, terminals: ["proxy"]) == .worktree("checkout"))
    }

    // MARK: - The breadcrumb

    /// Opened from a workspace that doesn't own the main checkout, a project
    /// terminal reads Workspace › Terminal, not the checkout's worktree with a
    /// menu of the workspace's own worktrees, none of them checked. Any other
    /// worktree's terminal keeps its worktree crumb.
    @Test("A project terminal's breadcrumb is the workspace and its name, in a workspace that doesn't own the checkout")
    func aProjectTerminalsBreadcrumbNamesTheTerminal() {
        var proxy = Self.terminal("proxy")
        proxy.title = "gcp proxy"
        let checkout = Self.worktree("checkout", terminals: [proxy, Self.terminal("agent", role: "agent", task: "t3")], main: true)
        let lane = Self.worktree("tax", terminals: [Self.terminal("s1")])
        let fleet = Self.fleet([checkout, lane])
        func crumbs(_ selection: Selection) -> [String] {
            WorkspaceNavigation.crumbs(
                selection, trail: nil, workspace: "Billing", task: { $0 }, worktree: { $0 },
                projectTerminal: { id, terminal in
                    ProjectTerminals.name(of: terminal, in: fleet.worktrees.first { $0.id == id }, fleet: fleet)
                }
            ).map(\.title)
        }
        func open(_ worktree: String, _ terminal: String?) -> Selection {
            .workspace(host: "", workspace: Self.board, focus: .worktree(worktree, terminal: terminal))
        }
        #expect(crumbs(open("checkout", "proxy")) == ["Billing", "gcp proxy"])
        // A task's agent in the checkout, the checkout whole, and a lane's
        // terminal are not project terminals.
        #expect(crumbs(open("checkout", "agent")) == ["Billing", "checkout"])
        #expect(crumbs(open("checkout", nil)) == ["Billing", "checkout"])
        #expect(crumbs(open("tax", "s1")) == ["Billing", "tax"])
    }
}
