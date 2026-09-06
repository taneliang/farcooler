import Foundation
import Testing

@testable import Far_Cooler

/// What a pane says when it is a chat with no agent in it.
///
/// In the target with teeth because every way of getting this wrong is
/// silent. The runner sends a stable machine word — `not-authenticated` — and
/// this app owns the sentence a person reads. A word with no case here draws
/// nothing at all; a case whose raw value drifts from the runner's word draws
/// nothing at all; and a sentence that quotes the word back is a Rust
/// identifier on a screen. None of the three is visible from inside this app,
/// and all three end at the same place the bug started: a chat that explains
/// nothing.
struct AgentFailureTests {
    /// The words, written out.
    ///
    /// The other half is `every_failure_has_a_stable_word` in
    /// `farcooler-agent-core`, which pins the same four strings on the sending
    /// side. Neither test can see the other, and that is exactly why both
    /// exist: the wire between them is four literals and nothing else.
    @Test func everyWordTheRunnerSendsHasACase() {
        #expect(AgentFailure(rawValue: "no-adapter") == .noAdapter)
        #expect(AgentFailure(rawValue: "not-authenticated") == .notAuthenticated)
        #expect(AgentFailure(rawValue: "adapter-silent") == .adapterSilent)
        #expect(AgentFailure(rawValue: "adapter-failed") == .adapterFailed)
    }

    /// A word this build does not know reads as no failure, never as a blank
    /// row.
    ///
    /// A newer runner inventing a fifth word must leave this app drawing what
    /// it drew before rather than an empty amber box with nothing in it.
    @Test func aWordFromTheFutureIsNotAFailureThisAppCanDraw() {
        #expect(AgentFailure(rawValue: "from-the-future") == nil)

        let json = """
            {"id":"t1","short":"t1","title":"Terminal 1","preset":"claude","state":"running",
             "epoch":1,"paneMode":"agent","agentFailure":"from-the-future"}
            """
        let terminal = try! JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
        #expect(terminal.chatFailure == nil)
    }

    /// A pane nobody has reported on is not a failed pane.
    @Test func aTerminalWithNoWordHasNoFailure() {
        let json = """
            {"id":"t1","short":"t1","title":"Terminal 1","preset":"claude","state":"running",
             "epoch":1,"paneMode":"agent"}
            """
        let terminal = try! JSONDecoder().decode(Terminal.self, from: Data(json.utf8))
        #expect(terminal.agentFailure == nil)
        #expect(terminal.chatFailure == nil)
    }

    /// The word the runner sent must never be the words a person reads.
    ///
    /// The same assertion `runner_pipe.rs` makes about `TunnelError::code`.
    /// Quoting the machine word back — "adapter-silent" on a screen — is the
    /// failure this whole convention exists to prevent, and it is the easiest
    /// one to write by accident when a sentence is being filled in quickly.
    @Test func noSentenceQuotesTheMachineWordBack() {
        for failure in AgentFailure.allCases {
            let copy = failure.sentence + " " + (failure.advice ?? "")
            #expect(
                !copy.contains(failure.rawValue),
                "the stable word leaked into the copy for \(failure.rawValue): \(copy)")
            #expect(!failure.sentence.isEmpty)
        }
    }

    /// Four failures, four different sentences.
    ///
    /// Each of these has a different fix — a config entry, a login on the
    /// runner, waiting, and nobody knows — so a shared sentence would be the
    /// endless spinner again with better manners.
    @Test func eachFailureSaysSomethingOfItsOwn() {
        let sentences = Set(AgentFailure.allCases.map(\.sentence))
        #expect(sentences.count == AgentFailure.allCases.count)
    }
}
