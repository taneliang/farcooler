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

// MARK: - A lock-screen tap on a closed hook ask (ov-57)

/// The sentences a card says when a hook ask is closed.
///
/// **Provisional (ov-57), D4**, and in one place so the owner's answer is a
/// change here and nowhere else. The T0 contract's C5 table.
public enum ClosedAskWording {
    /// The hold is over, or the verdict never reached the hook: the dialog may
    /// still be up at the keyboard.
    public static let tooLate = "Too late here. Answer it in the terminal."
    /// Refused as no longer held while the hold was still running: another
    /// device won.
    public static let answeredElsewhere = "Answered on another device."
    /// Refused as no longer held, with no hold end to tell the two causes
    /// apart by.
    public static let eitherCause = "Answered elsewhere, or it timed out."
}

extension GlanceAnswer {
    /// How a closed ask settles, and what the card says about it.
    public struct Closing: Equatable, Sendable {
        public let outcome: Outcome
        public let message: String
    }

    /// A tap refused on the phone, before any connection, because the hold is
    /// over at `now`; nil when the tap may go out. Nothing is sent, and the
    /// buttons stay off: there is nothing left to answer from here.
    ///
    /// No `until` (an ask the app filed, not the card) is never refused here;
    /// the daemon decides.
    public static func refusedHere(until: Date?, now: Date) -> Closing? {
        guard let until, now >= until else { return nil }
        return Closing(outcome: .over, message: ClosedAskWording.tooLate)
    }

    /// The runner's refusal of an answer to a HOOK ask, read as the ask being
    /// closed; nil for any other refusal, which keeps its existing reading.
    ///
    /// The daemon refuses both `not_held` (answered, withdrawn, or the hold ran
    /// out) and `not_delivered` (the verdict never reached the hook) with the
    /// word `resource-conflict`, and says which in the message
    /// (`crates/daemon/src/rpc.rs`, `terminal.agent_answer`). Both are `.over`.
    /// Which sentence depends on the cause and, for `not_held`, on whether the
    /// hold had run out when the tap was made.
    public static func closing(
        request: String, word: String?, message: String, until: Date?, now: Date
    ) -> Closing? {
        guard request.hasPrefix(CardAsk.idPrefix),
            word == RunnerRefusal.resourceConflict.rawValue
        else { return nil }
        let sentence: String
        if message.contains("not_delivered") {
            sentence = ClosedAskWording.tooLate
        } else if let until {
            sentence = now >= until ? ClosedAskWording.tooLate : ClosedAskWording.answeredElsewhere
        } else {
            sentence = ClosedAskWording.eitherCause
        }
        return Closing(outcome: .over, message: sentence)
    }
}

/// What a lock-screen tap does first (ov-57 T-iOS-2).
public enum GlanceTapRoute: Equatable, Sendable {
    /// Settle without connecting: nothing can be sent.
    case refuse(GlanceAnswer.Closing)
    /// Connect and send. `verifyFirst` replays the pane's stream to check the
    /// request is still the one pending, which a hook ask skips: the daemon
    /// refuses a stale `hook-ask-` id itself, under the ledger's lock, so the
    /// replay would only spend up to seven of the hold's seconds re-proving it.
    case send(verifyFirst: Bool)

    public static func route(request: String, until: Date?, now: Date) -> GlanceTapRoute {
        if let refused = GlanceAnswer.refusedHere(until: until, now: now) { return .refuse(refused) }
        return .send(verifyFirst: !request.hasPrefix(CardAsk.idPrefix))
    }
}
