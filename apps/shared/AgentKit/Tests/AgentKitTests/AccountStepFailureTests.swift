import Foundation
import Testing

@testable import AgentKit

/// ov-505: a failed refresh, a request refused after a good refresh, and a
/// Keychain that would not keep the new tokens are three different faults and
/// get three different sentences. The wording is spelled out here, not read
/// back from the enum, so a test cannot pass for the wrong words.
@MainActor
struct AccountStepFailureTests {
    @Test("The relay refusing the refresh itself says the sign-in ended")
    func refusedRefresh() async {
        var stored = false
        let outcome = await Account.refreshingAccessToken(
            refreshToken: "dead",
            request: { _ in throw AccountError.unauthorized },
            store: { _ in stored = true; return nil })

        #expect(outcome.failure == .refreshRefused)
        #expect(!stored)
        #expect(
            AccountError.refreshRefused.message
                == "Your sign-in couldn’t be renewed. If this keeps happening, sign in again under Settings > Account.")
        #expect(AccountError.refreshRefused.diagnostic == "HTTP 401 on refresh")
    }

    @Test("A relay 502 or 429 on refresh is an outage, never a refused sign-in (ov-574)",
          arguments: [(502, AccountError.relayFailed(status: 502)),
                      (500, AccountError.relayFailed(status: 500)),
                      (429, AccountError.rateLimited),
                      (401, AccountError.refreshRefused)])
    func refreshByStatus(status: Int, expected: AccountError) async {
        var stored = false
        let outcome = await Account.refreshingAccessToken(
            refreshToken: "rt",
            request: { _ in
                // The same switch `post` runs on the wire.
                if let failure = Account.failure(forStatus: status) { throw failure }
                return [:]
            },
            store: { _ in stored = true; return nil })

        #expect(outcome.failure == expected)
        #expect(!stored)
        if status != 401 { #expect(outcome.failure != .refreshRefused) }
    }

    @Test("A refresh refused, seen from a request, is not the after-refresh failure")
    func refusedRefreshThroughARequest() async {
        var requests = 0
        let result: Result<String, AccountError> = await Account.retryingUnauthorized(
            token: "stale",
            refresh: {
                await Account.refreshingAccessToken(
                    refreshToken: "dead",
                    request: { _ in throw AccountError.unauthorized },
                    store: { _ in nil })
            },
            request: { _ in
                requests += 1
                throw AccountError.unauthorized
            })

        #expect(result.failure == .refreshRefused)
        #expect(requests == 1, "no second request after a refresh that failed")
    }

    @Test("A request refused after a good refresh keeps its own wording")
    func refusedAfterGoodRefresh() async {
        let result: Result<String, AccountError> = await Account.retryingUnauthorized(
            token: "stale",
            refresh: {
                await Account.refreshingAccessToken(
                    refreshToken: "good",
                    request: { _ in ["accessToken": "fresh"] },
                    store: { _ in nil })
            },
            request: { _ in throw AccountError.unauthorized })

        #expect(result.failure == .unauthorized)
        #expect(
            AccountError.unauthorized.message
                == "The relay turned down your session even though it was just renewed. Try again in a few minutes. If it keeps happening, sign out and back in.")
        #expect(AccountError.unauthorized.diagnostic == "HTTP 401 after refresh")
    }
}
