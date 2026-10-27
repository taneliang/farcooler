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
    ///
    /// `until` names a probe the caller expects: drawing goes on, 20 ms at a
    /// time, until it shows (or a minute passes), so a loaded runner that is
    /// slow to lay the page out is waited for rather than judged.
    static func drawn(_ plan: PlanStore, page: PlanPage, until expected: String? = nil) async -> Set<String> {
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
        var laps = 0
        while laps < 15 || (expected.map { !seen.views.keys.contains($0) } == true && laps < 3_000) {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
            laps += 1
        }
        return Set(seen.views.keys)
    }

    @Test("A page on a runner that gives no answer says it's reconnecting, not a spinner and not a broken plan")
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
            defaults: ScratchDefaults.suite("p6-\(UUID().uuidString)")!)
        let ids = await Self.drawn(plan, page: .theme("00000000-0000-0000-0000-000000003001"))
        #expect(calls.args.contains { $0.first == "plan" }, "the page asked for the plan")
        #expect(ids.contains("plan-reconnecting"), "drew \(ids)")
        #expect(!ids.contains("plan-page-unavailable"), "no answer isn't an unreadable plan (ov-631)")
        #expect(!ids.contains("plan-page-reading"), "no spinner once the read has failed")
        #expect(plan.trouble == nil, "the app's sentence, never the CLI's stderr")
        #expect(plan.reconnecting)
    }

    @Test("A page whose plan read hasn't come back yet shows a spinner")
    func reading() async throws {
        // The read is held until the test lets it go. It used to sleep 30 s
        // and then fail, and a runner too loaded to inspect the page inside
        // those 30 s saw the failed read's state instead of the spinner
        // (run 40477375974: 40.7 s).
        let held = HeldRead()
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in await held.answer() }
        let plan = PlanStore(
            client: client, workspace: .implicit(repository: "r"), host: "mini",
            defaults: ScratchDefaults.suite("p6-\(UUID().uuidString)")!)
        let ids = await Self.drawn(
            plan, page: .lane("00000000-0000-0000-0000-000000002002"), until: "plan-page-reading")
        #expect(ids.contains("plan-page-reading"), "drew \(ids)")
        #expect(!ids.contains("plan-page-unavailable"))
        held.release()
    }

    /// A runner that has not answered, and does not until `release()`.
    @MainActor final class HeldRead {
        private var waiting: [CheckedContinuation<(data: Data?, message: String?), Never>] = []
        private var released = false
        func answer() async -> (data: Data?, message: String?) {
            if released { return (nil, "released") }
            return await withCheckedContinuation { waiting.append($0) }
        }
        func release() {
            released = true
            for each in waiting { each.resume(returning: (nil, "released")) }
            waiting = []
        }
    }
}
