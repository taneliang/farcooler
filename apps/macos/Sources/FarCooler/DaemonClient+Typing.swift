import Foundation

/// How a task's or a prompt's text is typed into an agent's box: `typeWhenIdle`
/// decides when, this says how.
extension DaemonClient {
    /// `send`, saying whether the runner took both the text and the Return,
    /// or, for an if-empty send, which word it refused with.
    /// Through `runRaw`, so a refusal is this path's to report rather than
    /// the failure of whichever action it runs inside.
    ///
    /// With `onlyIfEmpty`, one call does both and the runner checks the box
    /// right before it types (`typeArguments`). Its refusal word is the `what:`
    /// line of the CLI's stderr under `--json`, never the sentence.
    func type(_ text: String, into terminal: String, onlyIfEmpty: Bool = false) async -> Typed {
        let (typed, message) = await runRaw(Self.typeArguments(text, into: terminal, onlyIfEmpty: onlyIfEmpty), background: true)
        guard typed != nil else { return .refused(what: onlyIfEmpty ? TaskFailure.what(in: message) : nil) }
        if onlyIfEmpty { return .sent }
        let (entered, _) = await runRaw(["terminal", "send-hex", terminal, "0d"], background: true)
        return entered != nil ? .sent : .refused(what: nil)
    }

    /// How a typing ended.
    enum Typed: Equatable {
        case sent
        /// The runner refused, with the word it named when it named one.
        case refused(what: String?)
    }

    /// Why an if-empty send the runner refused with `what` didn't go, as the
    /// cause its sentence is chosen by (`SuggestThemes.undelivered`).
    static func cause(ofRefusal what: String?) -> TaskFailure.Undelivered {
        switch what {
        case "draft": return .notEmpty
        case "prompt", "dialog": return .askedFirst
        // Typed, and it may have been taken: not a failure to try again.
        case "unconfirmed", "paste_left": return .mayHaveTyped
        default: return .notTyped
        }
    }

    /// The `terminal send` that types `text`. With `--if-empty`, `--json`
    /// for the refusal's word, and a `--` before the positionals, so a text
    /// that starts with a dash is data.
    static func typeArguments(_ text: String, into terminal: String, onlyIfEmpty: Bool) -> [String] {
        onlyIfEmpty
            ? ["terminal", "send", "--json", "--if-empty", "--", terminal, text]
            : ["terminal", "send", terminal, text]
    }
}
