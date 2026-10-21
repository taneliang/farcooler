import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A plan that says when it is stale (ov-662): a missing CLI is said at once
/// rather than as "Reconnecting…", and a plan already on screen carries a
/// quiet line with Try Again once the runner has stayed out of reach.
@MainActor
@Suite(.serialized)
struct PlanStaleTests {
    @Test("A missing CLI says it isn't installed at once, not that the runner is reconnecting")
    func missingCLI() async throws {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.binaryForTesting = ""
        client.startEvents()
        #expect(client.isCLIMissing)
        client.commandRunnerForTesting = { _ in (nil, "no tool") }
        let plan = PlanStore(
            client: client, workspace: .implicit(repository: "r"), host: "mini",
            defaults: ScratchDefaults.suite("stale-\(UUID().uuidString)")!)
        plan.retryDelay = .seconds(600)
        plan.retryCeiling = .seconds(600)
        await plan.reload()
        #expect(plan.trouble == PlanWords.notInstalled)
        #expect(!plan.reconnecting, "waiting would not install it")
    }

    @Test("After the cap, a plan in hand carries a notice, and Try Again clears it when the runner answers")
    func staleAfterTheCap() async throws {
        let runner = PlanReconnectingTests.Runner()
        runner.answer = try PlanViewTests.fixture()
        let plan = PlanReconnectingTests.store(runner)
        plan.giveUpAfter = .milliseconds(100)
        await plan.reload()
        #expect(plan.hasRead && plan.staleLine == nil)
        runner.answer = nil
        await plan.reload()
        #expect(plan.reconnecting && plan.staleLine == nil, "still within the wait")
        await PlanReconnectingTests.eventually("the cap shows the notice") { plan.staleLine == PlanWords.staleUnreachable }
        #expect(plan.hasRead && !plan.plan.themes.isEmpty, "the last plan stays")
        runner.answer = try PlanViewTests.fixture()
        await plan.tryAgain()
        #expect(plan.staleLine == nil && plan.trouble == nil)
    }

    @Test("A plan in hand on a runner with no CLI says so over the plan")
    func staleNotInstalled() async throws {
        let runner = PlanReconnectingTests.Runner()
        runner.answer = try PlanViewTests.fixture()
        let plan = PlanReconnectingTests.store(runner)
        await plan.reload()
        runner.answer = nil
        plan.cliIsMissing = { true }
        await plan.reload()
        #expect(plan.staleLine == PlanWords.staleNotInstalled)
        #expect(!plan.reconnecting)
    }
}
