import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A runner that is restarting is not a broken plan (ov-631). The owner saw
/// "couldn't read this board's plan" right after an update restarted the
/// daemon, and it cleared on its own later.
@MainActor
@Suite(.serialized)
struct PlanReconnectingTests {
    final class Runner {
        var answer: Data?
        var asked = 0
        /// How long a read takes, so a test can close the view while one is out.
        var delay: Duration = .zero
    }

    static func store(_ runner: Runner) -> PlanStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            if args.first == "plan" {
                runner.asked += 1
                if runner.delay > .zero { try? await Task.sleep(for: runner.delay) }
                return (runner.answer, runner.answer == nil ? "connection refused" : nil)
            }
            return (Data(), nil)
        }
        let plan = PlanStore(
            client: client, workspace: .implicit(repository: "r"), host: "mini",
            defaults: ScratchDefaults.suite("reconnecting-\(UUID().uuidString)")!)
        plan.retryDelay = .milliseconds(10)
        plan.retryCeiling = .milliseconds(40)
        plan.linkIsDown = { true }
        return plan
    }

    static func eventually(_ what: String, _ done: () -> Bool) async {
        for _ in 0..<200 where !done() { try? await Task.sleep(for: .milliseconds(10)) }
        #expect(done(), "never: \(what)")
    }

    @Test("A daemon restart shows reconnecting with the last plan, then the fresh plan, and never couldn't read")
    func restart() async throws {
        let runner = Runner()
        runner.answer = try PlanViewTests.fixture()
        let plan = Self.store(runner)
        await plan.reload()
        let before = plan.plan
        #expect(!before.themes.isEmpty)
        #expect(plan.hasRead && !plan.reconnecting)

        // The daemon goes down for the update: the read gets no answer.
        runner.answer = nil
        await plan.reload()
        #expect(plan.reconnecting)
        #expect(plan.trouble == nil, "not \"couldn’t read\"")
        #expect(plan.plan == before, "the last plan stays")
        #expect(plan.hasRead)

        // It comes back with a fresh plan, and nobody asks for another read.
        runner.answer = try PlanViewTests.emptyPlan()
        await Self.eventually("the retry reads the fresh plan") { !plan.reconnecting && plan.plan.themes.isEmpty }
        #expect(plan.trouble == nil)
    }

    @Test("A command that fails with the link up is a plan that couldn't be read, not a runner reconnecting")
    func aFailedCommandIsNotReconnecting() async throws {
        let runner = Runner()
        let plan = Self.store(runner)
        plan.linkIsDown = { false }
        await plan.reload()
        #expect(plan.trouble == PlanWords.couldntRead, "a missing CLI or a non-zero exit is not the runner coming back")
        #expect(!plan.reconnecting)
    }

    @Test("Reconnecting gives up after the cap: couldn't read, with Try Again, and nothing more is asked")
    func givesUp() async throws {
        let runner = Runner()
        let plan = Self.store(runner)
        plan.giveUpAfter = .milliseconds(120)
        await plan.reload()
        #expect(plan.reconnecting)
        await Self.eventually("the cap ends the wait") { plan.trouble == PlanWords.couldntRead && !plan.reconnecting }
        let asked = runner.asked
        try await Task.sleep(for: .milliseconds(200))
        #expect(runner.asked == asked, "it stopped asking")
        // Try Again is a fresh wait, and the runner has come back.
        runner.answer = try PlanViewTests.fixture()
        await plan.tryAgain()
        #expect(plan.trouble == nil && !plan.reconnecting && plan.hasRead)
    }

    @Test("The retry stops when the last view of the plan goes away, and starts again when one appears")
    func stopsWhenNobodyLooks() async throws {
        let runner = Runner()
        let plan = Self.store(runner)
        runner.delay = .milliseconds(30)
        plan.attach()
        await plan.reload()
        await Self.eventually("it asks again") { runner.asked >= 3 }
        // Close the view while a read is out: its answer must not arm another.
        let before = runner.asked
        await Self.eventually("a read is out") { runner.asked > before }
        plan.detach()
        try await Task.sleep(for: .milliseconds(80))
        let asked = runner.asked
        try await Task.sleep(for: .milliseconds(200))
        #expect(runner.asked == asked, "nobody is looking, so nobody asks")
        plan.attach()
        await Self.eventually("a view appearing asks again") { runner.asked > asked }
        plan.detach()
    }

    @Test("A malformed plan still can't be read, names the failing field and not its value, and isn't retried")
    func malformed() async throws {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var lanes = try #require(object["lanes"] as? [[String: Any]])
        lanes[0]["name"] = 12345
        object["lanes"] = lanes
        let runner = Runner()
        runner.answer = try JSONSerialization.data(withJSONObject: object)
        let plan = Self.store(runner)
        var logged: [String] = []
        plan.onDecodeFailure = { logged.append($0) }
        await plan.reload()
        #expect(plan.trouble == PlanWords.couldntRead)
        #expect(!plan.reconnecting)
        #expect(logged.count == 1 && logged[0].hasPrefix("lanes[0]."), "logged \(logged)")
        #expect(!logged.joined().contains("12345"), "a path, never a value")
        try await Task.sleep(for: .milliseconds(150))
        #expect(runner.asked == 1, "a plan that's wrong stays wrong; only silence is asked again")
    }
}
