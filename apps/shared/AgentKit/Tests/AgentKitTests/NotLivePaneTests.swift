import Foundation
import Testing

@testable import AgentKit

/// The one terminal phase with no way out of it.
///
/// `.notLive` is deliberately not polled — a pane the runner says is not there
/// is not a pane that could not be reached, and asking again every second
/// would spend a round trip a second to be told the same true thing. What was
/// missing is that the pane can come BACK, and the fleet poll has been carrying
/// the word for it all along.
struct NotLivePaneTests {
    /// **The way back.** A pane restarted from the Mac reports `running` on the
    /// very next fleet poll, and that transition is worth one re-attach.
    @Test func aPaneThatStartsRunningAgainIsWorthAsking() {
        #expect(NotLivePane.revives(from: .exited, to: .running))
        #expect(NotLivePane.revives(from: .lost, to: .running))
    }

    /// A pane still coming up counts too: it will be there in a moment, and the
    /// re-attach lands on the screen call that decides.
    @Test func aStartingPaneCounts() {
        #expect(NotLivePane.revives(from: .exited, to: .starting))
    }

    /// **The cost this rule exists to bound.** The fleet re-derives every
    /// terminal's state on every three-second poll, so a rule read as a level
    /// would re-attach forever while the host and the FFI disagreed — which is
    /// exactly the round trip a second `.notLive` refuses to spend. No
    /// transition, no ask.
    @Test func anUnchangedStateIsNotATransition() {
        for state in [StateKind.running, .starting, .exited, .error, .lost, .unknown] {
            #expect(!NotLivePane.revives(from: state, to: state))
        }
    }

    /// The states `.notLive` is honestly reporting do not revive it, however
    /// they are arrived at.
    @Test func theDeadStatesStayDead() {
        #expect(!NotLivePane.revives(from: .running, to: .exited))
        #expect(!NotLivePane.revives(from: .exited, to: .error))
        #expect(!NotLivePane.revives(from: .error, to: .lost))
    }

    /// A word from a daemon newer than this build. Guessing that an unreadable
    /// state means "running" would re-attach on every state a future runner
    /// invents.
    @Test func anUnreadableStateIsNotAnInvitation() {
        #expect(!NotLivePane.revives(from: .exited, to: .unknown))
        #expect(!NotLivePane.revives(from: .unknown, to: .unknown))
        // And it is not a trap either: coming OUT of an unknown state into a
        // running one is the ordinary case for a runner this build is behind.
        #expect(NotLivePane.revives(from: .unknown, to: .running))
    }

    // MARK: - What it says

    /// Not a failure and not an alarm, and the pane is named — a screen that
    /// said only "Not live" would not say which of several panes it meant.
    @Test func theSentenceNamesThePane() {
        #expect(NotLivePane.message(for: "claude 2") == "claude 2 has no running pane right now.")
        #expect(NotLivePane.title == "Not live")
    }

    /// Title case, as every other button in this app is.
    @Test func theButtonIsTitleCase() {
        #expect(NotLivePane.action == "Try Again")
    }
}
