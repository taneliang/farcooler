import AgentKit
import SwiftUI

/// A workflow change on a task's card: all of it, then Approve at the end (ov-978).
///
/// Sends the words the daemon asked for through the card's own answer path, so it is an ordinary answer
/// by the person; nothing else on the Mac approves a change.
struct WorkflowChangeReviewCard: View {
    let review: WorkflowChangeReview
    let canAnswer: Bool
    /// `RunnerPeople.you`: only the owner's Approve counts (ov-991).
    var viewer: String?
    let onAnswer: (String) async -> Bool
    @State private var sending = false
    @State private var failure: String?

    var body: some View {
        WorkflowChangeReviewView(
            review: review, mayApprove: WorkflowChangeReview.mayApprove(mayAct: canAnswer, viewer: viewer), sending: sending, failure: failure,
            approve: send, decline: send)
    }

    private func send(_ words: String) {
        guard !sending else { return }
        sending = true
        failure = nil
        Task {
            let sent = await onAnswer(words)
            sending = false
            if !sent { failure = "Couldn’t send that. Try again." }
        }
    }
}
