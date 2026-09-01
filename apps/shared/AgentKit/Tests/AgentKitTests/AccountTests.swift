import Foundation
import Testing

@testable import AgentKit

/// The pure half of signing in.
///
/// These four functions decide whether PKCE protects anything, and all of them
/// fail the same way: silently. A base64url that leaves a `+` or an `=` in place
/// produces a challenge WorkOS rejects, an `exp` that fails to parse means every
/// call refreshes a token that did not need refreshing — and both present to the
/// user as "sign-in just doesn't work", with nothing local to point at.
@MainActor
struct AccountTests {
    /// RFC 7636 appendix B, the published verifier and its challenge.
    ///
    /// Worth using the spec's own vector rather than a round trip through our
    /// own code: a round trip proves we are self-consistent, which is exactly
    /// what a wrong implementation also is.
    @Test func aChallengeMatchesTheSpecVector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        #expect(Account.challenge(for: verifier) == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
    }

    /// base64url, not base64. The three characters that differ are the three
    /// that break a URL, and a `+` in a query string arrives as a space.
    @Test func aChallengeCarriesNothingThatBreaksAUrl() throws {
        let verifier = try #require(Account.randomVerifier())
        let challenge = try #require(Account.challenge(for: verifier))
        for forbidden in ["+", "/", "="] {
            #expect(!challenge.contains(forbidden))
            #expect(!verifier.contains(forbidden))
        }
    }

    /// 32 bytes, which base64url encodes to 43 characters with no padding —
    /// inside RFC 7636's 43-to-128 range, and long enough to be unguessable.
    @Test func aVerifierIsLongEnoughToBeWorthSomething() throws {
        let verifier = try #require(Account.randomVerifier())
        #expect(verifier.count == 43)
        #expect(Account.randomVerifier() != verifier, "two sign-ins must not share a verifier")
    }

    @Test func aMalformedTokenHasNoExpiry() {
        #expect(Account.jwtExpiry("not a jwt") == nil)
        #expect(Account.jwtExpiry("only.two") == nil)
        #expect(Account.jwtExpiry("a.!!!not-base64!!!.c") == nil)
    }

    /// Authentication Services completes on Safari's XPC queue. Constructing
    /// its callback on the main actor makes Swift 6 trap before the callback
    /// body runs, so exercise the production callback entirely off that actor.
    @Test func authenticationCanCompleteOffTheMainActor() async throws {
        let expected = try #require(URL(string: "farcooler://auth?code=accepted"))
        let callback = try await Task.detached {
            try await withCheckedThrowingContinuation { continuation in
                let completion = Account.authenticationCompletion(for: continuation)
                completion(expected, nil)
            }
        }.value

        #expect(callback == expected)
    }

    /// Unverified on purpose — the relay holds the JWKS and does the verifying.
    /// This only decides whether to bother sending a token that is already
    /// stale, so a forged expiry buys an attacker one extra refresh.
    @Test func anExpiryIsReadFromThePayload() throws {
        let payload = try JSONSerialization.data(withJSONObject: ["exp": 1_700_000_000])
        let encoded = payload.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
        let expiry = try #require(Account.jwtExpiry("header.\(encoded).signature"))
        #expect(expiry.timeIntervalSince1970 == 1_700_000_000)
    }

    /// A 401 means the relay answered and rejected this bearer. Calling it an
    /// outage both lies to the user and prevents the one recovery that can
    /// work: refresh the session and retry once.
    @Test func responseStatusSelectsTheRightAuthenticationAction() {
        #expect(Account.responseAction(for: 200) == .accept)
        #expect(Account.responseAction(for: 401) == .refreshSession)
        #expect(Account.responseAction(for: 500) == .fail)
    }

    @Test func anUnauthorizedResponseRefreshesAndRetriesOnce() async {
        var requested: [String] = []
        var refreshes = 0

        let result = await Account.retryingUnauthorized(
            token: "stale",
            refresh: {
                refreshes += 1
                return .success("fresh")
            },
            request: { token in
                requested.append(token)
                if token == "stale" { throw AccountError.unauthorized }
                return "accepted"
            })

        #expect((try? result.get()) == "accepted")
        #expect(requested == ["stale", "fresh"])
        #expect(refreshes == 1)
    }

    @Test func aFreshTokenRejectedByTheRelayStopsAfterOneRetry() async {
        var requested: [String] = []

        let result: Result<String, AccountError> = await Account.retryingUnauthorized(
            token: "stale",
            refresh: { .success("fresh") },
            request: { token in
                requested.append(token)
                throw AccountError.unauthorized
            })

        // And it says WHICH refusal this was. The relay answered, twice, and
        // would not take the bearer either time — the one failure in this file
        // where "try signing in again" is the right thing to tell somebody.
        #expect(result.failure == .unauthorized)
        #expect(requested == ["stale", "fresh"])
    }

    @Test func anUnauthorizedResponseWithNoRefreshedTokenStopsBeforeRetrying() async {
        var requested: [String] = []
        var refreshes = 0

        let result: Result<String, AccountError> = await Account.retryingUnauthorized(
            token: "stale",
            refresh: {
                refreshes += 1
                return .failure(.notSignedIn)
            },
            request: { token in
                requested.append(token)
                throw AccountError.unauthorized
            })

        // The REFRESH's reason travels, not the request's. A refresh that
        // failed because this device holds no credential is a different
        // sentence from one that failed because the relay refused the token,
        // and both used to arrive as the same `nil`.
        #expect(result.failure == .notSignedIn)
        #expect(requested == ["stale"])
        #expect(refreshes == 1)
    }

    /// A refresh that could not be attempted is not a session that has ended.
    /// Somebody on a train told to sign in again is sent to a sheet that also
    /// cannot work.
    @Test func aRefreshThatCouldNotBeReachedIsNotAnExpiredSession() async {
        let result: Result<String, AccountError> = await Account.retryingUnauthorized(
            token: "stale",
            refresh: { .failure(.unreachable(reason: "offline")) },
            request: { _ in throw AccountError.unauthorized })

        #expect(result.failure == .unreachable(reason: "offline"))
        #expect(result.failure?.message == "Couldn’t reach the relay. Check your internet connection.")
    }

    @Test func aRelayFailureAfterRefreshStopsAfterOneRetry() async {
        var requested: [String] = []

        let result: Result<String, AccountError> = await Account.retryingUnauthorized(
            token: "stale",
            refresh: { .success("fresh") },
            request: { token in
                requested.append(token)
                if token == "stale" { throw AccountError.unauthorized }
                throw AccountError.relayFailed(status: 500)
            })

        #expect(result.failure == .relayFailed(status: 500))
        #expect(requested == ["stale", "fresh"])
    }

    @Test func anotherRelayFailureDoesNotRefreshTheSession() async {
        var refreshes = 0

        let result: Result<String, AccountError> = await Account.retryingUnauthorized(
            token: "current",
            refresh: {
                refreshes += 1
                return .success("unused")
            },
            request: { _ in throw AccountError.relayFailed(status: 500) })

        #expect(result.failure == .relayFailed(status: 500))
        #expect(refreshes == 0)
    }

    @Test func aFailedRefreshLeavesTheLocalSessionAlone() async {
        var stored = false

        let token = await Account.refreshingAccessToken(
            refreshToken: "still-local",
            request: { _ in throw AccountError.relayFailed(status: 500) },
            store: { _ in stored = true })

        #expect(token.failure == .relayFailed(status: 500))
        #expect(!stored)
    }

    @Test func aSuccessfulRefreshStoresAndReturnsTheNewToken() async {
        var stored: [String: Any]?

        let token = await Account.refreshingAccessToken(
            refreshToken: "refresh",
            request: { _ in ["accessToken": "new-access"] },
            store: { stored = $0 })

        #expect((try? token.get()) == "new-access")
        #expect(stored?["accessToken"] as? String == "new-access")
    }
}

extension Result {
    /// The error, for tests that care which one it was.
    var failure: Failure? {
        guard case .failure(let error) = self else { return nil }
        return error
    }
}

/// A relay that answers from a script, with no relay in it.
///
/// Everything between a status code and the sentence somebody reads is
/// production code: `Account.failure(forStatus:)` is the switch `post` runs on
/// the wire, and `Account.authenticating` is the whole of `authenticatedPost`
/// minus the URL session. The only thing this class owns is which answer comes
/// next — so a test that passes here is a test about the app, not about itself.
@MainActor
final class ScriptedRelay {
    /// The status codes to answer with, in order. Anything past the end is 200.
    private var answers: [Int]
    /// Which bearer was sent, in order. Two entries means it retried.
    private(set) var sent: [String] = []
    private(set) var refreshes = 0

    /// What the token store has, and what a forced refresh comes back with.
    var credential: Result<String, AccountError> = .success("bearer")
    var refreshed: Result<String, AccountError> = .success("fresh")
    /// Thrown instead of answering at all: the request that never left.
    var transport: (any Error)?

    init(_ answers: [Int] = []) { self.answers = answers }

    func call() async -> Result<[String: Any], AccountError> {
        await Account.authenticating(
            credential: { forceRefresh in
                guard forceRefresh else { return self.credential }
                self.refreshes += 1
                return self.refreshed
            },
            request: { bearer in
                self.sent.append(bearer)
                if let transport = self.transport { throw transport }
                let status = self.answers.isEmpty ? 200 : self.answers.removeFirst()
                if let failure = Account.failure(forStatus: status) { throw failure }
                return [:]
            })
    }

    /// The sentence a person would be shown, or "" if the call worked.
    func sentence() async -> String {
        guard case .failure(let why) = await call() else { return "" }
        return why.message
    }
}

/// Five ways one relay call fails, and five different true things to say.
///
/// The defect these exist for: `authenticatedPost` answered `nil` for every one
/// of them, its callers turned that `nil` into one fixed sentence — "The relay
/// wouldn’t issue a token. Try signing in again." — and the app therefore
/// asserted one cause for at least five failures. It was right about one fifth
/// of the time and there was no way, from inside or outside, to know which.
///
/// Each test spells the sentence out in full rather than comparing against the
/// enum's own `message`. A test that read the property back would pass for any
/// wording, including the wrong one.
@MainActor
struct RelayFailureTests {
    /// Every case, so the whole-vocabulary properties below are about the whole
    /// vocabulary rather than the part somebody remembered to list.
    static let everyFailure: [AccountError] = [
        .notSignedIn,
        .signInIncomplete,
        .relayAddressInvalid,
        .unreachable(reason: "offline"),
        .unauthorized,
        .rateLimited,
        .endpointMissing,
        .relayFailed(status: 500),
        .requestRefused(status: 400),
        .malformedResponse,
    ]

    // MARK: - One sentence per cause

    @Test("Signed out on this device: nothing is even sent")
    func signedOut() async {
        let relay = ScriptedRelay()
        relay.credential = .failure(.notSignedIn)

        #expect(
            await relay.sentence()
                == "You’re not signed in on this device. Sign in under Settings ▸ Account.")
        #expect(relay.sent.isEmpty, "there is no credential, so there is nothing to send")
        #expect(relay.refreshes == 0)
    }

    @Test("A bearer refused twice is the one case that says sign in again")
    func refusedAfterARefresh() async {
        let relay = ScriptedRelay([401, 401])

        #expect(
            await relay.sentence()
                == "The relay wouldn’t accept your session. Try signing in again.")
        // It really did refresh and really did try again, which is what makes
        // "sign in again" honest here and a guess everywhere else.
        #expect(relay.sent == ["bearer", "fresh"])
        #expect(relay.refreshes == 1)
    }

    @Test("A relay that broke says so, and does not blame your sign-in")
    func relayFailed() async {
        let relay = ScriptedRelay([500])
        let sentence = await relay.sentence()

        #expect(
            sentence == "The relay had a problem of its own. "
                + "Nothing on this device is wrong — try again in a few minutes.")
        #expect(!sentence.contains("sign"), "a 500 is not a credential problem")
        #expect(relay.refreshes == 0, "a 500 is not repaired by a new bearer")
    }

    @Test("A missing endpoint points at the relay address, not at your account")
    func endpointMissing() async {
        let relay = ScriptedRelay([404])
        let sentence = await relay.sentence()

        #expect(
            sentence == "This relay doesn’t offer what the app asked for. "
                + "Check the relay address under Settings ▸ Advanced.")
        #expect(!sentence.contains("sign"))
    }

    @Test("A request that never left says check the network")
    func transportFailed() async {
        let relay = ScriptedRelay()
        relay.transport = URLError(.notConnectedToInternet)

        #expect(
            await relay.sentence()
                == "Couldn’t reach the relay. Check your internet connection.")
        #expect(relay.refreshes == 0, "being offline is not a reason to spend a refresh token")
    }

    @Test("Too many auth requests is its own answer, and it is to wait")
    func rateLimited() async {
        let relay = ScriptedRelay([429])

        #expect(
            await relay.sentence()
                == "The relay is turning away sign-ins right now. Wait a minute and try again.")
    }

    @Test("A 200 a refresh repaired is not a failure at all")
    func aRefreshRepairsIt() async {
        let relay = ScriptedRelay([401, 200])

        #expect(await relay.sentence() == "", "the retry worked, so nobody is told anything")
        #expect(relay.sent == ["bearer", "fresh"])
        #expect(relay.refreshes == 1)
    }

    // MARK: - The vocabulary as a whole

    @Test("No two failures read the same")
    func everySentenceIsDifferent() {
        let sentences = Self.everyFailure.map(\.message)
        #expect(Set(sentences).count == sentences.count)
        #expect(sentences.allSatisfy { !$0.isEmpty })
    }

    /// The advice that used to be given for all of them.
    @Test("Exactly one failure tells you to sign in again")
    func onlyOneSaysSignInAgain() {
        let saying = Self.everyFailure.filter { $0.message.contains("signing in again") }
        #expect(saying == [.unauthorized])
    }

    /// Apple copy: a person gets a sentence they can act on. The status code is
    /// for the log and for Copy Details, and must not reach a screen.
    @Test("No sentence shows a status code or a raw error")
    func noSentenceLeaksMachinery() {
        for failure in Self.everyFailure {
            let sentence = failure.message
            #expect(!sentence.contains("HTTP"), "\(failure) put a protocol word on screen")
            #expect(!sentence.contains("40"), "\(failure) put a status code on screen")
            #expect(!sentence.contains("50"), "\(failure) put a status code on screen")
            #expect(!sentence.contains("URLError"))
        }
    }

    // MARK: - What the developer gets, and what it must never carry

    @Test("A diagnostic names the call and the code, and nothing else")
    func aDiagnosticIsCodesNotProse() {
        let line = RelayDiagnostic(
            path: "/v1/daemons", failure: .relayFailed(status: 503), at: .now
        ).line

        #expect(line.contains("/v1/daemons"))
        #expect(line.contains("HTTP 503"))
    }

    /// The property that makes it safe to log this and to put it on a
    /// clipboard. Every component is a literal in `Account.swift` or an integer
    /// read off the transport — never a header, a request body or a response
    /// body — so there is no route by which a bearer or a pairing token reaches
    /// it.
    @Test("No diagnostic can carry a credential")
    func aDiagnosticCarriesNoCredential() {
        for failure in Self.everyFailure {
            let line = RelayDiagnostic(path: "/v1/daemons", failure: failure, at: .now)
                .line.lowercased()
            #expect(!line.contains("bearer"))
            #expect(!line.contains("token"))
            #expect(!line.contains("authorization"))
        }
    }

    /// `localizedDescription` is prose other frameworks assemble, sometimes out
    /// of the failing URL, and this string is logged. So a transport failure is
    /// named by its code and never by its text.
    @Test("A transport failure is named by its code, never by its message")
    func aTransportFailureIsNamedByCode() {
        #expect(Account.transportReason(URLError(.notConnectedToInternet)) == "offline")
        #expect(Account.transportReason(URLError(.cannotFindHost)) == "DNS")
        #expect(Account.transportReason(URLError(.timedOut)) == "timed out")
        #expect(Account.named(URLError(.timedOut)) == .unreachable(reason: "timed out"))

        let talkative = NSError(
            domain: "Test", code: 1,
            userInfo: [NSLocalizedDescriptionKey: "Bearer sk-live-should-never-appear"])
        #expect(!Account.transportReason(talkative).contains("sk-live"))
        #expect(!Account.named(talkative).diagnostic.contains("sk-live"))
    }

    // MARK: - The property that must not regress

    /// A relay refusal must not sign the user out. `store` is the only thing in
    /// the refresh path that writes identity or tokens, so a failure that never
    /// reaches it is a failure that cannot have ended a session.
    @Test("No relay refusal writes to the local session")
    func aRefusalNeverTouchesTheSession() async {
        for failure in Self.everyFailure {
            var stored = false
            let outcome = await Account.refreshingAccessToken(
                refreshToken: "still-local",
                request: { _ in throw failure },
                store: { _ in stored = true })

            #expect(outcome.failure == failure)
            #expect(!stored, "\(failure) wrote to the session")
        }
    }

    /// And the structural half of the same promise: `clearSession()` is the
    /// only thing that can end a session, and `signOut()` is the only thing
    /// allowed to call it. Checked in the source because that is where it can
    /// actually break — a future `catch` that "cleans up" would compile, pass
    /// every behavioral test above, and silently sign people out on a bad day.
    @Test("Only signing out ends a session")
    func onlySignOutClearsTheSession() throws {
        let source = try String(contentsOf: Self.accountSource, encoding: .utf8)
        // One declaration and one call.
        #expect(source.components(separatedBy: "clearSession()").count - 1 == 2)

        let signOut = try #require(source.range(of: "public func signOut() async {"))
        let after = source[signOut.upperBound...]
        let end = try #require(after.range(of: "\n    }\n"))
        #expect(after[..<end.lowerBound].contains("clearSession()"))
    }

    private static var accountSource: URL {
        URL(filePath: #filePath)
            .deletingLastPathComponent()  // AgentKitTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // the package
            .appending(path: "Sources/AgentKit/Account.swift")
    }
}
