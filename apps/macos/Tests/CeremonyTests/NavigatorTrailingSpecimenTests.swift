import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The navigator with terminals and worktrees (ov-257), drawn offscreen in
/// both appearances so the trailing column can be looked at: the status rings
/// and the counts should share one right edge. A rendering written where
/// `FARCOOLER_GLANCE_OUT` says, driven through the model, with no input sent.
@MainActor
struct NavigatorTrailingSpecimenTests {
    /// The board column as the owner's screenshot had it: terminals (one with
    /// a port, one working, one waiting) and a worktree that wants attention.
    static func board(_ store: TaskBoardStore) -> TaskBoardView {
        func terminal(_ id: String, _ title: String, activity: String?, ports: [Int]? = nil) -> Terminal {
            var t = Terminal(id: id, short: id, title: title, preset: "zsh", state: "running", epoch: 0)
            t.activity = activity
            t.ports = ports
            return t
        }
        var terminals = ProjectTerminals(
            terminals: [
                terminal("t1", "proxy", activity: nil, ports: [5173]),
                terminal("t2", "build", activity: "working"),
                terminal("t3", "claude", activity: "blocked"),
            ],
            selected: "t2")
        terminals.onNew = {}
        var waiting = terminal("w1t", "agent", activity: "blocked")
        waiting.activity = "blocked"
        let loose = Worktree(
            id: "w1", short: "w1", task: "ov-9", branch: "ov-9", repository: "r", host: "", path: "/tmp/w1",
            state: "active", terminals: [waiting], repositoryID: "r", workspace: nil)
        let quiet = Worktree(
            id: "w2", short: "w2", task: "ov-12", branch: "ov-12", repository: "r", host: "", path: "/tmp/w2",
            state: "active", terminals: [terminal("w2t", "shell", activity: "working")], repositoryID: "r", workspace: nil)
        var worktrees = BoardWorktrees(shown: [loose, quiet], selected: "w1", terminals: terminals)
        worktrees.onNew = {}
        return TaskBoardView(
            store: store, client: store.client, agents: .none, onGoTo: { _ in },
            defaults: UserDefaults(suiteName: "trailing-\(UUID().uuidString)")!,
            worktrees: { _ in worktrees }, orchestrator: GridGeometryTests.orchestrator(running: true))
    }

    @Test("Write the navigator's trailing-column sheets")
    func writeSheets() async throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let store = await GridGeometryTests.store()
        for dark in [false, true] {
            let width = WorkspaceColumns.navigatorDefault
            let height: CGFloat = 900
            let host = NSHostingView(
                rootView: Self.board(store).frame(width: width, height: height, alignment: .topLeading)
                    .background(dark ? Color(white: 0.12) : Color.white))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: width, height: height), styleMask: [.borderless],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.appearance = host.appearance
            window.contentView = host
            defer { window.close() }
            await NavigatorRhythmTests.settle(host) { "\(host.fittingSize)" }
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("navigator-\(dark ? "dark" : "light").png"))
        }
    }
}
