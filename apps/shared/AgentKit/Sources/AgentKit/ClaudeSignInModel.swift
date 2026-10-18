import Combine
import Foundation

/// The sign-in sheet's steps (ov-584), one model for the Mac and the iPhone.
/// The sheet draws `step`; this decides what comes next and what is said.
///
/// The calls are the app's: the phone's go through the client core, the
/// Mac's through the CLI. Each throws `ClaudeSignIn.CallError`, which is all
/// the runner's refusals and silences this has to tell apart.
@MainActor
public final class ClaudeSignInModel: ObservableObject {
    /// The page the runner handed over, and when it stops working.
    public struct Started: Equatable, Sendable {
        public let session: String
        public let url: String
        public let expiresAt: Date?

        public init(session: String, url: String, expiresAt: Date? = nil) {
            self.session = session
            self.url = url
            self.expiresAt = expiresAt
        }

        /// `claude.sign_in`'s answer: `{session, url, expires_at}`, the
        /// expiry in Unix milliseconds.
        public init?(json data: Data) {
            guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let session = object["session"] as? String, !session.isEmpty,
                let url = object["url"] as? String
            else { return nil }
            self.init(
                session: session, url: url,
                expiresAt: (object["expires_at"] as? NSNumber).map { Date(timeIntervalSince1970: $0.doubleValue / 1000) })
        }
    }

    /// How this app reaches the runner.
    public struct Calls: Sendable {
        public var start: @Sendable (_ handle: String) async throws -> Started
        public var finish: @Sendable (_ session: String, _ code: String) async throws -> ClaudeLogin
        public var cancel: @Sendable (_ session: String) async -> Void

        public init(
            start: @escaping @Sendable (String) async throws -> Started,
            finish: @escaping @Sendable (String, String) async throws -> ClaudeLogin,
            cancel: @escaping @Sendable (String) async -> Void
        ) {
            self.start = start
            self.finish = finish
            self.cancel = cancel
        }
    }

    public enum Step: Equatable {
        /// Asking the runner for a link.
        case starting
        /// The link is here: open it, then paste the code it shows.
        case page(URL)
        /// The code is with the runner.
        case finishing
        /// Signed in.
        case done(email: String)
        case failed(ClaudeSignIn.Failure)
    }

    @Published public private(set) var step: Step = .starting
    /// What is in the code field.
    @Published public var code = "" {
        didSet {
            guard code != oldValue else { return }
            // Longer than any code: the paste is refused and the field stays
            // as it was, with supporting text rather than an error.
            if code.utf8.count > ClaudeSignIn.maxCode {
                code = oldValue
                codeNotice = ClaudeSignIn.codeTooLong
                return
            }
            // A notice about the old text is no news about the new.
            codeNotice = nil
        }
    }
    /// Said beside the field when the code isn't whole; the sign-in goes on.
    @Published public private(set) var codeNotice: String?
    /// Said beside Open Sign-In Page when no browser took the page.
    @Published public private(set) var openNotice: String?

    public let handle: String
    public let runner: String
    private let calls: Calls
    private let now: () -> Date
    private var session: String?
    private var expiresAt: Date?
    /// The sheet went away: a link that arrives after is cancelled.
    private var closed = false
    private var began = false
    /// Bumped by every start and by closing, so a reply to a call made
    /// before that finds the model has moved on and is dropped.
    private var generation = 0

    public init(handle: String, runner: String, calls: Calls, now: @escaping () -> Date = Date.init) {
        self.handle = handle
        self.runner = runner
        self.calls = calls
        self.now = now
    }

    /// Whether Sign In can be pressed: a link is up and the field has a code.
    public var canFinish: Bool {
        guard case .page = step else { return false }
        return !code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Ask the runner for a sign-in link. Also what Try Again and Start
    /// Again do: a new link, since the last one is spent.
    public func begin() async {
        began = true
        generation += 1
        let mine = generation
        // A sheet that has gone asks the runner for nothing.
        guard !closed else { return }
        step = .starting
        code = ""
        codeNotice = nil
        openNotice = nil
        if let old = session {
            session = nil
            await calls.cancel(old)
        }
        do {
            let started = try await calls.start(handle)
            // Closed, or started over, while the runner was answering: that
            // link is nobody's, so it is ended and the screen left alone.
            guard !closed, mine == generation else {
                await calls.cancel(started.session)
                return
            }
            // The runner vouched for it; this checks again before anything is opened.
            guard let url = ClaudeSignIn.page(started.url) else {
                await calls.cancel(started.session)
                step = .failed(ClaudeSignIn.tooOld(runner: runner))
                return
            }
            session = started.session
            expiresAt = started.expiresAt
            step = .page(url)
        } catch {
            guard !closed, mine == generation else { return }
            step = .failed(failure(error, during: .start))
        }
    }

    /// `begin`, once: what the sheet does when it appears, so a model already
    /// driven past its first step is not started over.
    public func beginIfNeeded() async {
        guard !began else { return }
        await begin()
    }

    /// The address to open now: nil when there is no link
    /// up or it has run out, which also moves the sheet to its expired step.
    public func pageToOpen() -> URL? {
        guard case .page(let url) = step else { return nil }
        if let expiresAt, now() >= expiresAt {
            step = .failed(ClaudeSignIn.expired)
            return nil
        }
        // Checked when it arrived (`begin`), and the only way into `.page`.
        return url
    }

    /// No browser took the page: said beside the button, and the sign-in
    /// stays up for another try.
    public func pageNotOpened() {
        openNotice = ClaudeSignIn.pageNotOpened
    }

    /// Send the pasted code.
    public func finish() async {
        guard case .page(let page) = step, let session else { return }
        let pasted = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !pasted.isEmpty else { return }
        if let expiresAt, now() >= expiresAt {
            step = .failed(ClaudeSignIn.expired)
            return
        }
        guard ClaudeSignIn.isWhole(code: pasted) else {
            codeNotice = ClaudeSignIn.incompleteCode
            return
        }
        codeNotice = nil
        generation += 1
        let mine = generation
        step = .finishing
        do {
            let login = try await calls.finish(session, pasted)
            guard !closed, mine == generation else { return }
            self.session = nil
            code = ""
            step = .done(email: login.email)
        } catch ClaudeSignIn.CallError.refused(word: _, what: "sign_in_code"?) {
            guard !closed, mine == generation else { return }
            // The runner held the sign-in: the code just wasn't whole.
            codeNotice = ClaudeSignIn.incompleteCode
            step = .page(page)
        } catch ClaudeSignIn.CallError.unreachable {
            guard !closed, mine == generation else { return }
            // No answer: the runner may have taken the code, or may still be
            // waiting for it. The sign-in stays, with the code, to send again.
            codeNotice = ClaudeSignIn.failure(for: .unreachable, during: .finish, runner: runner).message
            step = .page(page)
        } catch {
            // Whatever else ends the sign-in on the runner's side.
            guard !closed, mine == generation else { return }
            self.session = nil
            step = .failed(failure(error, during: .finish))
        }
    }

    /// The sheet is closing: end a sign-in still under way, so no process
    /// waits on the runner for a code that will never come.
    public func close() async {
        closed = true
        generation += 1
        guard let old = session else { return }
        session = nil
        await calls.cancel(old)
    }

    private func failure(_ error: Error, during phase: ClaudeSignIn.Phase) -> ClaudeSignIn.Failure {
        ClaudeSignIn.failure(for: error as? ClaudeSignIn.CallError ?? .refused(word: nil, what: nil), during: phase, runner: runner)
    }
}
