import Testing

@testable import AgentKit

/// When a claude TUI pane reads its ask, and when it shows it.
struct TerminalAskCardTests {
    /// Blocked and on screen: read.
    @Test func aBlockedPaneOnScreenIsRead() {
        #expect(TerminalAskCard.reads(onScreen: true, blocked: true, asking: false))
    }

    /// No longer blocked, but its ask not yet resolved: still read, until the
    /// daemon's `Resolved` takes the ask away. Stopping here left the old ask
    /// to show on the next block.
    @Test func anAskIsReadUntilItIsResolvedNotUntilThePaneUnblocks() {
        #expect(TerminalAskCard.reads(onScreen: true, blocked: false, asking: true))
        #expect(!TerminalAskCard.reads(onScreen: true, blocked: false, asking: false))
    }

    /// Off screen (hidden, under the overview, or the app in the background):
    /// never read, blocked or not.
    @Test func aPaneOffScreenIsNotRead() {
        #expect(!TerminalAskCard.reads(onScreen: false, blocked: true, asking: true))
    }

    /// An ask held from before the stream last started is not shown until the
    /// stream has read again: it may have been resolved meanwhile.
    @Test func anAskIsShownOnlyOnceTheStreamHasCaughtUp() {
        #expect(!TerminalAskCard.shows(asking: true, caughtUp: false))
        #expect(TerminalAskCard.shows(asking: true, caughtUp: true))
        #expect(!TerminalAskCard.shows(asking: false, caughtUp: true))
    }
}
