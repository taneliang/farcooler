import Foundation
import Testing
@testable import AgentKit

/// The Review screen reads the question the daemon's own producer wrote (ov-978).
struct WorkflowChangeReviewTests {
    private func real() throws -> String {
        let here = URL(fileURLWithPath: #filePath)
        let root = (0..<6).reduce(here) { url, _ in url.deletingLastPathComponent() }
        return try String(contentsOf: root.appendingPathComponent("crates/client/src/testdata/workflow-change-question.txt"), encoding: .utf8)
    }

    @Test func theRealQuestionIsReadInFull() throws {
        let review = try #require(WorkflowChangeReview.parse(try real()))
        #expect(review.headline == "Publish this workflow? 4 checks must pass, each ticket opens a pull request, you approve each ticket.")
        #expect(review.intro.count == 2 && review.intro[0].hasPrefix("The orchestrator’s reason:"))
        #expect(review.parts.map(\.title) == ["What runs as you", "Checks that must pass", "How code lands", "Who approves", "In full, exactly as it runs"])
        #expect(review.approval == "approve a1b2c3")
        // Every line of every part survives: nothing is dropped between the producer and the screen.
        let bullets = try real().components(separatedBy: "\n").filter { $0.hasPrefix("- ") }
        #expect(review.parts.flatMap(\.items).count == bullets.count && bullets.count >= 20)
        #expect(review.parts.flatMap(\.items) == bullets.map { String($0.dropFirst(2)) })
    }

    @Test func theRecordSaysTheHeadlineOnceTheReviewShowsTheRest() throws {
        let line = try #require(WorkflowChangeReview.recordLine(of: try real()))
        #expect(line.hasPrefix("Publish this workflow? 4 checks must pass"))
        #expect(!line.contains("What runs as you") && line.count < 200)
        #expect(WorkflowChangeReview.recordLine(of: "Which branch?") == nil)
    }

    @Test func aQuestionThatIsNotAChangeIsNeverGivenAnApproveButton() throws {
        let text = try real()
        #expect(WorkflowChangeReview.parse("Which branch should I use?") == nil)
        // A missing part, a part out of order, a stray paragraph and a missing code each fail closed.
        #expect(WorkflowChangeReview.parse(text.replacingOccurrences(of: "Who approves:\n- You approve each ticket before it lands.\n\n", with: "")) == nil)
        #expect(WorkflowChangeReview.parse(text.replacingOccurrences(of: "How code lands:", with: "How it lands:")) == nil)
        #expect(WorkflowChangeReview.parse(text.replacingOccurrences(of: "\n\nTo publish", with: "\n\nAnd also this.\n\nTo publish")) == nil)
        #expect(WorkflowChangeReview.parse(text.replacingOccurrences(of: "approve a1b2c3", with: "approve now")) == nil)
        #expect(WorkflowChangeReview.parse(text.replacingOccurrences(of: "answer `decline`", with: "answer `no`")) == nil)
    }
}

/// Approve is drawn only for who the daemon would let approve (ov-991).
struct WorkflowChangeApproveTests {
    @Test func onlyTheOwnerWithWriteAccessIsShownApprove() {
        #expect(WorkflowChangeReview.mayApprove(mayAct: true, viewer: ""), "the owner")
        #expect(WorkflowChangeReview.mayApprove(mayAct: true, viewer: nil), "not told yet, as Keep")
        #expect(!WorkflowChangeReview.mayApprove(mayAct: true, viewer: "bob"), "a member with write access")
        #expect(!WorkflowChangeReview.mayApprove(mayAct: true, viewer: "(released)"), "a released device")
        #expect(!WorkflowChangeReview.mayApprove(mayAct: false, viewer: ""), "an owner's Read grant")
    }
}
