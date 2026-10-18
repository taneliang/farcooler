import Foundation

/// Where a member's Claude login stands on a runner (ov-584), as
/// `member.list` and the sign-in calls say it.
public enum ClaudeLoginState: String, Equatable, Sendable {
    /// No login yet, or signed out.
    case none
    case signedIn = "signed_in"
    /// Their agents' turns fail to authenticate (ov-587).
    case stoppedWorking = "stopped_working"
}

/// A member's Claude login: its state, and the account's email when the
/// runner told this device (only the member themself, and the owner, are).
public struct ClaudeLogin: Equatable, Sendable, Decodable {
    public let state: ClaudeLoginState
    public let email: String

    public static let signedOut = ClaudeLogin(state: .none, email: "")

    public init(state: ClaudeLoginState, email: String = "") {
        self.state = state
        self.email = email
    }

    /// A state this build has never heard of reads as no login, and a missing
    /// email as none: never a guess that someone is signed in.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Key.self)
        state = (try? c.decode(String.self, forKey: .state)).flatMap(ClaudeLoginState.init(rawValue:)) ?? .none
        email = ((try? c.decode(String.self, forKey: .email)) ?? "").trimmingCharacters(in: .whitespaces)
    }

    /// `claude.sign_in_finish`'s and `claude.sign_out`'s answer, or nil when
    /// it isn't that.
    public init?(json data: Data) {
        guard let read = try? JSONDecoder().decode(ClaudeLogin.self, from: data) else { return nil }
        self = read
    }

    private enum Key: String, CodingKey { case state, email }
}

/// A member signing in to Claude on a runner from their own device (ov-584):
/// the page the runner hands over and the check on it, and every sentence the
/// sheet and the People section say, so the Mac and the iPhone say the same
/// things and the tests pin them once. Android says them in `ClaudeSignIn.kt`.
public enum ClaudeSignIn {
    // MARK: - The page

    /// The longest page address taken, as the runner's own cap says.
    public static let maxPageLength = 4096

    /// The sign-in pages claude is measured to give, by host and path: a
    /// claude.ai login, and the Console's.
    private static let pages: [(host: String, path: String)] = [
        ("claude.com", "/cai/oauth/authorize"), ("platform.claude.com", "/oauth/authorize"),
    ]

    /// The page in `text`, when it is one this app will open: https, one of
    /// the two hosts at its path with a query, and no user, password, port,
    /// fragment, space, control character or backslash, at most
    /// `maxPageLength` long. The runner has checked it already; a member is
    /// still never sent to an address this app can't vouch for itself, since
    /// what arrives here has crossed a wire.
    public static func page(_ text: String) -> URL? {
        guard text.utf8.count <= maxPageLength, text.hasPrefix("https://"),
            !text.unicodeScalars.contains(where: {
                CharacterSet.controlCharacters.contains($0) || CharacterSet.whitespacesAndNewlines.contains($0)
                    || $0 == "\\" || !$0.isASCII
            }),
            let parts = URLComponents(string: text),
            parts.scheme == "https", parts.user == nil, parts.password == nil, parts.port == nil,
            parts.fragment == nil,
            let host = parts.percentEncodedHost,
            pages.contains(where: { $0.host == host && $0.path == parts.percentEncodedPath }),
            let query = parts.percentEncodedQuery, !query.isEmpty
        else { return nil }
        return parts.url
    }

    // MARK: - The code

    /// Whether `code` is whole as claude takes it: `code#state`, both halves
    /// there once trimmed of the spaces and line break a clipboard brings.
    public static func isWhole(code: String) -> Bool {
        let parts = code.trimmingCharacters(in: .whitespacesAndNewlines).split(
            separator: "#", maxSplits: 1, omittingEmptySubsequences: false)
        return parts.count == 2 && !parts[0].isEmpty && !parts[1].isEmpty
    }

    // MARK: - The Claude section

    public static let sectionTitle = "Claude"
    public static let notSignedInBody =
        "Sign in so your rep and the lanes you start use your own Claude account and its connectors."
    public static let signInButton = "Sign In to Claude…"
    public static let signInAgainButton = "Sign In Again…"
    public static let signOutButton = "Sign Out"
    public static let sheetTitle = "Sign In to Claude"
    public static let cancelButton = "Cancel"
    public static let doneButton = "Done"
    public static let closeButton = "Close"
    public static let openPageButton = "Open Sign-In Page"
    public static let finishButton = "Sign In"
    public static let codeLabel = "Code"
    public static let openPageStep = "Open the sign-in page and approve with the Claude account you want to use."
    public static let codeStep = "Claude shows a code. Copy it, then paste it here."
    public static let finishing = "Signing in…"
    public static let stoppedWorkingLine = "Claude login stopped working"
    public static let notSignedInLine = "Not signed in to Claude"

    /// The row for a signed-in login: "Claude · alex@example.com".
    public static func signedInLine(email: String) -> String {
        email.isEmpty ? "Signed in to Claude" : "Claude · \(email)"
    }

    /// `email` as it is drawn at an accessibility size, where it wraps: a
    /// zero-width space after each `.`, `@` and `-`, so a line breaks at
    /// punctuation and automatic hyphenation has no mid-word break to mark
    /// with a hyphen that reads as part of the address. For display only; a
    /// label or a copy keeps the real email.
    public static func breakable(_ email: String) -> String {
        var out = ""
        for c in email {
            out.append(c)
            if c == "." || c == "@" || c == "-" { out.append("\u{200B}") }
        }
        return out
    }

    public static func starting(runner: String) -> String { "Getting a sign-in link from \(runner)…" }

    /// The words before the email, which the Done step sets on a line of their own.
    public static let signedInAsLead = "Signed in as"

    public static func signedInAs(_ email: String) -> String {
        email.isEmpty ? "Signed in to Claude." : "\(signedInAsLead) \(email)."
    }

    public static func signOutTitle(runner: String) -> String { "Sign out of Claude on \(runner)?" }
    public static let signOutMessage = "Your rep and lanes there stop until you sign in again."

    public static let signOutFailed = "Couldn’t sign out. Try again."

    /// Said beside Open Sign-In Page when nothing took the address.
    public static let pageNotOpened =
        "Couldn’t open the sign-in page. Check that a web browser is available, then try again."

    /// Said under the sign-in button and in the sheet. The lead is per
    /// workspace and this is per runner, so it names no one.
    public static func trustCopy(runner: String) -> String {
        "Your Claude login is kept on \(runner). Its owner, and anyone with Control of it, could use it. "
            + "Far Cooler uses it only for your rep and the lanes you start. "
            + "Lanes the orchestrator starts for your asks use the workspace lead’s login."
    }

    /// What the owner reads where the shared account used to be named.
    public static let ownerTrustLine = "Each member’s agents use that member’s own Claude login."

    /// The longest a code may be, as the runner's own cap says.
    public static let maxCode = 2048
    public static let codeTooLong = "That’s longer than a sign-in code."

    public static let incompleteCode = "That code isn’t complete. Copy the whole code and try again."

    // MARK: - What went wrong

    /// How a sign-in call failed, as an app tells the model: the runner's
    /// stable word and the case it named, or that it never answered.
    public enum CallError: Error, Equatable, Sendable {
        case refused(word: String?, what: String?)
        case unreachable

        /// A failed `farcooler --json` call, from what it printed on standard
        /// error: a refusal carries the runner's `code: <word>` line and its
        /// `what: <case>` line; anything without a `code:` line (ssh not
        /// getting through) is no answer. Nothing on standard error at all is
        /// a call that never ran (the tool missing), which is no answer to
        /// blame on the runner either: a plain failure.
        public init(cliErrors: String) {
            if cliErrors.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                self = .refused(word: nil, what: nil)
                return
            }
            func line(_ prefix: String) -> String? {
                cliErrors.split(whereSeparator: \.isNewline)
                    .map { $0.trimmingCharacters(in: .whitespaces) }
                    .first { $0.hasPrefix(prefix) }
                    .map { String($0.dropFirst(prefix.count)) }
            }
            guard let word = line("code: ") else {
                self = .unreachable
                return
            }
            self = .refused(word: word, what: line("what: "))
        }
    }

    /// What a person can do about a failure, as the button's title.
    public enum Retry: String, Equatable, Sendable {
        case tryAgain = "Try Again"
        case startAgain = "Start Again"
    }

    /// A sign-in that ended badly: the sentence, and the one move that helps.
    public struct Failure: Equatable, Sendable {
        public let message: String
        public let retry: Retry?

        public init(_ message: String, retry: Retry? = nil) {
            self.message = message
            self.retry = retry
        }
    }

    /// Which call failed: an expired link is news only about a finish.
    public enum Phase: Sendable { case start, finish }

    /// The sentence for `error` on `runner`. The runner's words are never
    /// shown. Only a word this build knows has a sentence of its own;
    /// "isn't answering" is for a link that never answered; the rest read as
    /// a plain "try again".
    public static func failure(for error: CallError, during phase: Phase, runner: String) -> Failure {
        switch error {
        case .unreachable:
            return Failure("\(runner) isn’t answering. Check your connection and try again.", retry: .tryAgain)
        case .refused(let word, let what):
            switch (what, word) {
            case ("code_refused", _):
                return Failure("That code didn’t work. Open the sign-in page again to get a new one.", retry: .tryAgain)
            case ("sign_in_code", _):
                return Failure(incompleteCode)
            case ("sign_in_too_old", _), ("sign_in_unreadable", _), (_, "capability-unsupported"?):
                return tooOld(runner: runner)
            case ("sign_in_too_often", _):
                return Failure("Too many sign-in attempts. Wait a minute, then try again.")
            case (_, "not-found"?) where phase == .finish:
                return Failure("The sign-in link expired.", retry: .startAgain)
            default:
                return Failure("Couldn’t sign in. Try again.", retry: .tryAgain)
            }
        }
    }

    static func tooOld(runner: String) -> Failure {
        Failure("Couldn’t start sign-in on \(runner). Claude Code there needs to be updated.")
    }

    static let expired = Failure("The sign-in link expired.", retry: .startAgain)
}
