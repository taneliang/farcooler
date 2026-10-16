import Foundation
import Security

/// What recording one sign-in or refresh answer came to.
struct RecordedSession {
    /// The first write the Keychain refused, if any.
    var failure: AccountError?
    /// The name and email in the answer. Nil after a refusal, so a session that
    /// was not saved is never labeled as signed in.
    var userId: String?
    var email: String?
    /// Whether the refresh token is safe: this answer's was saved, or it
    /// carried none and the one already held is untouched.
    var refreshSafe = false

    /// What a refresh reports: a refused access-token write is not a failure
    /// while the refresh token is safe, because the access token in the answer
    /// is still good and the next refresh can save it again. A refused
    /// refresh-token write still reports. Sign-in uses ``failure`` instead.
    var refreshFailure: AccountError? { refreshSafe ? nil : failure }
}

/// The session as it was before a sign-in began writing.
struct SessionSnapshot {
    var access: String?
    var refresh: String?
    var pendingRefresh: String?
}

/// Keeps the session tokens, and keeps the newest refresh token alive when the
/// Keychain will not take it (ov-505).
///
/// A WorkOS refresh token is single use: by the time a refresh answer arrives,
/// the token that was sent is already retired. If the new one is then dropped
/// because a Keychain write failed, the session is gone for good. So an unsaved
/// refresh token is held here, in memory only, preferred over the Keychain, and
/// saved again on the next ask. It is never logged, never written anywhere
/// but the Keychain, and forgotten on sign-out.
@MainActor
final class TokenKeeper {
    private let write: (String, String) -> OSStatus
    private let read: (String) -> String?
    private let delete: (String) -> Void
    private let defaults: UserDefaults

    /// The refresh token the Keychain has not yet accepted.
    private(set) var pendingRefresh: String?

    init(
        write: @escaping (String, String) -> OSStatus = { TokenStore.writeStatus($0, $1) },
        read: @escaping (String) -> String? = { TokenStore.read($0) },
        delete: @escaping (String) -> Void = { TokenStore.delete($0) },
        defaults: UserDefaults = .standard
    ) {
        self.write = write
        self.read = read
        self.delete = delete
        self.defaults = defaults
    }

    /// The refresh token to use now, saving a pending one first if it can.
    func refreshToken() -> String? {
        guard let pending = pendingRefresh else { return read(Account.refreshKey) }
        if write(Account.refreshKey, pending) == errSecSuccess { pendingRefresh = nil }
        return pending
    }

    /// The refresh token as it stands, pending first, without saving anything.
    /// For sign-out, which must end the session that is actually live.
    func currentRefreshToken() -> String? {
        pendingRefresh ?? read(Account.refreshKey)
    }

    /// Save an answer's tokens: the refresh token first, then both attempted,
    /// reporting the first refusal. The labels go to `defaults` only when
    /// everything was kept.
    func record(_ body: [String: Any]) -> RecordedSession {
        var failure: AccountError?
        var refreshSafe = body["refreshToken"] == nil
        if let refresh = body["refreshToken"] as? String {
            let status = write(Account.refreshKey, refresh)
            refreshSafe = status == errSecSuccess
            if status == errSecSuccess {
                pendingRefresh = nil
            } else {
                pendingRefresh = refresh
                failure = .keychainWriteFailed(status: Int(status))
            }
        }
        if let access = body["accessToken"] as? String {
            let status = write(Account.accessKey, access)
            if status != errSecSuccess, failure == nil {
                failure = .keychainWriteFailed(status: Int(status))
            }
        }
        if failure != nil { return RecordedSession(failure: failure, refreshSafe: refreshSafe) }
        var recorded = RecordedSession(refreshSafe: refreshSafe)
        if let id = body["userId"] as? String, !id.isEmpty {
            recorded.userId = id
            defaults.set(id, forKey: "account.userId")
        }
        if let mail = body["email"] as? String, !mail.isEmpty {
            recorded.email = mail
            defaults.set(mail, forKey: "account.email")
        }
        return recorded
    }

    /// Both items and the pending token, before a sign-in writes anything.
    func snapshot() -> SessionSnapshot {
        SessionSnapshot(
            access: read(Account.accessKey), refresh: read(Account.refreshKey),
            pendingRefresh: pendingRefresh)
    }

    /// A sign-in that failed after writing: put back the session that was
    /// saved before it, or delete what it wrote if there was none, so no
    /// orphan refresh token is left and no valid session is lost.
    func restore(_ before: SessionSnapshot) {
        pendingRefresh = before.pendingRefresh
        for (key, value) in [(Account.accessKey, before.access), (Account.refreshKey, before.refresh)] {
            if let value { _ = write(key, value) } else { delete(key) }
        }
    }

    /// Sign-out.
    func forget() { pendingRefresh = nil }
}

extension Account {
    /// Trade a sign-in code for a session and keep it, or leave nothing behind.
    ///
    /// A 401 here is the sign-in being refused, not a session that survived a
    /// refresh, so it gets its own error.
    static func completingSignIn(
        request: () async throws -> [String: Any],
        keeper: TokenKeeper
    ) async throws -> RecordedSession {
        let before = keeper.snapshot()
        let body: [String: Any]
        do {
            body = try await request()
        } catch AccountError.unauthorized {
            throw AccountError.signInRefused
        }
        let recorded = keeper.record(body)
        if let failure = recorded.failure {
            keeper.restore(before)
            throw failure
        }
        return recorded
    }
}
