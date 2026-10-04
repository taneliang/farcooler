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
