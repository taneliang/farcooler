import Testing

@testable import AgentKit

/// Which blocked panes the phone reads a permission off, so the lock screen
/// card can offer Allow and Deny for a claude TUI pane.
struct BlockedAskLookupsTests {
    /// A pane the poll says is blocked is read.
    @Test func aBlockedPaneIsRead() {
        var lookups = BlockedAskLookups()
        let step = lookups.poll(blocked: ["b", "a"])
        #expect(step.read == ["a", "b"])
        #expect(step.clear == [])
    }

    /// Read again on the next poll, because the first read can come before the
    /// hook's ask reaches the ring, and one ask can follow another between
    /// two polls.
    @Test func aPaneStillBlockedIsReadAgainOnceItsReadFinished() {
        var lookups = BlockedAskLookups()
        _ = lookups.poll(blocked: ["a"])
        let filed = lookups.finished("a")
        #expect(filed)
        #expect(lookups.poll(blocked: ["a"]).read == ["a"])
    }

    /// No second read of one pane while the first is still running.
    @Test func aPaneBeingReadIsNotReadTwice() {
        var lookups = BlockedAskLookups()
        _ = lookups.poll(blocked: ["a"])
        let step = lookups.poll(blocked: ["a", "b"])
        #expect(step.read == ["b"])
    }

    /// Answered at the keyboard, or the turn moved on: the card must stop
    /// offering buttons for it.
    @Test func aPaneThatStopsBeingBlockedIsCleared() {
        var lookups = BlockedAskLookups()
        _ = lookups.poll(blocked: ["a", "b"])
        let step = lookups.poll(blocked: ["b"])
        #expect(step.clear == ["a"])
        #expect(step.read == [])
    }

    /// A pane that was never blocked is never cleared, so a chat pane's record
    /// written by its own stream is left alone.
    @Test func aPaneNeverBlockedIsNeverCleared() {
        var lookups = BlockedAskLookups()
        #expect(lookups.poll(blocked: []).clear == [])
        _ = lookups.poll(blocked: ["a"])
        #expect(lookups.poll(blocked: ["a"]).clear == [])
    }

    /// A read that finishes after the pane stopped being blocked is not filed:
    /// the poll between already cleared it, and filing would bring the buttons
    /// back for an ask that is over.
    @Test func aReadThatOutlivedTheBlockIsNotFiled() {
        var lookups = BlockedAskLookups()
        _ = lookups.poll(blocked: ["a"])
        _ = lookups.poll(blocked: [])
        let filed = lookups.finished("a")
        #expect(!filed)
    }

    /// And one that finishes while the pane is still blocked is.
    @Test func aReadThatFinishesWhileBlockedIsFiled() {
        var lookups = BlockedAskLookups()
        _ = lookups.poll(blocked: ["a"])
        _ = lookups.poll(blocked: ["a"])
        let filed = lookups.finished("a")
        #expect(filed)
    }
}
