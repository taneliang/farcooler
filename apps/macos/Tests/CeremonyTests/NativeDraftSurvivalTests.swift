import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The conversation composer's draft survives the pane's model going away
/// (ov-369 F4, R-38): app quit, jetsam and process death all end as "a new
/// model for the same terminal". It is cleared when the runner confirms the
/// send, not when Send is pressed.
@MainActor
@Suite(.serialized)
struct NativeDraftSurvivalTests {
    typealias Sink = NativeAgentTests.StandInSink

    private func defaults() -> UserDefaults {
        UserDefaults(suiteName: "native-draft-survival-\(UUID().uuidString)")!
    }

    private func model(_ defaults: UserDefaults, sink: Sink? = nil) throws -> NativePaneModel {
        let terminal = try NativeAgentTests.terminal()
        NativePaneModel.remember(false, for: terminal.id)
        return NativePaneModel(
            terminal: terminal.id, store: AgentRowStore(key: "test-\(UUID())", cache: nil), sink: sink,
            draftKeeper: DraftKeeper(terminal: terminal.id, defaults: defaults, delay: .milliseconds(1)))
    }

    @Test("A draft survives the model being made again")
    func aDraftSurvivesTheModelBeingRecreated() async throws {
        let store = defaults()
        let first = try model(store)
        first.draft = "fix the login, then the tests"
        await first.draftKeeper?.settled()
        let second = try model(store)
        #expect(second.draft == "fix the login, then the tests")
    }

    @Test("A send the runner confirmed clears the saved draft")
    func aConfirmedSendClearsIt() async throws {
        let store = defaults()
        let sink = Sink()
        await sink.set(.success(false))
        let first = try model(store, sink: sink)
        first.draft = "ship it"
        await first.draftKeeper?.settled()
        await first.send()
        #expect(await sink.sent == ["ship it"])
        let second = try model(store)
        #expect(second.draft.isEmpty)
    }

    @Test("A send the runner didn't confirm keeps the saved draft")
    func anUnconfirmedSendKeepsIt() async throws {
        let store = defaults()
        let sink = Sink()
        await sink.set(.failure(.refused("no", word: "resource-conflict", what: "draft")))
        let first = try model(store, sink: sink)
        first.draft = "ship it"
        await first.draftKeeper?.settled()
        await first.send()
        #expect(first.issue != nil && first.draft == "ship it")
        let second = try model(store)
        #expect(second.draft == "ship it")
    }
}
