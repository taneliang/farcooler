import Foundation

/// A workflow change, read back out of the question the daemon asks a person (ov-978).
///
/// The daemon writes the question: a headline, the whole change in labeled parts, and last the words that
/// approve it, ``answer `approve <code>` ``. A Review screen shows every part, then Approve at the end,
/// which sends exactly those words. Anything that doesn't have this shape returns nil, so a question that
/// isn't a change (or a change from a newer daemon) is shown as the plain card it always was and never
/// gains an Approve button from a guess.
///
/// The shape is pinned by `crates/client/src/testdata/workflow-change-question.txt`, written by the daemon's
/// own producer; the Android twin is `WorkflowChangeReview.kt`.
public struct WorkflowChangeReview: Equatable, Sendable {
    public struct Part: Equatable, Sendable {
        public let title: String
        public let items: [String]
    }

    /// The one sentence a row shows.
    public let headline: String
    /// What comes before the parts: the orchestrator's reason, and what this replaces.
    public let intro: [String]
    /// Every part the daemon wrote, in its order.
    public let parts: [Part]
    /// The closing paragraph, as written.
    public let closing: String
    let code: String

    /// The headings the daemon writes, in order. A question missing one, or with another, isn't read.
    static let headings = ["What runs as you:", "Checks that must pass:", "How code lands:", "Who approves:", "In full, exactly as it runs:"]
    public static let decline = "decline"

    /// What Approve sends.
    public var approval: String { "approve \(code)" }

    public static func parse(_ question: String) -> WorkflowChangeReview? {
        let paragraphs = question.components(separatedBy: "\n\n")
        guard paragraphs.count >= 3, let headline = paragraphs.first, headline.hasPrefix("Publish this workflow?"),
              let closing = paragraphs.last,
              let range = closing.range(of: #"answer `approve ([0-9a-f]{6})`"#, options: .regularExpression)
        else { return nil }
        let code = String(closing[range].dropFirst("answer `approve ".count).dropLast())
        guard closing.contains("answer `decline`") else { return nil }
        var intro: [String] = []
        var parts: [Part] = []
        for paragraph in paragraphs.dropFirst().dropLast() {
            let lines = paragraph.components(separatedBy: "\n")
            if let title = lines.first, headings.contains(title) {
                let items = lines.dropFirst().map { $0.hasPrefix("- ") ? String($0.dropFirst(2)) : nil }
                guard !items.isEmpty, !items.contains(where: { $0 == nil }) else { return nil }
                parts.append(Part(title: String(title.dropLast()), items: items.compactMap { $0 }))
            } else if parts.isEmpty {
                intro.append(paragraph)
            } else {
                return nil
            }
        }
        guard parts.map({ $0.title + ":" }) == headings else { return nil }
        return WorkflowChangeReview(headline: headline, intro: intro, parts: parts, closing: closing, code: code)
    }
}
