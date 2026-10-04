import Foundation

/// Typed input the runner did not take, held so it can be sent again.
///
/// A terminal write used to be `try?`: a keystroke the runner never answered
/// was simply gone, and the screen looked the same as when it worked (ov-238).
/// This is the value that makes the failure visible. The phone keeps the bytes,
/// says in one quiet line why they are waiting, and offers Try Again, which
/// sends them first and in order.
///
/// A value in AgentKit rather than state in the terminal view, because `apps/ios`
/// has no unit tests CI runs and the rule — what is held, in what order, and
/// what the line says — is the part worth reading back.
public struct UnsentInput: Equatable, Sendable {
    /// Why the runner did not take it, in the three ways a phone can tell apart.
    public enum Why: Equatable, Sendable {
        /// No answer by the call's deadline.
        case timedOut
        /// The link is gone.
        case disconnected
        /// Anything else: a refusal, or an answer this build couldn't read.
        case other
    }

    /// Everything not yet delivered, oldest first.
    public private(set) var bytes: [UInt8]
    public private(set) var why: Why

    public init(bytes: [UInt8], why: Why) {
        self.bytes = bytes
        self.why = why
    }

    /// The reason for a failed `terminal.write`, from what the answer line said.
    ///
    /// `word` is `RunnerRefusal.word(inAnswerLine:)`, so a call that passed its
    /// deadline arrives as `RunnerRefusal.timedOutWord`.
    public static func why(word: String?, disconnected: Bool) -> Why {
        if disconnected { return .disconnected }
        return word == RunnerRefusal.timedOutWord ? .timedOut : .other
    }

    /// What to send now: whatever is held, then the new bytes. The order they
    /// were typed in, whether or not the last attempt failed.
    public static func bytesToSend(after held: UnsentInput?, adding new: [UInt8]) -> [UInt8] {
        (held?.bytes ?? []) + new
    }

    /// This, with `more` held behind it and the latest reason.
    public func holding(_ more: [UInt8], why latest: Why) -> UnsentInput {
        UnsentInput(bytes: bytes + more, why: latest)
    }

    /// The one line a screen shows.
    ///
    /// It says why, then that nothing was lost. It never quotes the core's own
    /// words, and it claims no cause it doesn't know.
    public var sentence: String {
        switch why {
        case .timedOut: "The runner took too long to answer. Your typing is waiting."
        case .disconnected: "Far Cooler lost the connection to this runner. Your typing is waiting."
        case .other: "The runner didn’t take that. Your typing is waiting."
        }
    }

    /// The button beside it.
    public static let retryTitle = "Try Again"
}
