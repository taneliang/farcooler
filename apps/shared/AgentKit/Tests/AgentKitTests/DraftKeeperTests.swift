import Foundation
import Testing

@testable import AgentKit

// The conversation composer's saved draft (ov-369 F4, R-38): per terminal,
// debounced, capped at 64 KB, and gone the moment it is empty.

@MainActor
struct DraftKeeperTests {
    private func defaults() -> UserDefaults {
        UserDefaults(suiteName: "DraftKeeperTests.\(UUID().uuidString)")!
    }

    @Test func aChangeIsSavedAfterTheQuietMomentAndRestored() async {
        let store = defaults()
        let keeper = DraftKeeper(terminal: "t1", defaults: store, delay: .milliseconds(1))
        keeper.changed("half a thought")
        await keeper.settled()
        #expect(DraftKeeper(terminal: "t1", defaults: store).restored == "half a thought")
        #expect(DraftKeeper(terminal: "t2", defaults: store).restored == "", "keyed by terminal")
    }

    @Test func typingWritesOnceAfterTheDelayAndNotPerKey() async {
        let store = defaults()
        let keeper = DraftKeeper(terminal: "t1", defaults: store, delay: .seconds(60))
        keeper.changed("a")
        keeper.changed("ab")
        try? await Task.sleep(for: .milliseconds(100))
        #expect(NativeDraftStore.read("t1", in: store) == "", "nothing before the delay")
        keeper.changed("")
    }

    @Test func anEmptyDraftClearsAtOnceAndAWaitingWriteCannotBringItBack() async {
        let store = defaults()
        NativeDraftStore.write("kept", for: "t1", in: store)
        let keeper = DraftKeeper(terminal: "t1", defaults: store, delay: .milliseconds(1))
        keeper.changed("sent text")
        keeper.changed("")
        #expect(store.object(forKey: NativeDraftStore.key("t1")) == nil)
        try? await Task.sleep(for: .milliseconds(100))
        #expect(keeper.restored == "", "the write that was waiting was cancelled")
    }

    @Test func aLongDraftIsCutToSixtyFourKilobytesOnACharacterBoundary() {
        let text = String(repeating: "é", count: 40_000)  // 80,000 bytes
        let saved = NativeDraftStore.capped(text)
        #expect(saved.utf8.count == NativeDraftStore.capBytes)
        #expect(text.hasPrefix(saved))
        #expect(NativeDraftStore.capped("short") == "short")
    }
}
