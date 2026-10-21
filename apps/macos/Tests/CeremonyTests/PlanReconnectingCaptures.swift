import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the plan's overview with the runner
/// reconnecting over a plan in hand, and never read (ov-631), and the Agents
/// section's header saying what it counts (ov-660), at a narrow and a wide
/// canvas, light and dark, in a real titled window off every screen, taken
/// with `screencapture -l`. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct PlanReconnectingCaptures {
    typealias T = NativeAgentTests

    static func shoot(_ name: String, width: CGFloat, height: CGFloat, reconnecting: Bool, never: Bool = false, stale: Bool = false) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let board = try await PlanSectionViewTests.store(.busy)
        if reconnecting {
            // The daemon went away: the next read gets no answer, and the retry waits.
            board.plan.retryDelay = .seconds(600)
            board.plan.retryCeiling = .seconds(600)
            board.client.commandRunnerForTesting = { _ in (nil, "connection refused") }
            await board.plan.reload()
            #expect(board.plan.reconnecting)
        }
        if stale {
            // The runner stayed out of reach past the cap: the plan stays, with the line.
            board.plan.retryDelay = .seconds(600)
            board.plan.retryCeiling = .seconds(600)
            board.plan.giveUpAfter = .zero
            board.client.commandRunnerForTesting = { _ in (nil, "connection refused") }
            await board.plan.reload()
            #expect(board.plan.staleLine == PlanWords.staleUnreachable)
        }
        let plan = never ? try await Self.neverRead() : board.plan
        for (variant, appearance) in RealWindowCaptures.variants.prefix(2) {
            let view = ScrollView {
                PlanOverviewView(
                    plan: plan, statuses: board.board.statuses, selected: nil, needsYou: 1, onOpen: { _ in },
                    defaults: PlanViewTests.defaults())
                    .padding(.horizontal, 16)
            }
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: view.frame(width: width, height: height).background(WorkspaceStyle.canvas))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            await T.settle(window, 900)
            let image = try #require(RealWindowCaptures.windowImage(window), "screencapture -l isn't allowed here")
            try #require(image.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("mac-plan-\(name)-\(Int(width))-\(variant).png"))
            window.close()
        }
    }

    /// A board whose first read got no answer.
    static func neverRead() async throws -> PlanStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { _ in (nil, "connection refused") }
        let plan = PlanStore(
            client: client, workspace: .implicit(repository: "r"), host: "mini",
            defaults: PlanViewTests.defaults())
        plan.retryDelay = .seconds(600)
        plan.retryCeiling = .seconds(600)
        await plan.reload()
        return plan
    }

    @Test(arguments: [480.0, 900.0])
    func reconnectingOverAPlan(_ width: CGFloat) async throws {
        try await Self.shoot("reconnecting-plan", width: width, height: 900, reconnecting: true)
    }

    @Test(arguments: [480.0, 900.0])
    func staleOverAPlan(_ width: CGFloat) async throws {
        try await Self.shoot("stale-plan", width: width, height: 900, reconnecting: false, stale: true)
    }

    @Test(arguments: [480.0, 900.0])
    func reconnectingNeverRead(_ width: CGFloat) async throws {
        try await Self.shoot("reconnecting-new", width: width, height: 300, reconnecting: false, never: true)
    }

    /// The whole overview, tall, so the Agents section's header is in it.
    @Test(arguments: [480.0, 900.0])
    func agentsHeader(_ width: CGFloat) async throws {
        try await Self.shoot("agents", width: width, height: 2600, reconnecting: false)
    }
}
