import Foundation
import Testing

@testable import AgentKit

/// The tab chosen in each worktree, kept across a launch (ov-233). Stands in an
/// enum of `PaneFocus`'s shape, which lives in the iOS target.
struct FocusMemoryTests {
    private enum Focus: Hashable, Codable, Sendable {
        case agent(String)
        case changes
    }

    private func defaults() -> UserDefaults {
        let name = "focus-memory-\(UUID().uuidString)"
        let suite = UserDefaults(suiteName: name)!
        suite.removePersistentDomain(forName: name)
        return suite
    }

    @Test func aChoiceSurvivesALaunch() {
        let suite = defaults()
        FocusMemory<Focus>.save(["w1": .changes, "w2": .agent("t9")], runner: "r1", to: suite)
        #expect(FocusMemory<Focus>.load(runner: "r1", from: suite) == ["w1": .changes, "w2": .agent("t9")])
    }

    @Test func eachRunnerKeepsItsOwn() {
        let suite = defaults()
        FocusMemory<Focus>.save(["w1": .changes], runner: "r1", to: suite)
        #expect(FocusMemory<Focus>.load(runner: "r2", from: suite).isEmpty)
    }

    @Test func nothingKeptOrDamageIsNoMemory() {
        let suite = defaults()
        #expect(FocusMemory<Focus>.load(runner: "r1", from: suite).isEmpty)
        suite.set(Data("not json".utf8), forKey: FocusMemory<Focus>.key(runner: "r1"))
        #expect(FocusMemory<Focus>.load(runner: "r1", from: suite).isEmpty)
    }

    @Test func aWorktreeGoneFromTheFleetIsForgotten() {
        let kept: [String: Focus] = ["w1": .changes, "gone": .agent("t")]
        #expect(FocusMemory<Focus>.pruned(kept, keeping: ["w1"]) == ["w1": .changes])
    }

    @Test func savingNothingRemovesTheKey() {
        let suite = defaults()
        FocusMemory<Focus>.save(["w1": .changes], runner: "r1", to: suite)
        FocusMemory<Focus>.save([:], runner: "r1", to: suite)
        #expect(suite.data(forKey: FocusMemory<Focus>.key(runner: "r1")) == nil)
    }
}

/// The ledger a `Connection` holds, across two launches of one runner (ov-233).
struct FocusLedgerTests {
    private enum Focus: Hashable, Codable, Sendable {
        case agent(String)
        case changes
    }

    private func suite() -> UserDefaults {
        let name = "focus-ledger-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    @Test func aChoiceInOneLaunchIsTheNextLaunchsMemory() {
        let defaults = suite()
        var first = FocusLedger<Focus>(defaults: defaults)
        first.adopt(runner: "r1")
        first.remember(.changes, in: "w1")
        var second = FocusLedger<Focus>(defaults: defaults)
        second.adopt(runner: "r1")
        #expect(second.memory == ["w1": .changes])
    }

    @Test func aChoiceMadeBeforeTheRunnerIsKnownIsNotLostAndDoesNotEraseTheRest() {
        let defaults = suite()
        var old = FocusLedger<Focus>(defaults: defaults)
        old.adopt(runner: "r1")
        old.remember(.agent("t"), in: "w2")
        var next = FocusLedger<Focus>(defaults: defaults)
        next.remember(.changes, in: "w1")
        next.adopt(runner: "r1")
        #expect(next.memory == ["w1": .changes, "w2": .agent("t")])
        var third = FocusLedger<Focus>(defaults: defaults)
        third.adopt(runner: "r1")
        #expect(third.memory == next.memory)
    }

    @Test func pruningKeepsWhatAFleetHasAndNeverActsOnAnEmptyOne() {
        let defaults = suite()
        var ledger = FocusLedger<Focus>(defaults: defaults)
        ledger.adopt(runner: "r1")
        ledger.remember(.changes, in: "w1")
        ledger.remember(.changes, in: "gone")
        ledger.prune(keeping: [])
        #expect(ledger.memory.count == 2)
        ledger.prune(keeping: ["w1"])
        var again = FocusLedger<Focus>(defaults: defaults)
        again.adopt(runner: "r1")
        #expect(again.memory == ["w1": .changes])
    }
}

/// `Connection` is the one place the ledger is adopted, remembered into and
/// pruned: the iOS target has no unit tests, so these read its source. Each
/// goes red when its call is removed, which is the restore going quietly dead.
struct FocusLedgerWiringTests {
    private func connection() throws -> String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try String(contentsOf: root.appendingPathComponent("apps/ios/FarCooler/Connection.swift"), encoding: .utf8)
    }

    private func body(of signature: String, in source: String) throws -> String {
        let start = try #require(source.range(of: signature), "no \(signature)")
        let rest = source[start.upperBound...]
        let end = rest.range(of: "\n    func ")?.lowerBound ?? rest.endIndex
        return String(rest[..<end])
    }

    @Test func startAdoptsTheKeptChoices() throws {
        #expect(try body(of: "func start(host: Runner) async", in: connection()).contains("adoptFocusMemory(runner:"))
    }

    @Test func aChoiceIsRememberedThroughTheLedger() throws {
        #expect(try body(of: "func rememberFocus(", in: connection()).contains("focusLedger.remember("))
    }

    @Test func aFleetReadPrunesTheLedger() throws {
        #expect(try connection().contains("pruneFocus(to: fleet)"))
        #expect(try body(of: "private func pruneFocus(", in: connection()).contains("focusLedger.prune("))
    }
}
