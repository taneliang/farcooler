import Foundation

/// Starting a task went wrong: what the panel says about it.
///
/// **The runner's words are not the sentence, and not the key either.** The
/// CLI prints the daemon's `Display` text ("error: branch already exists"),
/// written for a log and free to be reworded. Under `--json` it also prints
/// `code: <word>` — the stable machine word of `farcooler_core::error::word`
/// — and that is what this switches on. Anything without a word it knows gets
/// a plain sentence of its own. The draft stays in the panel either way, so
/// nothing the person typed is lost to a failure this cannot name.
///
enum TaskFailure {
    /// The word on a `code:` line of the CLI's stderr, if there is one.
    static func code(in message: String?) -> String? {
        (message ?? "").split(whereSeparator: \.isNewline)
            .lazy
            .compactMap { line -> String? in
                let line = line.trimmingCharacters(in: .whitespaces)
                guard line.hasPrefix("code: ") else { return nil }
                return String(line.dropFirst("code: ".count))
            }
            .first
    }

    /// The word on a `what:` line of the CLI's stderr, if there is one: which
    /// argument or conflict the runner named (`not_held`, `not_delivered`).
    static func what(in message: String?) -> String? {
        (message ?? "").split(whereSeparator: \.isNewline)
            .lazy
            .compactMap { line -> String? in
                let line = line.trimmingCharacters(in: .whitespaces)
                guard line.hasPrefix("what: ") else { return nil }
                return String(line.dropFirst("what: ".count))
            }
            .first
    }

    /// Why a task that was to be typed in never was.
    enum Undelivered {
        /// The agent asked something first: a trust screen, a resume dialog.
        case askedFirst
        /// Not ready within the minute.
        case neverReady
        /// Its terminal went away.
        case gone
        /// The runner didn't take the keystrokes.
        case notTyped
        /// An if-empty send found text in the agent's box (ov-557).
        case notEmpty
        /// An if-empty send was typed, and the agent may have taken it: it
        /// wasn't confirmed, or was left in the box.
        case mayHaveTyped
    }

    /// A task that was to be typed into its agent and wasn't. It's on the
    /// clipboard by then, and each sentence says what to do with it for its
    /// own cause: answer first, wait, or start one.
    static func undelivered(name: String, _ cause: Undelivered) -> String {
        let task = "“\(WorktreeName.display(name))”"
        switch cause {
        case .askedFirst:
            return "The agent in \(task) asked a question first, so what you typed is on the clipboard. Paste it in once you’ve answered."
        case .neverReady:
            return "The agent in \(task) wasn’t ready after a minute. What you typed is on the clipboard, so you can paste it in when the agent is ready."
        case .gone:
            return "The agent in \(task) closed before it got what you typed. It’s on the clipboard, so you can start another and paste it in."
        case .notTyped:
            return "Couldn’t type what you asked for into the agent in \(task). It’s on the clipboard, so you can paste it in yourself."
        case .notEmpty:
            return "The agent in \(task) had something typed in its box, so what you asked for is on the clipboard. Paste it in once the box is clear."
        case .mayHaveTyped:
            return "What you asked for may have reached the agent in \(task). Check its terminal before you paste it in again; it’s on the clipboard."
        }
    }

    static func sentence(for message: String?) -> String {
        let context = "Couldn’t start the agent on this runner."
        // The one word with a sentence of its own: the start panel picks the
        // name itself, so "pick another name" would send a person looking for
        // a field that isn't there, and starting again takes the next one.
        if ["branch-exists", "worktree-exists"].contains(code(in: message)) {
            return "This runner already has a branch or folder with that name. Start again to use a different one."
        }
        if let shared = RefusalCopy.sentence(after: context, message) { return shared }
        // A word this build has no sentence for is a newer runner's: it
        // answered, so it is not said to be unreachable.
        if code(in: message) != nil { return "\(context) Try again." }
        return "\(context) Check that it’s reachable, then try again."
    }
}
