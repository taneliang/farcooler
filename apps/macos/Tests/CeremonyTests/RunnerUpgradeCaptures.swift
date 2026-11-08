import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the recovery screen (ov-984) in a real
/// titled window at the narrowest the main window goes
/// (`MainWindowChrome.minimumWidth`), light and dark. Before: the button.
/// During: a real run held at Verify, so the rows are the model's own. A
/// failure at Verify, with Recover. After: Upgrade Finished.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct RunnerUpgradeCaptures {
    /// A runner that answers at once, except that `holding` waits until let go.
    final class Runner: @unchecked Sendable {
        let lock = NSLock()
        var holding: String?
        var refusing: String?
        var inspected = RunnerUpgradeTests.legacy

        func run(_ args: [String]) async -> CLI.Result {
            let op = args[2]
            while lock.withLock({ holding == op }) { try? await Task.sleep(for: .milliseconds(20)) }
            if op == "inspect" {
                return CLI.Result(ok: true, output: lock.withLock { inspected }, errors: "")
            }
            if lock.withLock({ refusing == op }) {
                return CLI.Result(
                    ok: false, output: "",
                    errors: "Error: That upgrade step was refused. Run `farcooler workflow-cutover inspect` to see "
                        + "where the upgrade stands.\ncode: operation-failed")
            }
            return CLI.Result(ok: true, output: "{\"schema\":1}", errors: "")
        }

        var driver: RunnerUpgrade.Driver {
            RunnerUpgrade.Driver(run: { await self.run($0) }, confirm: { _ in .yes }, attempt: { "mac-capture" })
        }
    }

    static func shoot(_ name: String, _ upgrade: RunnerUpgrade, until reached: @escaping @MainActor () -> Bool) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let width = MainWindowChrome.minimumWidth
        let height: CGFloat = 560
        for (variant, appearance) in RealWindowCaptures.variants.prefix(2) {
            let root = FleetPlaceholder(
                phase: .upgrade, welcome: WelcomeSetup(
                    model: WelcomeModel(home: "/nowhere", scan: { [] }), step: .ready([.claude]),
                    agents: HarnessAvailability(agentsFound: ["claude"]), register: { _ in nil },
                    onCheckAgain: {}, onAnotherRunner: nil), onNewWorktree: {}, onTryAgain: {},
                upgrade: upgrade
            )
            .frame(width: width, height: height)
            let controller = NSHostingController(rootView: root)
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height),
                styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered,
                defer: false)
            window.isReleasedWhenClosed = false
            window.title = "Far Cooler"
            window.contentViewController = controller
            window.setContentSize(NSSize(width: width, height: height))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            try await Capture.keep(
                window, to: URL(fileURLWithPath: out).appendingPathComponent("ov984-\(name)-\(variant).png"),
                expecting: name, until: reached)
            window.close()
        }
    }

    @Test func beforeDuringFailedAndAfter() async throws {
        let runner = Runner()
        let upgrade = RunnerUpgrade(driver: runner.driver)
        try await Self.shoot("1-before", upgrade) { upgrade.phase == .ready(notice: nil) }

        runner.holding = "verify"
        let run = Task { await upgrade.finish() }
        try await Self.shoot("2-during", upgrade) { upgrade.phase == .running(.verify) }
        runner.lock.withLock { runner.refusing = "verify"; runner.holding = nil }
        await run.value
        try await Self.shoot("3-failed", upgrade) {
            if case .failed(.step(.verify), _) = upgrade.phase { return true }
            return false
        }

        // Committed by then, so Recover goes on through the steps left.
        runner.lock.withLock { runner.refusing = nil; runner.inspected = RunnerUpgradeTests.at("committed") }
        await upgrade.recover()
        try await Self.shoot("4-after", upgrade) { upgrade.phase == .done }
    }
}
