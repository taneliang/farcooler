import Foundation

/// One pane's answer to its agent's permission ask, while it is sent and
/// after, for the card that offers it.
///
/// **The card stays up until the runner takes the answer.** It used to come
/// down on the tap, with `terminal.agent_answer`'s error dropped. For a claude
/// TUI ask the daemon holds the hook until an answer lands, so a refused
/// answer left the ask held on the runner and gone from the phone for good:
/// its `Permission` was behind the stream's cursor and nothing else would show
/// it again. Now the buttons are off while the answer is out, and a failure
/// leaves the card up with a sentence saying so.
///
/// **One refusal is not a failure.** `resource-conflict` is what the daemon
/// says for an ask nothing holds any more: answered at the keyboard, from the
/// watch or another phone, or withdrawn when its hold ran out. The ask is
/// over, so the card comes down without a word.
public struct PermissionAnswering: Equatable, Sendable {
    /// The request whose answer is out, if one is.
    public private(set) var sending: String?

    /// The last answer that did not land, and what to say about it.
    public private(set) var failure: Failure?

    public struct Failure: Equatable, Sendable {
        public let request: String
        public let sentence: String
    }

    /// How one answer ended.
    public enum Outcome: Equatable, Sendable {
        /// The runner took it.
        case sent
        /// Nothing holds that ask any more: it was answered somewhere else,
        /// or it ended.
        case answeredElsewhere
        /// It did not land, and the ask may still be waiting.
        case failed(String)
    }

    public init() {}

    /// Take ownership of one answer, or refuse while another is out.
    public mutating func begin(_ request: String) -> Bool {
        guard sending == nil else { return false }
        sending = request
        failure = nil
        return true
    }

    /// Say how the answer to `request` ended. Returns whether the card for it
    /// comes down.
    public mutating func finish(_ request: String, _ outcome: Outcome) -> Bool {
        if sending == request { sending = nil }
        switch outcome {
        case .sent, .answeredElsewhere:
            if failure?.request == request { failure = nil }
            return true
        case let .failed(sentence):
            failure = Failure(request: request, sentence: sentence)
            return false
        }
    }

    /// What to say under the card for `request`, if its last answer failed.
    public func sentence(for request: String) -> String? {
        failure?.request == request ? failure?.sentence : nil
    }

    /// The outcome of an answer the runner did not take, from the word it
    /// refused with. Nil for no word at all: a dropped link or a timeout,
    /// where the answer may or may not have landed.
    ///
    /// Trying again is safe in every case: a second answer to a hook ask that
    /// the first one settled is refused as `resource-conflict`, which clears
    /// the card.
    public static func outcome(refusedWith word: String?) -> Outcome {
        if word == RunnerRefusal.resourceConflict.rawValue { return .answeredElsewhere }
        guard let word, !word.isEmpty else {
            return .failed("Your answer may not have reached the runner. Try again.")
        }
        return .failed(
            RunnerRefusal.trouble(
                forWord: word, message: "", after: "The runner didn’t take your answer."
            ).sentence)
    }
}
