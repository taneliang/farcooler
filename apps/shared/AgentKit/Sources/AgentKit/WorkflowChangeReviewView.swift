#if canImport(SwiftUI)
import SwiftUI

/// A workflow change, shown whole, with Approve at the end (ov-978).
///
/// Every command, environment variable, argument and remote the change names is a row of its own, wrapped
/// and never cut off, so a person weighs all of it before the one button that publishes it. Approve is the
/// last thing on the screen, below "In full, exactly as it runs". It sends the words the daemon asked for
/// and nothing else; a row, a notification or a watch can't, because they never show these parts.
public struct WorkflowChangeReviewView: View {
    let review: WorkflowChangeReview
    let mayApprove: Bool
    let sending: Bool
    let failure: String?
    let approve: (String) -> Void
    let decline: (String) -> Void

    public init(review: WorkflowChangeReview, mayApprove: Bool, sending: Bool, failure: String?,
                approve: @escaping (String) -> Void, decline: @escaping (String) -> Void) {
        self.review = review; self.mayApprove = mayApprove; self.sending = sending; self.failure = failure
        self.approve = approve; self.decline = decline
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(review.headline).font(.headline).fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            ForEach(Array(review.intro.enumerated()), id: \.offset) { _, line in inline(line) }
            ForEach(Array(review.parts.enumerated()), id: \.offset) { _, part in
                VStack(alignment: .leading, spacing: 6) {
                    Text(part.title).font(.subheadline.weight(.semibold)).accessibilityAddTraits(.isHeader)
                    ForEach(Array(part.items.enumerated()), id: \.offset) { _, item in
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text("•").foregroundStyle(.quietInk).accessibilityHidden(true)
                            inline(item)
                        }
                    }
                }
            }
            if mayApprove {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Approving publishes exactly what is listed above. Nothing changes until you do.")
                        .font(.footnote).foregroundStyle(.quietInk).fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 12) {
                        Button("Approve") { approve(review.approval) }
                            .buttonStyle(.borderedProminent)
                            .accessibilityIdentifier("workflow-change-approve")
                        Button("Decline") { decline(WorkflowChangeReview.decline) }
                            .buttonStyle(.bordered)
                            .accessibilityIdentifier("workflow-change-decline")
                        if sending { ProgressView().controlSize(.small) }
                    }
                    .disabled(sending)
                }
            } else {
                Text("You can read this change, but only the owner can approve it.")
                    .font(.footnote).foregroundStyle(.quietInk)
            }
            if let failure { Text(failure).font(.footnote).foregroundStyle(.quietInk) }
        }
        .frame(maxWidth: 680, alignment: .leading)
        // Contain, so the buttons keep their own identifiers: a bare identifier on a container
        // is the only one a UI test can find, and hides the children's (ov-991).
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("workflow-change-review")
    }

    /// A line with its `code` spans, wrapped whole.
    private func inline(_ text: String) -> some View {
        Text((try? AttributedString(markdown: text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(text))
            .font(.subheadline)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .textSelection(.enabled)
    }
}
#endif
