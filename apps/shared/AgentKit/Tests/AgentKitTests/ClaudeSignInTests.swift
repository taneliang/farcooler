import Foundation
import Testing

@testable import AgentKit

/// A member signing in to Claude from their own device (ov-584): the page
/// the app will open, the sheet's steps, and the words.
struct ClaudeSignInTests {
    static let claudeAi = "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=xyz"
    static let console = "https://platform.claude.com/oauth/authorize?code=true&client_id=abc&state=xyz"

    // MARK: - The page

    @Test("Both sign-in pages claude gives are opened, and nothing else")
    func pages() {
        #expect(ClaudeSignIn.page(Self.claudeAi)?.absoluteString == Self.claudeAi)
        #expect(ClaudeSignIn.page(Self.console)?.absoluteString == Self.console)
        let refused = [
            "http://claude.com/cai/oauth/authorize?x=1",
            "https://evil.example/cai/oauth/authorize?x=1",
            "https://claude.com.evil.example/cai/oauth/authorize?x=1",
            "https://claude.com@evil.example/cai/oauth/authorize?x=1",
            "https://user:pw@claude.com/cai/oauth/authorize?x=1",
            "https://claude.com:8443/cai/oauth/authorize?x=1",
            "https://claude.com/cai/oauth/authorize",
            "https://claude.com/cai/oauth/authorize?",
            "https://claude.com/oauth/authorize?x=1",
            "https://platform.claude.com/cai/oauth/authorize?x=1",
            "https://claude.com/cai/oauth/authorize/extra?x=1",
            "https://claude.com/cai/oauth/authorize?x=1#frag",
            "https://Claude.com/cai/oauth/authorize?x=1",
            "https://claude.com./cai/oauth/authorize?x=1",
            "https://claude%2ecom/cai/oauth/authorize?x=1",
            "https://claude.com/cai/oauth/authorize?x=1 y",
            "https://claude.com/cai/oauth/authorize?x=1\n",
            "https://claude.com\\@evil.example/cai/oauth/authorize?x=1",
            "https://claude.com/cai/oauth/authorize?x=é",
            "javascript:alert(1)",
            "https://",
            "",
        ]
        for text in refused { #expect(ClaudeSignIn.page(text) == nil, "opened \(text.debugDescription)") }
    }

    @Test("A page address has a length cap")
    func pageLength() {
        let base = "https://claude.com/cai/oauth/authorize?x="
        #expect(ClaudeSignIn.page(base + String(repeating: "a", count: ClaudeSignIn.maxPageLength - base.utf8.count)) != nil)
        #expect(ClaudeSignIn.page(base + String(repeating: "a", count: ClaudeSignIn.maxPageLength - base.utf8.count + 1)) == nil)
    }

    // MARK: - The code

    @Test("A code is whole with both halves around a #, after a clipboard's spaces and line break")
    func codes() {
        #expect(ClaudeSignIn.isWhole(code: "abc#def"))
        #expect(ClaudeSignIn.isWhole(code: "  abc#def\n"))
        #expect(!ClaudeSignIn.isWhole(code: "abcdef"))
        #expect(!ClaudeSignIn.isWhole(code: "abc#"))
        #expect(!ClaudeSignIn.isWhole(code: "#def"))
        #expect(!ClaudeSignIn.isWhole(code: ""))
    }

    // MARK: - The words

    @Test("Each way a sign-in fails has its sentence, and the move that helps")
    func failures() {
        func said(_ e: ClaudeSignIn.CallError, _ phase: ClaudeSignIn.Phase = .finish) -> ClaudeSignIn.Failure {
            ClaudeSignIn.failure(for: e, during: phase, runner: "Studio")
        }
        #expect(
            said(.refused(word: "resource-conflict", what: "code_refused"))
                == .init("That code didn’t work. Open the sign-in page again to get a new one.", retry: .tryAgain))
        #expect(
            said(.refused(word: "not-found", what: nil)) == .init("The sign-in link expired.", retry: .startAgain))
        let tooOld = ClaudeSignIn.Failure("Couldn’t start sign-in on Studio. Claude Code there needs to be updated.")
        #expect(said(.refused(word: "resource-conflict", what: "sign_in_too_old")) == tooOld)
        #expect(said(.refused(word: "resource-conflict", what: "sign_in_unreadable")) == tooOld)
        #expect(said(.refused(word: "capability-unsupported", what: nil)) == tooOld)
        #expect(
            said(.refused(word: "resource-conflict", what: "sign_in_too_often"))
                == .init("Too many sign-in attempts. Wait a minute, then try again."))
        #expect(
            said(.unreachable)
                == .init("Studio isn’t answering. Check your connection and try again.", retry: .tryAgain))
        // A word this build has never seen: a plain sentence, never the word.
        let other = said(.refused(word: "operation-failed", what: "something_new"))
        #expect(other == .init("Couldn’t sign in. Try again.", retry: .tryAgain))
        #expect(!other.message.contains("something_new") && !other.message.contains("operation-failed"))
        // A link that expired is news about a finish only; a start that finds nothing is just a failure.
        #expect(said(.refused(word: "not-found", what: nil), .start) == other)
        // And "isn't answering" is for a link that never answered, not for an answer this build can't read.
        #expect(!said(.refused(word: nil, what: nil)).message.contains("answering"))
    }

    @Test("A failed CLI call is a refusal when the runner named a code, and no answer when it didn't")
    func cliErrors() {
        let refused = "error: the sign-in didn't accept that code\ncode: resource-conflict\nwhat: code_refused\n"
        #expect(ClaudeSignIn.CallError(cliErrors: refused) == .refused(word: "resource-conflict", what: "code_refused"))
        #expect(
            ClaudeSignIn.CallError(cliErrors: "code: not-found") == .refused(word: "not-found", what: nil))
        #expect(ClaudeSignIn.CallError(cliErrors: "ssh: connect to host studio port 22: Operation timed out") == .unreachable)
        // Nothing printed: the call never ran, which is a plain failure and not the runner's silence.
        #expect(ClaudeSignIn.CallError(cliErrors: "") == .refused(word: nil, what: nil))
    }

    @Test("The trust copy names the runner, and the workspace lead without naming one")
    func trust() {
        #expect(
            ClaudeSignIn.trustCopy(runner: "Studio")
                == "Your Claude login is kept on Studio. Its owner, and anyone with Control of it, could use it. "
                + "Far Cooler uses it only for your rep and the lanes you start. "
                + "Lanes the orchestrator starts for your asks use the workspace lead’s login.")
        #expect(ClaudeSignIn.ownerTrustLine == "Each member’s agents use that member’s own Claude login.")
    }

    @Test("The rows and the sheet say what the design says")
    func sentences() {
        #expect(ClaudeSignIn.signedInLine(email: "alex@example.com") == "Claude · alex@example.com")
        #expect(ClaudeSignIn.signedInAs("alex@example.com") == "Signed in as alex@example.com.")
        #expect(ClaudeSignIn.starting(runner: "Studio") == "Getting a sign-in link from Studio…")
        #expect(ClaudeSignIn.signOutTitle(runner: "Studio") == "Sign out of Claude on Studio?")
        #expect(ClaudeSignIn.signOutMessage == "Your rep and lanes there stop until you sign in again.")
        #expect(
            ClaudeSignIn.notSignedInBody
                == "Sign in so your rep and the lanes you start use your own Claude account and its connectors.")
    }

    // MARK: - The wire

    @Test("An email wrapped at an accessibility size breaks after punctuation, and is the same address without the breaks")
    func breakableEmail() {
        let email = "bob.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"
        let shown = ClaudeSignIn.breakable(email)
        #expect(shown.replacingOccurrences(of: "\u{200B}", with: "") == email)
        #expect(shown.hasPrefix("bob.\u{200B}montgomery-\u{200B}fitzgerald@\u{200B}platform-\u{200B}"))
        // Nothing is added inside a word, and a plain address gets one after its dot and its @.
        #expect(ClaudeSignIn.breakable("a@b.c") == "a@\u{200B}b.\u{200B}c")
        #expect(ClaudeSignIn.breakable("") == "")
    }

    @Test("A login and a started sign-in decode from the runner's JSON")
    func wire() throws {
        let login = try #require(ClaudeLogin(json: Data(#"{"state":"signed_in","email":" alex@example.com "}"#.utf8)))
        #expect(login == ClaudeLogin(state: .signedIn, email: "alex@example.com"))
        // A state from a newer runner is no login, never a guess at one.
        #expect(try #require(ClaudeLogin(json: Data(#"{"state":"revoked"}"#.utf8))) == .signedOut)
        #expect(ClaudeLogin(json: Data("[]".utf8)) == nil)
        let started = try #require(
            ClaudeSignInModel.Started(
                json: Data(#"{"session":"s1","url":"https://claude.com/x","expires_at":1760000000000}"#.utf8)))
        #expect(started.session == "s1" && started.expiresAt == Date(timeIntervalSince1970: 1_760_000_000))
        #expect(ClaudeSignInModel.Started(json: Data(#"{"url":"u"}"#.utf8)) == nil)
    }
}
