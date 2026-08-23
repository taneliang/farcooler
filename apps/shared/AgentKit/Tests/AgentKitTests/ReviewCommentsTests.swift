import Foundation
import Testing

@testable import AgentKit

/// The queue two apps now share, and the four claims its doc comments make.
///
/// Nothing here draws anything. What is tested is what would be invisible until
/// it bit somebody: the message an agent actually receives, that a failed send
/// keeps the notes, that the queue survives the process, and that a receipt
/// written before `placedInComposer` existed still decodes — which is the
/// claim that, if wrong, silently empties a reviewer's queue on upgrade.
@MainActor
struct ReviewCommentQueueTests {
    /// A defaults suite of its own per test. `UserDefaults.standard` here would
    /// leave review notes behind in whichever app ran the suite.
    private func scratch() -> (UserDefaults, String) {
        let name = "farcooler.tests.\(UUID().uuidString)"
        return (UserDefaults(suiteName: name)!, name)
    }

    private func queue(
        _ defaults: UserDefaults,
        deliver: @escaping ReviewCommentQueue.Deliver = { _, _ in nil }
    ) -> ReviewCommentQueue {
        ReviewCommentQueue(workspace: "ws", defaults: defaults, deliver: deliver)
    }

    private func note(_ text: String, _ anchor: ReviewAnchor) -> ReviewComment {
        ReviewComment(anchor: anchor, text: text)
    }

    // MARK: The message

    @Test func theMessageIsNumberedInReadingOrder() {
        let (defaults, _) = scratch()
        let q = queue(defaults)
        q.add(note("Handle 429 as well", ReviewAnchor(file: "push.ts", firstLine: 120, lastLine: 148)))
        q.add(note("This can be one query", ReviewAnchor(file: "db.rs", firstLine: 12, lastLine: 12)))

        #expect(
            q.message(branch: "feature") == """
                Review notes on `feature` from Far Cooler (2):

                1. In `push.ts`, around lines 120-148:
                   Handle 429 as well

                2. In `db.rs`, around line 12:
                   This can be one query
                """)
    }

    /// A line-level anchor reads as one line rather than as a range of one,
    /// which is the whole reason a pointer's anchor beats a thumb's.
    @Test func aSingleLineAnchorNamesTheLine() {
        let anchor = ReviewAnchor(file: "a.swift", firstLine: 41, lastLine: 41)
        #expect(anchor.placeDescription == "line 41")
        #expect(anchor.promptDescription == "`a.swift`, around line 41")
    }

    @Test func aFileAnchorClaimsNoLines() {
        let anchor = ReviewAnchor(file: "a.swift", commit: "0123456789abcdef")
        #expect(anchor.placeDescription == "the whole file")
        #expect(anchor.promptDescription == "`a.swift` (commit 01234567)")
    }

    @Test func aQuoteIsCutRatherThanSentWhole() {
        let long = String(repeating: "x", count: 400)
        let quote = ReviewAnchor.quoting("   \(long)   ")
        #expect(quote?.count == 121)
        #expect(quote?.hasSuffix("…") == true)
        #expect(ReviewAnchor.quoting("   ") == nil)
    }

    @Test func aQuotedLineTravelsWithTheNote() {
        let (defaults, _) = scratch()
        let q = queue(defaults)
        q.add(
            note(
                "Why?",
                ReviewAnchor(file: "a.swift", firstLine: 3, lastLine: 3, quote: "let x = 1")))
        #expect(q.message(branch: "").contains("   > let x = 1"))
        // No branch, no backticked branch name in the header.
        #expect(q.message(branch: "").hasPrefix("Review note from Far Cooler (1):"))
    }

    // MARK: Sending

    @Test func aSentBatchLeavesTheQueueAndLeavesAReceipt() async {
        let (defaults, _) = scratch()
        var handed: [String] = []
        let q = queue(defaults) { _, text in
            handed.append(text)
            return nil
        }
        q.add(note("Do the thing", ReviewAnchor(file: "a.swift")))
        await q.send(to: ReviewAgentTarget(id: "%3", name: "claude"), branch: "feature")

        #expect(handed.count == 1)
        #expect(q.pending.isEmpty)
        #expect(q.sent.count == 1)
        #expect(q.sent.first?.count == 1)
        #expect(q.sent.first?.agentName == "claude")
        #expect(q.sent.first?.text == handed.first)
        #expect(q.sent.first?.placedInComposer == nil)
        #expect(q.failure == nil)
    }

    /// The claim `send`'s doc comment makes: a failure here means "this client
    /// did not get an answer", so the notes stay and nothing retries.
    @Test func aFailedSendKeepsTheNotesAndSaysWhy() async {
        let (defaults, _) = scratch()
        var attempts = 0
        let q = queue(defaults) { _, _ in
            attempts += 1
            return ReviewTrouble(sentence: "The connection dropped.", transcript: "exit 1")
        }
        q.add(note("Do the thing", ReviewAnchor(file: "a.swift")))
        await q.send(to: ReviewAgentTarget(id: "%3", name: "claude"), branch: "feature")

        #expect(attempts == 1)
        #expect(q.pending.count == 1)
        #expect(q.sent.isEmpty)
        #expect(q.failure?.sentence == "The connection dropped.")
        #expect(q.failure?.transcript == "exit 1")

        // Nothing is retried on its own — a second attempt happens only because
        // this line is a person pressing Try Again.
        await q.send(to: ReviewAgentTarget(id: "%3", name: "claude"), branch: "feature")
        #expect(attempts == 2)
    }

    @Test func writingANoteClearsAFailureItNoLongerDescribes() async {
        let (defaults, _) = scratch()
        let q = queue(defaults) { _, _ in ReviewTrouble(sentence: "No.") }
        q.add(note("One", ReviewAnchor(file: "a.swift")))
        await q.send(to: ReviewAgentTarget(id: "%3", name: "claude"), branch: "")
        #expect(q.failure != nil)
        q.add(note("Two", ReviewAnchor(file: "b.swift")))
        #expect(q.failure == nil)
    }

    @Test func anEmptyQueueSendsNothing() async {
        let (defaults, _) = scratch()
        var attempts = 0
        let q = queue(defaults) { _, _ in
            attempts += 1
            return nil
        }
        await q.send(to: ReviewAgentTarget(id: "%3", name: "claude"), branch: "feature")
        #expect(attempts == 0)
        #expect(q.sent.isEmpty)
    }

    @Test func onlyTheLastFiveReceiptsAreKept() async {
        let (defaults, _) = scratch()
        let q = queue(defaults)
        for i in 1...7 {
            q.add(note("note \(i)", ReviewAnchor(file: "a.swift")))
            await q.send(to: ReviewAgentTarget(id: "%3", name: "claude"), branch: "")
        }
        #expect(q.sent.count == 5)
        // Newest first.
        #expect(q.sent.first?.text.contains("note 7") == true)
    }

    // MARK: Put in composer

    @Test func puttingNotesInAComposerEmptiesTheQueueWithoutSending() {
        let (defaults, _) = scratch()
        var attempts = 0
        let q = queue(defaults) { _, _ in
            attempts += 1
            return nil
        }
        q.add(note("Handle 429", ReviewAnchor(file: "push.ts", firstLine: 9, lastLine: 9)))
        let text = q.putInComposer(
            ReviewAgentTarget(id: "%3", name: "claude", showsChat: true), branch: "feature")

        #expect(attempts == 0)
        #expect(text?.contains("Handle 429") == true)
        #expect(q.pending.isEmpty)
        #expect(q.sent.first?.placedInComposer == true)
        #expect(q.sent.first?.text == text)
    }

    @Test func thereIsNothingToPutInAComposerWhenNothingIsWritten() {
        let (defaults, _) = scratch()
        let q = queue(defaults)
        #expect(q.putInComposer(ReviewAgentTarget(id: "%3", name: "claude"), branch: "") == nil)
        #expect(q.sent.isEmpty)
    }

    // MARK: Storage

    @Test func theQueueSurvivesTheProcessThatWroteIt() async {
        let (defaults, _) = scratch()
        let first = queue(defaults)
        first.add(note("Unsent", ReviewAnchor(file: "a.swift", firstLine: 2, lastLine: 2)))
        first.add(note("Also unsent", ReviewAnchor(file: "b.swift")))

        let second = queue(defaults)
        #expect(second.pending.count == 2)
        #expect(second.pending.first?.text == "Unsent")
        #expect(second.pending.first?.anchor.firstLine == 2)

        second.remove(second.pending[0])
        #expect(queue(defaults).pending.count == 1)
    }

    /// The compatibility claim on `SentReviewBatch.placedInComposer`. A receipt
    /// written by a build that predates the field has no key for it, and a
    /// non-optional field would fail the decode of `Stored` — taking the
    /// PENDING notes with it, which are the only thing on this screen a person
    /// typed.
    @Test func aReceiptWrittenBeforeTheComposerExistedStillDecodes() throws {
        let (defaults, _) = scratch()
        let stored = """
            {"pending":[{"id":"\(UUID().uuidString)",
                         "anchor":{"file":"a.swift"},
                         "text":"Still here","writtenAt":1750000000}],
             "sent":[{"id":"\(UUID().uuidString)","text":"Review note (1):",
                      "agentName":"claude","sentAt":1750000000,"count":1}]}
            """
        defaults.set(Data(stored.utf8), forKey: "changes.comments.ws")

        let q = queue(defaults)
        #expect(q.pending.count == 1)
        #expect(q.pending.first?.text == "Still here")
        #expect(q.sent.count == 1)
        #expect(q.sent.first?.placedInComposer == nil)
    }

    /// Reading a queue back must not immediately write it out again — the
    /// `didSet`s fire during `init`, and `loading` is what stops the round trip.
    @Test func loadingAQueueDoesNotRewriteIt() {
        let (defaults, _) = scratch()
        let first = queue(defaults)
        first.add(note("One", ReviewAnchor(file: "a.swift")))
        let written = defaults.data(forKey: "changes.comments.ws")

        _ = queue(defaults)
        #expect(defaults.data(forKey: "changes.comments.ws") == written)
    }
}
