import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A theme's or lane's page on a runner that can't answer (ov-273, review
/// 1004i P6): the page restored at launch before anything was read, or on a
/// runner that has gone. Once the read fails it shows the unavailable state,
/// never a spinner forever.
@MainActor
@Suite(.serialized)
struct PlanPageUnavailableTests {
    /// What the page drew, by probe id.
    static func drawn(_ plan: PlanStore, page: PlanPage) async -> Set<String> {
        let seen = NavigatorFilterTests.Seen()
        let context = PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })
        let host = NSHostingView(
            rootView: PlanPageView(plan: plan, page: page, context: context)
                .frame(width: 600, height: 500)
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    let _ = seen.views = Dictionary(probed.map { ($0.id, .zero) }, uniquingKeysWith: { a, _ in a })
                    Color.clear
                })
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 600, height: 500), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.close() }
        for _ in 0..<15 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return Set(seen.views.keys)
    }

    @Test("A page on a runner whose plan read fails shows the unavailable state, not a spinner")
    func failedRead() async throws {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let calls = PlanViewTests.Calls()
        // The CLI giving up on the runner, as it does after its timeout.
        client.commandRunnerForTesting = { args in
            calls.args.append(args)
            return (nil, "ssh: connect to host mini port 22: Operation timed out")
        }
        let plan = PlanStore(
            client: client, workspace: .implicit(repository: "r"), host: "mini",
            defaults: UserDefaults(suiteName: "p6-\(UUID().uuidString)")!)
        let ids = await Self.drawn(plan, page: .theme("00000000-0000-0000-0000-000000003001"))
        #expect(calls.args.contains { $0.first == "plan" }, "the page asked for the plan")
        #expect(ids.contains("plan-page-unavailable"), "drew \(ids)")
        #expect(!ids.contains("plan-page-reading"), "no spinner once the read has failed")
        #expect(plan.trouble == PlanWords.couldntRead, "the app's sentence, never the CLI's stderr")
    }

    @Test("A page whose plan read hasn't come back yet shows a spinner")
    func reading() async throws {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in
            try? await Task.sleep(for: .seconds(30))
            return (nil, "cancelled")
        }
        let plan = PlanStore(
            client: client, workspace: .implicit(repository: "r"), host: "mini",
            defaults: UserDefaults(suiteName: "p6-\(UUID().uuidString)")!)
        let ids = await Self.drawn(plan, page: .lane("00000000-0000-0000-0000-000000002002"))
        #expect(ids.contains("plan-page-reading"), "drew \(ids)")
        #expect(!ids.contains("plan-page-unavailable"))
    }
}
