import Foundation
import Testing

@testable import AgentKit

// What a phone says when a pane is a chat with no agent in it.
//
// Here rather than in the iOS UI suite, which CI compiles and never runs, and
// rather than in a `View.body`, which nothing can read back at all. Every way
// of getting this wrong is silent: a word with no copy draws nothing, a raw
// value that drifts from the runner's word draws nothing, and a sentence that
// quotes the word back puts a Rust identifier on a screen. All three end where
// the bug started — a chat that spins forever and explains nothing.
//
// Run by `swift test --package-path apps/shared/AgentKit`, `.github/workflows/
// ci.yml:404-405`. Android mirrors these in
// `apps/android/.../ui/AgentEmptyStateTest.kt`, because Kotlin cannot import
// this.

/// The words, written out.
///
/// The other half is `every_failure_has_a_stable_word` in
/// `farcooler-agent-core`, which pins the same four strings on the sending
/// side. Neither test can see the other, and that is exactly why both exist:
/// the wire between them is four literals and nothing else.
@Test func everyWordTheRunnerSendsHasItsOwnCopy() {
    #expect(AgentFailure(rawValue: "no-adapter") == .noAdapter)
    #expect(AgentFailure(rawValue: "not-authenticated") == .notAuthenticated)
    #expect(AgentFailure(rawValue: "adapter-silent") == .adapterSilent)
    #expect(AgentFailure(rawValue: "adapter-failed") == .adapterFailed)

    for failure in AgentFailure.allCases {
        #expect(AgentFailureCopy.forWord(failure.rawValue) == AgentFailureCopy.copy(for: failure))
    }
}

/// A word this build has never heard of still says a pane failed.
///
/// **The one place the phones part company with the Mac**, and deliberately.
/// The runner sends this field only to report that a pane gave up; reading a
/// fifth word as silence would put back the endless spinner this whole path
/// exists to end. It degrades to the generic failure — which is what
/// `adapter-failed` already means — and it still offers the way out.
@Test func aWordFromTheFutureReadsAsAGenericFailure() {
    let copy = AgentFailureCopy.forWord("from-the-future")
    #expect(copy == AgentFailureCopy.copy(for: .adapterFailed))
    #expect(copy?.action.isEmpty == false)
}

/// A pane nobody has reported on is not a failed pane.
///
/// Nil, so the caller keeps drawing the spinner and the ladder underneath it: a
/// pane that is still coming up is indistinguishable from this, and calling it
/// broken would be the mirror image of the bug.
@Test func aTerminalWithNoWordHasNoFailure() {
    #expect(AgentFailureCopy.forWord(nil) == nil)
    #expect(AgentFailureCopy.forWord("") == nil)
}

/// The word the runner sent must never be the words a person reads.
///
/// The same assertion `runner_pipe.rs` makes about `TunnelError.code`. Quoting
/// the machine word back — "adapter-silent" on a screen — is the failure this
/// convention exists to prevent, and it is the easiest one to write by accident
/// when a sentence is being filled in quickly.
@Test func noSentenceQuotesTheMachineWordBack() {
    for failure in AgentFailure.allCases {
        let copy = AgentFailureCopy.copy(for: failure)
        let words = copy.title + " " + copy.message + " " + copy.action
        #expect(
            !words.contains(failure.rawValue),
            "the stable word leaked into the copy for \(failure.rawValue): \(words)")
        // The case names are internal identifiers too, and they are the other
        // thing a hurried `switch` puts on a screen.
        for name in ["noAdapter", "notAuthenticated", "adapterSilent", "adapterFailed"] {
            #expect(!words.contains(name))
        }
    }
}

/// Four failures, four different headlines.
///
/// Each of these has a different fix — a config entry, a login on the runner,
/// waiting, and nobody knows — so a shared sentence would be the endless
/// spinner again with better manners.
@Test func eachFailureSaysSomethingOfItsOwn() {
    let titles = Set(AgentFailure.allCases.map { AgentFailureCopy.copy(for: $0).title })
    #expect(titles.count == AgentFailure.allCases.count)
}

/// Nothing renders blank, and every failure offers the way out.
///
/// The action is the point of the ruling: the pane STAYS a chat, and the switch
/// back to the terminal is named rather than performed, because performing it
/// respawns the pane under whatever the reader was in the middle of typing. A
/// failure that forgot to offer it would leave the reader exactly as stuck as
/// the spinner did.
@Test func everyFailureIsReadableAndOffersTheTerminal() {
    for failure in AgentFailure.allCases {
        let copy = AgentFailureCopy.copy(for: failure)
        #expect(!copy.title.isEmpty)
        #expect(!copy.message.isEmpty)
        #expect(copy.title.first?.isUppercase == true)
        // One label for one action: this is the pane's existing overflow item.
        #expect(copy.action == "Show the Terminal")
    }
}

/// The word arrives on the terminal the fleet already decodes.
///
/// The decoder ignores keys it does not know, so the field being absent from
/// `Terminal` broke nothing and reported nothing — which is how it stayed
/// missing on both phones for a day after the runner started sending it.
@Test func aTerminalCarriesTheRunnersWordAndTurnsItIntoASentence() throws {
    let json = """
        {"id":"t1","short":"t1","title":"Terminal 1","preset":"claude","state":"running",
         "epoch":1,"paneMode":"agent","agentFailure":"not-authenticated"}
        """
    let terminal = try JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
    #expect(terminal.agentFailure == "not-authenticated")
    #expect(terminal.chatFailure == AgentFailureCopy.copy(for: .notAuthenticated))
    #expect(terminal.isAgentPane)
}

/// And a terminal nobody reported on has no sentence at all.
@Test func aTerminalWithoutTheWordDrawsNoFailure() throws {
    let json = """
        {"id":"t1","short":"t1","title":"Terminal 1","preset":"claude","state":"running",
         "epoch":1,"paneMode":"agent"}
        """
    let terminal = try JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
    #expect(terminal.agentFailure == nil)
    #expect(terminal.chatFailure == nil)
}
