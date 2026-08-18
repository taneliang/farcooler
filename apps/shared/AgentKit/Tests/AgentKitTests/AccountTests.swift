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
    @Test func anUnauthorizedResponseRefreshesTheSession() {
        #expect(Account.responseAction(for: 200) == .accept)
        #expect(Account.responseAction(for: 401) == .refreshSession)
        #expect(Account.responseAction(for: 500) == .fail)
    }
}
