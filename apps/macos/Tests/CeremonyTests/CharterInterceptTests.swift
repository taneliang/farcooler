import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The real window's Start Orchestrator, for a workspace with no charter, reads
/// the repository and starts nothing (ov-519 phase 2b, review S9); with a
/// charter there, it starts. Hosted: the screens are the window's `@State`,
/// which only a window on screen keeps.
@MainActor
@Suite(.serialized)
struct CharterInterceptTests {
    /// What the stubbed CLI was asked, in order.
    final class Asked { var calls: [[String]] = [] }

    static func client(charter: String, asked: Asked, prompt: String = "", orchestrator: Bool = false) -> DaemonClient {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            asked.calls.append(args)
            let json: String
            switch Array(args.prefix(2)) {
            case ["--json", "status"]:
                json = #"""
                    {"daemonVersion":"t","buildsMatch":true,"runtimeHealthy":true,
                     "capabilities":["workstreams","repo_survey","agents_found"],"agentsFound":["claude"],
                     "suggestThemesPrompt":"\#(prompt)"}
                    """#
            case ["repo", "list"]:
                json = #"{"repositories":[{"id":"repo-1","short":"repo-1","displayName":"harbor","remote":"","repositoryRootId":""}]}"#
            case ["worktree", "list"]:
                let terminals = orchestrator
                    ? #"[{"id":"t1","short":"t1","title":"","preset":"claude","state":"running","activity":"idle","epoch":0,"workspace":"ws-main","role":"orchestrator"}]"#
                    : "[]"
                let worktrees = #"""
                    [{"id":"w1","short":"w1","task":"main","branch":"main","repository":"harbor","repository_id":"repo-1",
                      "workspace":"ws-main","foreign_writers":[],"host":"","worktree":"/tmp/harbor","state":"active",
                      "terminals":\#(terminals)}]
                    """#
                json = #"""
                    {"runtime_healthy":true,"live_panes":0,"worktrees":\#(worktrees),
                     "workspaces":[{"id":"ws-main","short":"ws-main","repository":"repo-1","name":"Main","task_prefix":"hb",
                       "is_main":true,"ordinal":0,"orchestrator":\#(orchestrator ? "\"t1\"" : "null"),"home":null,
                       "charter":"\#(charter)"}]}
                    """#
            case ["repo", "survey"]:
                return (CharterSetupTests.survey(), nil)
            default:
                return (Data(), nil)
            }
            return (Data(json.utf8), nil)
        }
        return client
    }

    /// A window on Main, on a store whose one runner is `client`. The
    /// selection is the standard defaults' key, put back afterwards.
    /// Where each `identified` view was last drawn, in the window's
    /// SwiftUI coordinates (top-left origin).
    final class Drawn { var frames: [String: CGRect] = [:] }

    static func window(_ store: FleetStore, drawn: Drawn = Drawn()) async throws -> NSWindow {
        let root = ContentView(store: store)
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                GeometryReader { proxy in
                    let _ = drawn.frames = Dictionary(probed.map { ($0.id, proxy[$0.bounds]) }) { first, _ in first }
                    Color.clear
                }
                // The window's own coordinates, title bar included, as a click's.
                .ignoresSafeArea()
            }
        let window = try await TitleBarHarness.window(root, width: 1100, height: 760)
        try await Task.sleep(for: .milliseconds(600))
        try await TitleBarHarness.settle(window)
        return window
    }

    static func withMain<T>(_ body: () async throws -> T) async throws -> T {
        let defaults = UserDefaults.standard
        let touched = ["window.sessions.v1", SelectionMemory.destinationKey, SelectionMemory.key]
        let saved = Dictionary(uniqueKeysWithValues: touched.map { ($0, defaults.object(forKey: $0)) })
        defer { RealWindowCaptures.restore(saved, in: defaults) }
        defaults.removeObject(forKey: "window.sessions.v1")
        defaults.removeObject(forKey: SelectionMemory.destinationKey)
        defaults.set("|ws-main|", forKey: SelectionMemory.key)
        return try await body()
    }

    static func until(_ timeout: Duration = .seconds(30), _ done: () -> Bool) async {
        let clock = ContinuousClock(), end = clock.now + timeout
        while !done(), clock.now < end { try? await Task.sleep(for: .milliseconds(50)) }
    }

    static func ran(_ asked: Asked, _ prefix: [String]) -> Bool {
        asked.calls.contains { $0.starts(with: prefix) }
    }

    /// The workspace as the window's store lists it, read off the stub.
    static func main(_ store: FleetStore) -> WorkspaceSummary? {
        store.fleet.runnerWorkspaces[""]?.first { $0.id == "ws-main" }
    }

    static func ready(charter: String, asked: Asked) async -> FleetStore {
        let client = Self.client(charter: charter, asked: asked)
        let store = FleetStore(clients: ["": client])
        await client.recheckInstalled()
        await client.refreshRepositories()
        await client.refresh()
        await Self.until { Self.main(store) != nil }
        return store
    }

    @Test("Start Orchestrator from the column's menu, with no charter, starts nothing; with one, it starts")
    func intercept() async throws {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("ov519-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: home) }
        let charter = home.appendingPathComponent("charter.md").path
        let asked = Asked()
        let store = await Self.ready(charter: charter, asked: asked)
        let main = try #require(Self.main(store))
        // The menu's own action: the invitation every empty state draws
        // Start Orchestrator from.
        let view = ContentView(store: store)
        let invite = try #require(view.orchestratorInvite(host: "", workspace: main))
        invite.onStart(.claude)
        try await Task.sleep(for: .milliseconds(800))
        #expect(!Self.ran(asked, ["workspace", "start-orchestrator"]), "started past the charter: \(asked.calls)")

        FileManager.default.createFile(atPath: charter, contents: Data("# charter\n".utf8))
        invite.onStart(.claude)
        await Self.until { Self.ran(asked, ["workspace", "start-orchestrator"]) }
        #expect(Self.ran(asked, ["workspace", "start-orchestrator", "ws-main", "--harness", "claude"]), "\(asked.calls)")
    }

    @Test("In a window, a draft left from Talk It Through brings the screens back, and they read the repository")
    func resumeInAWindow() async throws {
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("ov519-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: home) }
        FileManager.default.createFile(atPath: home.appendingPathComponent(CharterNeed.draftFile).path, contents: Data("# draft\n".utf8))
        let asked = Asked()
        let store = await Self.ready(charter: home.appendingPathComponent("charter.md").path, asked: asked)
        try await Self.withMain {
            let drawn = Drawn()
            let window = try await Self.window(store, drawn: drawn)
            defer { window.close() }
            await Self.until { drawn.frames["charter-resume"] != nil }
            #expect(Self.ran(asked, ["repo", "survey", "--repo", "repo-1"]), "\(asked.calls)")
            #expect(drawn.frames["charter-resume-button"] != nil, "\(drawn.frames.keys.sorted())")
            #expect(!Self.ran(asked, ["workspace", "start-orchestrator"]))
        }
    }

    @Test("Suggest Themes is offered only with a live orchestrator and a runner that has the prompt, and types it in")
    func suggestThemes() async throws {
        let workspace = WorkspaceSummary(
            id: "ws-main", name: "Main", taskPrefix: "hb", isMain: true, ordinal: 0, repository: "repo-1", orchestrator: "t1")
        for (prompt, live, offered) in [("", true, false), ("Suggest themes.", false, false), ("Suggest themes.", true, true)] {
            let asked = Asked()
            let client = Self.client(charter: "/nonexistent/charter.md", asked: asked, prompt: prompt, orchestrator: live)
            client.typingPasses = 60
            let store = FleetStore(clients: ["": client])
            await client.recheckInstalled()
            await client.refresh()
            await Self.until { store.fleet.runnerWorkspaces[""]?.isEmpty == false }
            let view = ContentView(store: store)
            let suggest = view.suggestThemes(host: "", workspace: workspace, canAct: true)
            #expect((suggest != nil) == offered, "prompt \(prompt.isEmpty ? "empty" : "set"), live \(live)")
            guard let suggest else { continue }
            suggest.send()
            await Self.until { Self.ran(asked, ["terminal", "send"]) }
            #expect(asked.calls.contains(["terminal", "send", "t1", "Suggest themes."]), "\(asked.calls)")
        }
    }
}
