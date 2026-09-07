import Foundation
import Testing

@testable import AgentKit

// What a phone says when a runner refuses one request.
//
// Here rather than in the iOS UI suite, which CI compiles and never runs, and
// rather than beside the sheet that draws it, because five different containers
// draw these — a sheet section, a full-pane state, an inline notice, a red
// banner and a watch glance — and five copies of a decision is the drift this
// codebase keeps finding.
//
// Every way of getting this wrong is quiet. A word that drifts from the Rust
// side matches nothing and the sentence silently reverts to the generic. A
// caller that treats an unrecognized word as "no failure" shows nothing at all,
// which is the bug that shipped in the failure-word work. And a sentence that
// quotes the word puts a proto identifier on a screen.
//
// Run by `swift test --package-path apps/shared/AgentKit`,
// `.github/workflows/ci.yml:405`. Android mirrors these in
// `apps/android/app/src/test/java/com/farcooler/model/RunnerRefusalTest.kt`,
// because Kotlin cannot import this.

/// Every word here is one a runner can really send.
///
/// The other half of this wire is `farcooler_core::error::word`, which is
/// exhaustive over `ErrorCode` and cannot see Swift. So this reads the proto
/// itself and checks each raw value names a code that is declared there — the
/// one drift that would break all fourteen at once, silently, by turning every
/// sentence back into the generic one.
@Test func everyWordNamesACodeTheProtocolDeclares() throws {
    // `#filePath` is this file inside the checkout, so the proto is findable
    // from it without the test knowing anything about the runner it is on.
    var root = URL(fileURLWithPath: #filePath)
    // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
    for _ in 0..<6 { root.deleteLastPathComponent() }
    let proto = root.appendingPathComponent("proto/farcooler.proto")

    // Loudly, not by skipping: a guard that quietly passes when it cannot find
    // what it guards is the failure mode this repo keeps finding.
    let text = try String(contentsOf: proto, encoding: .utf8)

    for refusal in RunnerRefusal.allCases {
        let declared = "ERROR_CODE_" + refusal.rawValue.uppercased().replacingOccurrences(
            of: "-", with: "_")
        #expect(
            text.contains(declared),
            "\(refusal.rawValue) is not a code the protocol declares (\(declared))")
    }
}

/// The word is read off the line the client core actually writes.
///
/// **A gap this suite had, found by breaking it.** Blanking the field read in
/// `ClientCore.drain` left every one of these tests green — the table was
/// perfect and nothing ever reached it, which is exactly the failure this whole
/// change is about, one layer up. So the two lines that read the line moved
/// here, where `swift test` can break them.
///
/// The shapes below are `push_call`'s in `crates/client/src/ffi.rs`, which the
/// Rust test `a_refusal_reaches_the_line_with_the_runner_s_word_on_it` pins from
/// the writing side.
@Test func theWordIsReadOffTheLineTheCoreWrites() {
    let refused: [String: Any] = [
        "ticket": 7, "ok": false, "disconnected": false,
        "error": "workspaces still exist under this resource",
        "code": "workspaces-exist",
    ]
    #expect(RunnerRefusal.word(inAnswerLine: refused) == "workspaces-exist")
    #expect(
        RunnerRefusal.trouble(
            forWord: RunnerRefusal.word(inAnswerLine: refused),
            message: refused["error"] as? String ?? "",
            otherwise: "Generic."
        ).sentence == RunnerRefusal.workspacesExist.sentence)

    // A dropped link carries no code at all — the key is absent, not null.
    let dropped: [String: Any] = [
        "ticket": 8, "ok": false, "disconnected": true, "error": "not connected",
    ]
    #expect(RunnerRefusal.word(inAnswerLine: dropped) == nil)

    // And an answer that worked carries neither.
    let fine: [String: Any] = ["ticket": 9, "ok": true, "result": [String: Any]()]
    #expect(RunnerRefusal.word(inAnswerLine: fine) == nil)
}

/// A word this build has never heard of still says something failed.
///
/// **The rule, stated where it is decided.** A runner newer than this app sends
/// a code that is not in this enum yet; reading it as "nothing to report" is
/// how a screen ends up blank where it owes the reader a failure. It falls back
/// to the caller's own generic sentence with the runner's words underneath —
/// which is exactly what every one of these screens showed before this table
/// existed, so an unknown code can only ever be as good as the old behavior and
/// never worse.
@Test func aCodeFromTheFutureStillReadsAsAFailure() {
    let trouble = RunnerRefusal.trouble(
        forWord: "from-the-future",
        message: "the runner said something this build cannot read",
        otherwise: "Adding this repository didn’t finish.")
    #expect(trouble.sentence == "Adding this repository didn’t finish.")
    #expect(trouble.transcript == "the runner said something this build cannot read")

    // The two words the client core sends for "no reason given" and "a reason
    // this build cannot read". Both are failures, and neither is a diagnosis.
    for word in ["unspecified", "unrecognized"] {
        let t = RunnerRefusal.trouble(forWord: word, message: "raw", otherwise: "Generic.")
        #expect(t.sentence == "Generic.", "\(word) must not claim a diagnosis")
        #expect(t.transcript == "raw", "\(word) must keep the runner's words")
    }
}

/// The codes that are real but have no sentence of their own, and why.
///
/// Deliberately listed rather than left to the fallback by accident. Twelve of
/// the protocol's twenty-eight cannot reach a phone at all, `operation-failed`
/// IS the generic, and `confirmation-required` is turned into a structured
/// outcome by the client core before an app sees it. If a later change makes
/// one of them reachable and worth a sentence, this test is where somebody
/// notices the decision was made.
@Test func theCodesWithNoSentenceFallBackToTheCallersOwn() {
    let noSentence = [
        // No `DomainError` variant produces it.
        "host-offline",
        // No production site anywhere in the tree.
        "dirty-worktree", "repository-locked", "output-gap", "attachment-limit",
        "dispatch-unknown", "diff-too-large", "diff-unsupported", "pr-state-unavailable",
        // Intercepted at the handshake as `SessionError::VersionMismatch`.
        "version-incompatible",
        // Raised on the local send; never put in a response.
        "client-too-slow",
        // No request path ever sets an idempotency key.
        "idempotency-mismatch",
        // The generic itself, and the one the client core answers structurally.
        "operation-failed", "confirmation-required",
    ]
    for word in noSentence {
        #expect(
            RunnerRefusal(rawValue: word) == nil,
            "\(word) has a sentence but nothing can show it")
        let t = RunnerRefusal.trouble(forWord: word, message: "raw", otherwise: "Generic.")
        #expect(t.sentence == "Generic.")
        #expect(t.transcript == "raw")
    }
}

/// Nothing refused anything, so there is nothing to diagnose.
///
/// A dropped link and an argument this app rejected before sending both arrive
/// with no word. They are still failures — the caller's generic sentence and
/// the words it has — just not ones a runner named.
@Test func aFailureNoRunnerNamedKeepsTheCallersSentence() {
    for word in [nil, ""] as [String?] {
        let t = RunnerRefusal.trouble(forWord: word, message: "not connected", otherwise: "Generic.")
        #expect(t.sentence == "Generic.")
        #expect(t.transcript == "not connected")
    }
}

/// A word we do know replaces the sentence and drops the transcript.
///
/// The transcript goes because we have a diagnosis of our own — the same
/// scoping `RunnerTrouble.showsTheRunnersOwnWords` uses. The core's `Display`
/// under one of these says strictly less than the sentence above it
/// ("workspaces still exist under this resource" under "Remove those first"),
/// so keeping it would be noise rather than diagnosis.
@Test func aKnownWordSpeaksForItselfAndNeedsNoTranscript() {
    for refusal in RunnerRefusal.allCases {
        let t = RunnerRefusal.trouble(
            forWord: refusal.rawValue,
            message: "the core’s own log line",
            otherwise: "Generic.")
        #expect(t.sentence == refusal.sentence)
        #expect(t.sentence != "Generic.", "\(refusal.rawValue) did not replace the generic")
        #expect(t.transcript == nil, "\(refusal.rawValue) kept a transcript it does not need")
    }
}

/// A step that failed keeps its own sentence and gains the reason.
///
/// Quick Task's three arms are the reason this exists: "Created the worktree,
/// but couldn't start Claude." says how much of the job got done, and losing
/// that to say why would be a worse screen, not a better one. Two sentences,
/// both this app's.
@Test func aStepThatFailedKeepsItsOwnSentenceAndGainsTheReason() {
    let known = RunnerRefusal.trouble(
        forWord: RunnerRefusal.tmuxUnavailable.rawValue,
        message: "tmux is unavailable",
        after: "Created the worktree, but couldn’t start Claude.")
    #expect(known.sentence.hasPrefix("Created the worktree, but couldn’t start Claude. "))
    #expect(known.sentence.hasSuffix(RunnerRefusal.tmuxUnavailable.sentence))
    #expect(known.transcript == nil)

    // A reason this build cannot read leaves the step standing alone, with the
    // runner's words in the box — which is what this screen showed before.
    let unknown = RunnerRefusal.trouble(
        forWord: "from-the-future",
        message: "tmux is unavailable",
        after: "Created the worktree, but couldn’t start Claude.")
    #expect(unknown.sentence == "Created the worktree, but couldn’t start Claude.")
    #expect(unknown.transcript == "tmux is unavailable")
}

/// The word the runner sent must never be the words a person reads.
@Test func noRefusalSentenceQuotesTheMachineWordBack() {
    for refusal in RunnerRefusal.allCases {
        let sentence = refusal.sentence
        #expect(!sentence.contains(refusal.rawValue), "\(refusal.rawValue) is quoted at the reader")
        // The proto's own spelling, and the Swift case name, are identifiers too.
        #expect(!sentence.contains("ERROR_CODE"))
        #expect(!sentence.contains("\(refusal)"))
        // The core's `Display` text is written for a daemon log. None of it
        // belongs in prose.
        #expect(!sentence.lowercased().contains("resource"))
        #expect(!sentence.contains("/"), "a sentence must never carry a path")
    }
}

/// Fourteen sentences, fourteen different things to do.
///
/// A table whose rows say the same thing is a `switch` with extra steps: the
/// whole reason these are separate codes is that the moves are different — stop
/// what is running, remove the workspaces, install tmux, pick another name.
@Test func eachRefusalSaysSomethingOfItsOwn() {
    var seen = Set<String>()
    for refusal in RunnerRefusal.allCases {
        #expect(seen.insert(refusal.sentence).inserted, "\(refusal.rawValue) reuses a sentence")
    }
}

/// This app's voice: a real sentence, and a curly apostrophe in a contraction.
///
/// Straight apostrophes are the drift the tree was swept for. Nothing else
/// checks these, because they live nowhere a UI test can reach.
@Test func everySentenceIsReadableInThisAppsVoice() {
    for refusal in RunnerRefusal.allCases {
        let sentence = refusal.sentence
        #expect(sentence.count > 30, "\(refusal.rawValue) says too little to act on")
        #expect(sentence.hasSuffix("."), "\(refusal.rawValue) is not a sentence")
        #expect(!sentence.contains("'"), "\(refusal.rawValue) uses a straight apostrophe")
        let first = try! #require(sentence.first)
        #expect(first.isUppercase, "\(refusal.rawValue) does not start a sentence")
    }
}
