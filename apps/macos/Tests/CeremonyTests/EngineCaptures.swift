import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

@MainActor
@Suite(.serialized)
struct EngineCaptures {
    typealias Window = WorktreeTerminalNavigationTests
    typealias Variant = (name: String, width: CGFloat, height: CGFloat, type: DynamicTypeSize, appearance: NSAppearance.Name)

    @Test("Plan opens Engine and selecting a station ticket shows the inspector")
    func stationsAndInspector() async throws {
        let out = ProcessInfo.processInfo.environment["FARCOOLER_ENGINE_CAPTURE_OUT"].map { URL(fileURLWithPath: $0) }
        if let out { try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true) }
        let variants: [Variant] = [
            ("wide", 1440, 1040, .large, .darkAqua),
            ("narrow", 420, 1060, .large, .aqua),
            ("largest-text", 420, 2300, .accessibility5, .accessibilityHighContrastDarkAqua),
        ]
        for variant in out == nil ? Array(variants.prefix(1)) : variants {
            try await check(variant, out: out)
        }
    }

    private func check(_ variant: Variant, out: URL?) async throws {
        let board = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults())
        let data = try Self.snapshot()
        let requests = PlanViewTests.Calls()
        board.client.commandRunnerForTesting = { args in
            requests.args.append(args)
            return (data, nil)
        }
        let (window, seen) = try await Window.show(
            Hosted(board: board)
                .environment(\.dynamicTypeSize, variant.type)
                .background { WindowPlane() },
            size: CGSize(width: variant.width, height: variant.height))
        defer { window.close() }
        window.appearance = NSAppearance(named: variant.appearance)
        try await Window.click("plan-open-engine", in: window, seen)
        let row = "engine-ticket-ov-1321"
        try await Capture.wait(expecting: "the ticket in the Build station") { seen.views[row] != nil }
        if variant.type == .accessibility5 {
            #expect(try #require(seen.views[row]).height > 200)
        }
        #expect(requests.args.contains(["plan", "engine", "--repo", "r", "--json"]))
        let host = try #require(window.contentView)
        if let out {
            try await Capture.keep(
                host, to: out.appendingPathComponent("stations-\(variant.name).png"), expecting: "the Build station"
            ) { seen.views[row] != nil }
        }
        try await Window.click(row, in: window, seen)
        try await Capture.wait(expecting: "the selected ticket’s inspector") { seen.views["engine-inspector"] != nil }
        if let out {
            try await Capture.keep(
                host, to: out.appendingPathComponent("inspector-\(variant.name).png"), expecting: "the run inspector"
            ) { seen.views["engine-inspector"] != nil }
        }
    }

    private struct Hosted: View {
        let board: TaskBoardStore
        @State var page: PlanPage?

        var body: some View {
            if case .engine(let focus) = page {
                EnginePage(client: board.client, workspace: board.plan.workspace, focus: focus, visible: true, openTicket: { _ in })
            } else {
                PlanHome(board: board, needsYou: .init(), onOpen: { page = $0 })
            }
        }
    }

    /// Representative Engine read with one long diagnostic and recent landing evidence.
    private static func snapshot() throws -> Data {
        let failure = "The agent’s session ended while the runner was restarting. Its work is still in the worktree for the next attempt. The session ended before it wrote a result."
        let longTitle = "Gate: a script change runs only its users when a shared file changes, so unrelated native tests can keep their place in the queue"
        let budget: [String: Any] = ["remaining": 2, "limit": 3, "history": [[
            "id": "back-1", "from": "Review", "to": "Build",
            "reason": "The review found that changes to shared scripts bypass the dependency check. Cover the shared-file path before sending this back for review."
        ]]]
        let failed: [String: Any] = ["id": "ov-1321", "title": longTitle, "step": "Build",
            "status": "Failed", "tone": "failed", "summary": failure, "failure": failure,
            "agent": "codex:gpt-6.1-sol", "review": "Sent Back from Review", "sendBacks": budget]
        let running: [String: Any] = ["id": "ov-1359", "title": "Tests: native tests stay steady when descendants stop",
            "step": "Build", "status": "Building", "tone": "normal", "review": "Review State Unavailable",
            "sendBacks": ["remaining": 3, "limit": 3, "history": []]]
        let todo: [String: Any] = ["id": "ov-1168", "title": "Engine: a failure says why in the workflow",
            "step": "To Do", "status": "Awaiting Approval", "tone": "waiting",
            "summary": "Waiting for approval from orchestrator.", "review": "No Review Step"]
        let wire: [String: Any] = [
            "workspace": "r", "epoch": 1, "revision": 208,
            "capacity": [
                ["id": "agent", "label": "Agent Slots", "used": 4, "limit": 4, "queued": 2],
                ["id": "landing", "label": "Landing Slots", "used": 0, "limit": 2, "queued": 0],
                ["id": "load", "label": "1-Minute Load"], ["id": "disk", "label": "Disk Free"],
            ],
            "stations": [
                ["id": "backlog", "label": "Backlog", "tickets": []],
                ["id": "todo", "label": "To Do", "tickets": [todo]],
                ["id": "build", "label": "Build", "tickets": [failed, running]],
                ["id": "review", "label": "Review", "tickets": []],
                ["id": "join", "label": "Join", "tickets": []],
            ],
            "landings": [[
                "id": "group-4320-a", "label": "Landing Group 4320", "status": "Local Gate · Failed", "tone": "failed",
                "sinceMs": 1_795_105_260_000, "historical": true, "tickets": [],
                "members": [["id": "ov-1352", "title": "Engine: free landing trees after recovery"]],
                "failure": "Disk admission refused. 60.61 GB free, 61.00 GB needed.",
            ]],
            "agents": [[
                "id": "effect-1359", "role": "Build", "ticket": "ov-1359", "model": "codex:gpt-6.1-sol",
                "status": "Building", "tone": "normal",
            ]],
        ]
        return try JSONSerialization.data(withJSONObject: wire)
    }
}
