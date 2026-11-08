import Foundation
import Testing

@testable import Far_Cooler

/// The recovery screen's one button (ov-984): Touch ID, then the runner's
/// five steps in order, from where it stands; a failure at any of them says
/// so in a sentence and offers Recover.
@MainActor
struct RunnerUpgradeTests {
    /// A runner that answers each `workflow-cutover` op as told, and remembers
    /// what was asked of it.
    final class Runner: @unchecked Sendable {
        let lock = NSLock()
        var asked: [[String]] = []
        /// What `inspect` prints, in order; the last one repeats.
        var inspected: [String]
        /// The ops that fail, with the runner's words.
        var refusing: [String: String]
        var confirmed: RunnerUpgrade.Confirmed = .yes
        var prompts: [String] = []

        init(inspected: [String] = [legacy], refusing: [String: String] = [:]) {
            self.inspected = inspected
            self.refusing = refusing
        }

        var ops: [String] { lock.withLock { asked.map { $0[2] } } }

        func run(_ args: [String]) -> CLI.Result {
            lock.withLock {
                asked.append(args)
                let op = args[2]
                if op == "inspect" {
                    let out = inspected.count > 1 ? inspected.removeFirst() : inspected[0]
                    return CLI.Result(ok: true, output: out, errors: "")
                }
                if let words = refusing[op] { return CLI.Result(ok: false, output: "", errors: words) }
                return CLI.Result(ok: true, output: "{\"schema\":1}", errors: "")
            }
        }

        var driver: RunnerUpgrade.Driver {
            RunnerUpgrade.Driver(
                run: { self.run($0) },
                confirm: { reason in self.lock.withLock { self.prompts.append(reason); return self.confirmed } },
                attempt: { "mac-test" })
        }
    }

    nonisolated static let legacy = #"{"schema":1,"artifacts":false,"attempt":null,"authority":{"phase":"legacy"}}"#
    nonisolated static func at(_ phase: String, attempt: String = "a-1") -> String {
        #"{"schema":1,"artifacts":true,"attempt":"\#(attempt)","authority":{"phase":"\#(phase)"}}"#
    }

    @Test func touchIDThenEveryStepInOrderThenTheWindowReadsTheRunnerAgain() async {
        let runner = Runner()
        let upgrade = RunnerUpgrade(driver: runner.driver)
        let refreshed = Counter()
        await upgrade.finish { refreshed.bump() }
        #expect(runner.prompts == [RunnerUpgrade.finishReason])
        #expect(runner.ops == ["inspect", "prepare", "commit", "verify", "release", "finish"])
        #expect(runner.asked[1] == ["--json", "workflow-cutover", "prepare", "--attempt", "mac-test"])
        #expect(upgrade.phase == .done)
        #expect(refreshed.count == 1)
    }

    @Test func noTouchIDRunsNothing() async {
        let runner = Runner()
        runner.confirmed = .cancelled
        let upgrade = RunnerUpgrade(driver: runner.driver)
        await upgrade.finish()
        #expect(runner.asked.isEmpty)
        #expect(upgrade.phase == .ready(notice: nil))

        runner.confirmed = .unavailable("Set a login password on this Mac to finish the upgrade.")
        await upgrade.finish()
        #expect(runner.asked.isEmpty)
        #expect(upgrade.phase == .ready(notice: "Set a login password on this Mac to finish the upgrade."))
    }

    /// A failure at each step: that step's sentence, the runner's words, and
    /// Recover; nothing after it runs.
    @Test(arguments: RunnerUpgrade.Step.allCases)
    func aFailureAtAnyStepSaysWhatHappenedAndOffersRecover(step: RunnerUpgrade.Step) async {
        let runner = Runner(refusing: [step.op: "code: operation-failed\nThat upgrade step was refused."])
        let upgrade = RunnerUpgrade(driver: runner.driver)
        await upgrade.finish()
        let failed = RunnerUpgrade.Failed.step(step)
        #expect(upgrade.phase == .failed(failed, detail: "code: operation-failed\nThat upgrade step was refused."))
        #expect(runner.ops.last == step.op, "nothing after \(step.op) runs")
        #expect(!step.failure.isEmpty && step.failure.hasSuffix("."))
        #expect(!step.failure.contains("cutover") && !step.failure.contains("`"), "plain words: \(step.failure)")
        #expect(RunnerUpgrade.recoverCopy(for: failed).contains("Recover"))
        #expect(!upgrade.isBusy, "Recover can be pressed")
        #expect(
            RunnerUpgradeView.actions(for: upgrade.phase) == [.recover],
            "a failure offers Recover, and only Recover")
    }

    @Test func recoverBeforeCommitPutsTheOriginalDataBack() async {
        let runner = Runner(inspected: [Self.legacy, Self.at("quiesced")], refusing: ["commit": "refused"])
        let upgrade = RunnerUpgrade(driver: runner.driver)
        await upgrade.finish()
        #expect(upgrade.phase == .failed(.step(.commit), detail: "refused"))
        #expect(!RunnerUpgrade.recoverCopy(for: .step(.commit)).contains("exactly as it was"), "Commit may have landed")
        let refreshed = Counter()
        await upgrade.recover { refreshed.bump() }
        #expect(runner.prompts == [RunnerUpgrade.finishReason, RunnerUpgrade.recoverReason])
        #expect(runner.ops.suffix(2) == ["inspect", "recover"])
        #expect(upgrade.phase == .ready(notice: RunnerUpgrade.recoveredNotice))
        #expect(refreshed.count == 1)
    }

    @Test func recoverAfterCommitFinishesTheStepsLeft() async {
        let runner = Runner(inspected: [Self.legacy, Self.at("committed")], refusing: ["verify": "refused"])
        let upgrade = RunnerUpgrade(driver: runner.driver)
        await upgrade.finish()
        #expect(upgrade.phase == .failed(.step(.verify), detail: "refused"))
        #expect(RunnerUpgrade.recoverCopy(for: .step(.verify)).contains("finishes the steps that are left"))
        runner.refusing = [:]
        await upgrade.recover()
        #expect(!runner.ops.contains("recover"), "a committed upgrade is never rolled back")
        #expect(runner.ops.suffix(4) == ["inspect", "verify", "release", "finish"])
        #expect(upgrade.phase == .done)
    }

    @Test func recoverWithNothingBegunGoesBackToTheStart() async {
        let runner = Runner(refusing: ["prepare": "Another process (pid 9) has this runner's database open. Close it, then try again."])
        let upgrade = RunnerUpgrade(driver: runner.driver)
        await upgrade.finish()
        runner.refusing = [:]
        await upgrade.recover()
        #expect(!runner.ops.contains("recover"))
        #expect(upgrade.phase == .ready(notice: nil))
    }

    @Test func aRecoverThatFailsSaysSoAndOffersItAgain() async {
        let runner = Runner(inspected: [Self.at("quiesced")], refusing: ["commit": "refused", "recover": "changed"])
        let upgrade = RunnerUpgrade(driver: runner.driver)
        await upgrade.finish()
        await upgrade.recover()
        #expect(upgrade.phase == .failed(.recover, detail: "changed"))
        #expect(RunnerUpgradeView.actions(for: upgrade.phase) == [.recover])
    }

    @Test func closingThePromptKeepsTheFailureAndItsRecover() async {
        let runner = Runner(refusing: ["verify": "refused"])
        let upgrade = RunnerUpgrade(driver: runner.driver)
        await upgrade.finish()
        runner.confirmed = .cancelled
        await upgrade.recover()
        #expect(upgrade.phase == .failed(.step(.verify), detail: "refused"))
    }

    @Test func itStartsWhereTheRunnerStands() async {
        for (inspected, ops) in [
            (Self.at("quiesced"), ["commit", "verify", "release", "finish"]),
            (Self.at("committed"), ["verify", "release", "finish"]),
            (Self.at("released"), ["finish"]),
        ] {
            let runner = Runner(inspected: [inspected])
            let upgrade = RunnerUpgrade(driver: runner.driver)
            await upgrade.finish()
            #expect(runner.ops == ["inspect"] + ops, "\(inspected)")
            #expect(upgrade.steps.map(\.op) == ops)
        }
    }

    /// A runner a live Prepare held is prepared under its attempt: the runner
    /// refuses any other.
    @Test func aHeldRunnerIsPreparedUnderItsOwnAttempt() async {
        let runner = Runner(inspected: [#"{"schema":1,"artifacts":true,"attempt":"held-7","authority":{"phase":"legacy"}}"#])
        await RunnerUpgrade(driver: runner.driver).finish()
        #expect(runner.asked[1] == ["--json", "workflow-cutover", "prepare", "--attempt", "held-7"])
    }

    @Test func anInspectTheRunnerCouldNotAnswerFailsAtTheFirstStep() async {
        let runner = Runner(inspected: ["not json"])
        let upgrade = RunnerUpgrade(driver: runner.driver)
        await upgrade.finish()
        #expect(upgrade.phase == .failed(.step(.prepare), detail: "not json"))
        #expect(runner.ops == ["inspect"])
    }

    /// Recover's promise follows where the runner stands, which a failed Commit
    /// can't tell (its reply may have been lost after it landed).
    @Test func aFailedCommitDoesNotPromiseAnExactRestore() {
        #expect(RunnerUpgrade.recoverCopy(for: .step(.prepare)).contains("exactly as it was"))
        let commit = RunnerUpgrade.recoverCopy(for: .step(.commit))
        #expect(!commit.contains("exactly as it was") && commit.contains("Recover"))
    }

    /// A refusal that will come again is not offered a retry, and the copy says
    /// the upgrade needs help instead.
    @Test func aRefusalThatWillComeAgainOffersNoRetry() async {
        for words in [
            "database changed since the upgrade; keeping it for review",
            "the attempt’s closed backup is missing", "the attempt's closed backup is missing",
        ] {
            #expect(RunnerUpgrade.needsHelp(words), "\(words)")
            let stuck = RunnerUpgrade.Phase.failed(.recover, detail: words)
            #expect(RunnerUpgradeView.actions(for: stuck).isEmpty, "\(words)")
            #expect(RunnerUpgrade.recoverCopy(for: .recover, detail: words).contains("needs help"))
            #expect(!RunnerUpgrade.recoverCopy(for: .recover, detail: words).contains("again"))
        }
        #expect(!RunnerUpgrade.needsHelp("Another process (pid 9) has this runner's database open."))
        let transient = RunnerUpgrade.Phase.failed(.recover, detail: "Another process (pid 9) has this runner's database open.")
        #expect(RunnerUpgradeView.actions(for: transient) == [.recover])
        #expect(RunnerUpgrade.recoverCopy(for: .recover, detail: "busy").contains("Try Recover again"))
    }

    /// The box holds the runner's sentences, not the CLI's labels or a pointer
    /// to a terminal.
    @Test func theRunnersWordsLoseTheTerminalPointerAndLabels() {
        let said = "code: operation-failed\nwhat: x\n"
            + "That upgrade step was refused. Run `farcooler workflow-cutover inspect` to see where the upgrade stands."
        #expect(RunnerUpgrade.plainWords(said) == "That upgrade step was refused.")
        #expect(RunnerUpgrade.plainWords("Another process has this runner's database open. Close it, then try again.")
            == "Another process has this runner's database open. Close it, then try again.")
        #expect(RunnerUpgrade.plainWords("code: operation-failed") == "")
        // The CLI puts the message on the `error:` line itself: the label goes, the sentence stays.
        #expect(
            RunnerUpgrade.plainWords("error: scope denied: person required\ncode: scope-denied")
                == "scope denied: person required")
        #expect(
            RunnerUpgrade.plainWords("Error: That upgrade step was refused. Run `farcooler workflow-cutover inspect` to see where.\ncode: operation-failed")
                == "That upgrade step was refused.")
    }

    @Test func restartingSaysTheRunnerNotTheApp() {
        #expect(RunnerUpgrade.Step.finish.title.contains("runner"))
        #expect(!RunnerUpgrade.Step.finish.title.contains("Far Cooler"))
    }

    /// A second press while the first waits on Touch ID asks nothing more.
    @Test func aDoublePressAsksOnce() async {
        final class Gate: @unchecked Sendable {
            let lock = NSLock()
            var waiting: CheckedContinuation<RunnerUpgrade.Confirmed, Never>?
            var asks = 0
        }
        let gate = Gate()
        let runner = Runner()
        let upgrade = RunnerUpgrade(
            driver: .init(
                run: { runner.run($0) },
                confirm: { _ in
                    // The first ask waits for the test; any other answers at once,
                    // so a missing guard fails the count rather than hanging.
                    let first = gate.lock.withLock { gate.asks += 1; return gate.asks == 1 }
                    if !first { return .cancelled }
                    return await withCheckedContinuation { c in gate.lock.withLock { gate.waiting = c } }
                },
                attempt: { "mac-test" }))
        let first = Task { await upgrade.finish() }
        while gate.lock.withLock({ gate.waiting }) == nil { await Task.yield() }
        await upgrade.finish()
        await upgrade.recover()
        #expect(gate.lock.withLock { gate.asks } == 1)
        gate.lock.withLock { gate.waiting }?.resume(returning: .cancelled)
        await first.value
        #expect(runner.asked.isEmpty)
    }

    @Test func aNewAttemptIsANameTheRunnerTakes() {
        let name = RunnerUpgrade.newAttempt(now: Date(timeIntervalSince1970: 1_794_000_000))
        #expect(name.hasPrefix("mac-"))
        #expect(name.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") })
    }

    /// The recovery screen is what a runner mid-upgrade draws, ahead of the
    /// fleet's other states, rather than "Couldn't Read the Fleet".
    @Test func aRunnerMidUpgradeDrawsTheRecoveryScreen() {
        let held = "Error: version-incompatible\ncode: version-incompatible\nwhat: cutover_held"
        #expect(
            FleetPlaceholder.phase(hasWorktrees: false, localLoaded: false, localError: held, hasRepositories: false)
                == .upgrade)
        #expect(
            FleetPlaceholder.phase(
                hasWorktrees: true, localLoaded: false, localError: held, hasRepositories: true, restoringOn: "",
                trouble: held) == .upgrade)
        #expect(
            FleetPlaceholder.phase(
                hasWorktrees: false, localLoaded: false, localError: "code: version-incompatible",
                hasRepositories: false) == .failed("code: version-incompatible"))
    }
}

final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    var count: Int { lock.withLock { value } }
    func bump() { lock.withLock { value += 1 } }
}
