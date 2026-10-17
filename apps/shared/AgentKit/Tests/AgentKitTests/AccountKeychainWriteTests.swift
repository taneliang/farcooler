import Foundation
import Security
import Testing

@testable import AgentKit

/// ov-505: a Keychain that refuses a write must not cost the person their
/// session. Every Keychain here is a dictionary the test owns, so the real one
/// is never touched.
@MainActor
final class FakeKeychain {
    var items: [String: String] = [:]
    /// Keys whose writes fail, with the status they fail with.
    var refusing: [String: OSStatus] = [:]
    var writes: [String] = []
    var deleted: [String] = []

    func keeper(defaults: UserDefaults) -> TokenKeeper {
        TokenKeeper(
            write: { key, value in
                self.writes.append(key)
                if let status = self.refusing[key] { return status }
                self.items[key] = value
                return errSecSuccess
            },
            read: { self.items[$0] },
            delete: { self.deleted.append($0); self.items[$0] = nil },
            defaults: defaults)
    }
}

@MainActor
struct AccountKeychainWriteTests {
    let defaults: UserDefaults
    let keychain = FakeKeychain()

    init() {
        defaults = ScratchDefaults.suite("ov505-\(UUID().uuidString)")!
    }

    static let answer: [String: Any] = [
        "accessToken": "a2", "refreshToken": "r2", "userId": "u1", "email": "me@example.com",
    ]

    @Test("The refresh token is written first, and both writes are attempted")
    func order() {
        keychain.refusing[Account.refreshKey] = -25308
        let recorded = keychain.keeper(defaults: defaults).record(Self.answer)

        #expect(keychain.writes == [Account.refreshKey, Account.accessKey])
        #expect(recorded.failure == .keychainWriteFailed(status: -25308))
        #expect(keychain.items[Account.accessKey] == "a2", "the access write was still attempted")
    }

    @Test("The first refusal is the one reported")
    func firstRefusal() {
        keychain.refusing[Account.refreshKey] = -25308
        keychain.refusing[Account.accessKey] = -34018
        let recorded = keychain.keeper(defaults: defaults).record(Self.answer)
        #expect(recorded.failure == .keychainWriteFailed(status: -25308))
    }

    @Test("A refused write leaves the name and email untouched")
    func labelsStayPut() {
        defaults.set("old", forKey: "account.userId")
        keychain.refusing[Account.accessKey] = -25308
        let recorded = keychain.keeper(defaults: defaults).record(Self.answer)

        #expect(recorded.failure != nil)
        #expect(recorded.userId == nil && recorded.email == nil)
        #expect(defaults.string(forKey: "account.userId") == "old")
        #expect(defaults.string(forKey: "account.email") == nil)
    }

    @Test("A kept session records its name and email")
    func labelsWritten() {
        let recorded = keychain.keeper(defaults: defaults).record(Self.answer)
        #expect(recorded.failure == nil)
        #expect(recorded.userId == "u1" && recorded.email == "me@example.com")
        #expect(defaults.string(forKey: "account.email") == "me@example.com")
    }

    @Test("The diagnostic carries the status integer and nothing else")
    func diagnostic() {
        #expect(
            AccountError.keychainWriteFailed(status: -25308).diagnostic
                == "keychain write failed (-25308)")
        #expect(
            AccountError.keychainWriteFailed(status: -25308).message
                == "Far Cooler couldn’t save your sign-in. Try again, and if this keeps happening, sign in again under Settings > Account.")
    }

    @Test("A sign-in whose write is refused throws, and takes back what it wrote")
    func exchangeThrows() async {
        keychain.refusing[Account.refreshKey] = -25308
        let keeper = keychain.keeper(defaults: defaults)
        var thrown: AccountError?
        do {
            _ = try await Account.completingSignIn(request: { Self.answer }, keeper: keeper)
        } catch {
            thrown = error as? AccountError
        }

        #expect(thrown == .keychainWriteFailed(status: -25308))
        #expect(keychain.items.isEmpty, "the access token it did write is deleted")
        #expect(keeper.pendingRefresh == nil, "a failed sign-in keeps no pending token")
        #expect(defaults.string(forKey: "account.email") == nil)
    }

    @Test("A 401 at sign-in is a sign-in error, not a failure after a refresh")
    func exchange401() async {
        var thrown: AccountError?
        do {
            _ = try await Account.completingSignIn(
                request: { throw AccountError.unauthorized }, keeper: keychain.keeper(defaults: defaults))
        } catch {
            thrown = error as? AccountError
        }
        #expect(thrown == .signInRefused)
        #expect(
            AccountError.signInRefused.message
                == "The relay turned down your sign-in. Try again from Settings > Account.")
        #expect(AccountError.signInRefused.diagnostic == "HTTP 401 at sign-in")
    }

    @Test("A refresh whose save is refused reports it, and the new token survives")
    func refreshSurfacesAndKeeps() async {
        keychain.items[Account.refreshKey] = "r1"
        keychain.refusing[Account.refreshKey] = -25308
        let keeper = keychain.keeper(defaults: defaults)

        let outcome = await Account.refreshingAccessToken(
            refreshToken: keeper.refreshToken() ?? "",
            request: { _ in Self.answer },
            store: { keeper.record($0).failure })

        #expect(outcome.failure == .keychainWriteFailed(status: -25308))
        #expect(keeper.pendingRefresh == "r2")
        #expect(keychain.items[Account.refreshKey] == "r1", "a refused write leaves the old item")
    }

    @Test("The next ask uses the pending token and retries the save")
    func pendingIsUsedAndSaved() {
        keychain.items[Account.refreshKey] = "r1"
        keychain.refusing[Account.refreshKey] = -25308
        let keeper = keychain.keeper(defaults: defaults)
        _ = keeper.record(Self.answer)

        // Still refused: the pending token is used, and stays pending.
        #expect(keeper.refreshToken() == "r2")
        #expect(keeper.pendingRefresh == "r2")

        // The Keychain recovers: the same ask saves it and clears it.
        keychain.refusing[Account.refreshKey] = nil
        #expect(keeper.refreshToken() == "r2")
        #expect(keychain.items[Account.refreshKey] == "r2")
        #expect(keeper.pendingRefresh == nil)
        #expect(keeper.refreshToken() == "r2", "now read back from the Keychain")
    }

    @Test("Signing out forgets a pending token")
    func forgetClears() {
        keychain.refusing[Account.refreshKey] = -25308
        let keeper = keychain.keeper(defaults: defaults)
        _ = keeper.record(Self.answer)
        keeper.forget()
        #expect(keeper.pendingRefresh == nil)
    }

    @Test("Sign-out reads the pending token, not the dead one in the Keychain")
    func signOutUsesPending() {
        keychain.items[Account.refreshKey] = "r1"
        keychain.refusing[Account.refreshKey] = -25308
        let keeper = keychain.keeper(defaults: defaults)
        #expect(keeper.currentRefreshToken() == "r1")
        _ = keeper.record(Self.answer)

        #expect(keeper.currentRefreshToken() == "r2")
        #expect(
            Account.logoutBody(refresh: keeper.currentRefreshToken(), pulse: nil)?["refreshToken"] as? String
                == "r2")
        let writesBefore = keychain.writes.count
        _ = keeper.currentRefreshToken()
        #expect(keychain.writes.count == writesBefore, "reading for sign-out saves nothing")
    }

    @Test("A refresh succeeds when only the access write was refused and the refresh token is saved")
    func refreshSurvivesAccessRefusal() async {
        keychain.items[Account.refreshKey] = "r1"
        keychain.refusing[Account.accessKey] = -25308
        let keeper = keychain.keeper(defaults: defaults)

        let outcome = await Account.refreshingAccessToken(
            refreshToken: "r1",
            request: { _ in Self.answer },
            store: { keeper.record($0).refreshFailure })

        #expect((try? outcome.get()) == "a2")
        #expect(keychain.items[Account.refreshKey] == "r2")
    }

    @Test("A sign-in still fails when only the access write is refused")
    func signInFailsOnAccessRefusal() async {
        keychain.refusing[Account.accessKey] = -25308
        var thrown: AccountError?
        do {
            _ = try await Account.completingSignIn(
                request: { Self.answer }, keeper: keychain.keeper(defaults: defaults))
        } catch {
            thrown = error as? AccountError
        }
        #expect(thrown == .keychainWriteFailed(status: -25308))
    }

    @Test("A failed re-sign-in puts the earlier session back")
    func failedReSignInRestores() async {
        keychain.items[Account.accessKey] = "a1"
        keychain.items[Account.refreshKey] = "r1"
        keychain.refusing[Account.accessKey] = -25308
        let keeper = keychain.keeper(defaults: defaults)

        do {
            _ = try await Account.completingSignIn(request: { Self.answer }, keeper: keeper)
        } catch {}

        #expect(keychain.items[Account.accessKey] == "a1")
        #expect(keychain.items[Account.refreshKey] == "r1")
    }
}
