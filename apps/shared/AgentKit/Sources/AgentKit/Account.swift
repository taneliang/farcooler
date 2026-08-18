import AuthenticationServices
import CryptoKit
import Foundation
import SwiftUI

/// Who you are, so a runner you own can reach a phone you carry.
///
/// This is the ONLY place in Far Cooler that knows about WorkOS, and it lives in
/// the apps rather than in the daemon on purpose. A daemon signs in to nothing:
/// it holds an opaque token this app asked the relay for and handed over ssh, so
/// a headless Linux box never needs a browser and a stolen daemon token is worth
/// exactly one thing — notifying the account it was stolen from.
///
/// The app carries a WorkOS client id, which is public by design. It never
/// carries the API key. The code exchange happens on the relay because that is
/// the single step that needs a secret, and a secret in an open-source repo or
/// an unzippable app bundle is not a secret.
@MainActor
public final class Account: NSObject, ObservableObject {
    public static let shared = Account()

    /// Signed in, and as whom.
    @Published public private(set) var email: String = ""
    @Published public private(set) var userId: String = ""
    @Published public private(set) var signingIn = false
    @Published public private(set) var lastError: String?

    public var isSignedIn: Bool { !userId.isEmpty }

    /// Where the relay lives. A setting so self-hosting is configuration rather
    /// than a fork, and so a development build can point at `wrangler dev`.
    public var relay: String {
        get { defaults.string(forKey: "account.relay") ?? Account.defaultRelay }
        set { defaults.set(newValue, forKey: "account.relay") }
    }

    /// The relay this build's channel talks to.
    ///
    /// One relay per channel, the same partition the bundle identifier follows.
    /// They are separate deployments with separate databases and separate
    /// WorkOS environments, so signing in on the beta is a different account
    /// from signing in on the release — which it already was, since the WorkOS
    /// user id is the account id and each environment issues its own.
    ///
    /// Release's URL is unchanged and must stay that way: it is compiled into
    /// binaries in the App Store, which cannot be told a new one for days.
    public static var defaultRelay: String {
        switch AppVersion.channel {
        case "stable": return "https://relay.farcooler.com"
        case "preview": return "https://relay-preview.farcooler.com"
        case "canary": return "https://relay-canary.farcooler.com"
        // Anything unstamped is a local build — see `AppVersion.channel`, which
        // defaults the same way and for the same reason.
        default: return "https://relay-local.farcooler.com"
        }
    }

    /// The AuthKit client id for this project. Public: it names the app, not the
    /// bearer, and every OAuth public client ships one.
    public var clientID: String {
        defaults.string(forKey: "account.clientID") ?? Account.bundledClientID
    }

    /// Overridable through Info.plist so a fork can point at its own WorkOS
    /// project without editing source.
    private static var bundledClientID: String {
        Bundle.main.object(forInfoDictionaryKey: "FarCoolerWorkOSClientID") as? String ?? ""
    }

    private let defaults = UserDefaults.standard
    private var session: ASWebAuthenticationSession?

    private override init() {
        super.init()
        // Before anything reads them. An earlier build put both tokens in
        // UserDefaults; this moves them and deletes the plaintext.
        TokenStore.migrateFromDefaults([Self.accessKey, Self.refreshKey], defaults: defaults)
        userId = defaults.string(forKey: "account.userId") ?? ""
        email = defaults.string(forKey: "account.email") ?? ""
    }

    /// Credentials, so: Keychain. Labels stay in UserDefaults — see TokenStore.
    private static let accessKey = "account.access"
    private static let refreshKey = "account.refresh"

    // MARK: - Signing in

    /// Open AuthKit and come back with a session.
    ///
    /// `ASWebAuthenticationSession` rather than an embedded web view: it is the
    /// only way to reach the system's existing sign-in state, it is what App
    /// Review expects, and — the reason it matters here — an embedded view would
    /// mean this app could read the password, which is the thing outsourcing
    /// auth was meant to avoid.
    public func signIn() async {
        guard !clientID.isEmpty else {
            lastError = "This build has no WorkOS client id. Set FarCoolerWorkOSClientID."
            return
        }
        signingIn = true
        lastError = nil
        defer { signingIn = false }

        // PKCE, because the redirect returns through a custom URL scheme that
        // any app on the device can claim. Without it, an app that registered
        // the same scheme could take the code and become you.
        guard let verifier = Self.randomVerifier(), let challenge = Self.challenge(for: verifier)
        else {
            lastError = "Could not start sign-in."
            return
        }
        // Bound to this sign-in, so a callback that did not come from it is
        // rejected. PKCE already defeats the practical code-injection attack —
        // an injected code was issued against someone else's challenge — but
        // `farcooler://` is a scheme any app on the device may claim, and a
        // flow with no request binding of its own has nothing to say about a
        // callback it never asked for.
        let state = Self.randomVerifier()

        var components = URLComponents(string: "https://api.workos.com/user_management/authorize")
        components?.queryItems = [
            .init(name: "client_id", value: clientID),
            .init(name: "redirect_uri", value: Self.redirectURI),
            .init(name: "response_type", value: "code"),
            .init(name: "provider", value: "authkit"),
            .init(name: "code_challenge", value: challenge),
            .init(name: "code_challenge_method", value: "S256"),
            .init(name: "state", value: state),
        ]
        guard let url = components?.url else { return }

        do {
            let callback = try await authenticate(url)
            let returned = URLComponents(url: callback, resolvingAgainstBaseURL: false)?.queryItems
            guard let code = returned?.first(where: { $0.name == "code" })?.value else {
                lastError = "Sign-in did not complete."
                return
            }
            guard returned?.first(where: { $0.name == "state" })?.value == state else {
                lastError = "Sign-in did not complete."
                return
            }
            try await exchange(code: code, verifier: verifier)
        } catch let error as ASWebAuthenticationSessionError where error.code == .canceledLogin {
            // Not an error. Someone closing the sheet is someone changing their
            // mind, and telling them they failed at it is obnoxious.
        } catch {
            lastError = error.localizedDescription
        }
    }

    /// Forget the session on this device.
    ///
    /// Paired runners keep working: they hold tokens, not your session, and
    /// signing out of a phone should not silence the fleet. `farcooler push
    /// forget` or the app's revoke button is how a runner is unpaired.
    public func signOut() async {
        // Told to the relay first, while there is still a token to tell it
        // with. Clearing locally alone left the refresh token valid at WorkOS
        // until natural expiry — so anyone who had lifted it kept minting
        // sessions after the user believed they had signed out.
        if let refresh = TokenStore.read(Self.refreshKey) {
            _ = try? await post("/v1/auth/logout", ["refreshToken": refresh])
        }
        clearSession()
    }

    // MARK: - Talking to the relay

    /// A valid access token, refreshing first if the stored one has expired.
    ///
    /// Refresh happens through the relay for the same reason the exchange does:
    /// WorkOS wants the API key on that call, and an app that could refresh
    /// alone would be an app carrying the key.
    public func accessToken(forceRefresh: Bool = false) async -> String? {
        guard let refresh = TokenStore.read(Self.refreshKey) else { return nil }
        if !forceRefresh, let access = TokenStore.read(Self.accessKey),
            let expiry = Self.jwtExpiry(access), expiry.timeIntervalSinceNow > 60
        {
            return access
        }
        return await Self.refreshingAccessToken(
            refreshToken: refresh,
            request: { try await self.post("/v1/auth/refresh", ["refreshToken": $0]) },
            store: { self.store($0) })
    }

    /// Refresh without inferring anything destructive from failure.
    ///
    /// The request can fail because the relay is unavailable, WorkOS is
    /// unavailable, or the refresh token was rejected. Those cases are
    /// indistinguishable here, so none of them is permission to erase a
    /// session. Only an explicit Sign Out action clears local identity.
    static func refreshingAccessToken(
        refreshToken: String,
        request: (String) async throws -> [String: Any],
        store: ([String: Any]) -> Void
    ) async -> String? {
        guard let body = try? await request(refreshToken) else { return nil }
        store(body)
        return body["accessToken"] as? String
    }

    /// Tell the relay where to reach this device, and what version is asking.
    ///
    /// Returns whether it worked, which it did not used to. The failure mode is
    /// the product's central promise going quietly missing: the device is never
    /// filed, the daemon's pushes reach zero addresses, and the settings screen
    /// goes on saying "Notifications can reach this device."
    @discardableResult
    public func registerDevice(
        pushToken: String,
        platform: String,
        label: String,
        environment: String,
        liveActivityStartToken: String? = nil
    ) async -> Bool {
        var payload: [String: Any] = [
            "pushToken": pushToken, "platform": platform, "label": label,
            // So the devices screen can show which of your runners is
            // behind, without anyone having to go and look.
            "version": AppVersion.reported,
            // Which APNs the relay has to talk to for this token. A build
            // installed from Xcode gets a sandbox token and the App Store build
            // of the same source gets a production one; sending either to the
            // wrong host is answered with `BadDeviceToken` and nothing else, so
            // the phone stays silent and the relay records a delivery it thinks
            // it attempted correctly.
            "environment": environment,
        ]
        // Omitted rather than sent as null when there is none, so the relay's
        // COALESCE keeps a token it already has. The Mac has no Live Activities
        // and a device can register before ActivityKit has handed one over —
        // neither should erase a good token.
        if let liveActivityStartToken {
            payload["liveActivityStartToken"] = liveActivityStartToken
        }
        let body = await authenticatedPost("/v1/devices", payload)
        return body != nil
    }

    /// File the push token for one running Live Activity, or clear it.
    ///
    /// Separate from the device token and from the push-to-start token, because
    /// ActivityKit issues three different things: one that addresses the device,
    /// one that can create an activity, and one per activity that can update or
    /// end that specific card. Only the third can take a card off the lock
    /// screen, and the relay cannot mint it — the phone has to send it up after
    /// the activity starts.
    ///
    /// `updateToken: nil` says the activity is over and the relay should forget
    /// it. Without that, an ended card leaves a row that the next `done` would
    /// try to end again, pushing to a token APNs has already retired.
    @discardableResult
    public func registerActivityToken(
        terminal: String, updateToken: String?, environment: String
    ) async -> Bool {
        // NSNull rather than leaving the key out or passing the Optional along:
        // JSONSerialization throws on a bare `Optional.none`, and an absent key
        // reads to the relay as "no change" — but clearing is the entire point
        // of the nil case.
        let update: Any = updateToken.map { $0 as Any } ?? NSNull()
        let body = await authenticatedPost(
            "/v1/devices/activity",
            ["terminal": terminal, "updateToken": update, "environment": environment])
        return body != nil
    }

    /// Ask for a token that lets one runner notify this account.
    ///
    /// Returned once and stored on the relay only as a hash, so this is the only
    /// moment it exists in readable form — hand it straight to the runner.
    public func pairDaemon(label: String) async -> String? {
        let body = await authenticatedPost("/v1/daemons", ["label": label])
        return body?["token"] as? String
    }

    /// Everything this account has registered, for the management screen.
    public func fetchRegistrations() async -> Registrations? {
        guard let body = await authenticatedPost("/v1/account", [:]) else { return nil }
        let devices = (body["devices"] as? [[String: Any]] ?? []).map {
            Registration(
                id: $0["id"] as? String ?? "",
                label: $0["label"] as? String ?? "Device",
                detail: ($0["platform"] as? String) == "fcm" ? "Android" : "Apple",
                version: $0["version"] as? String,
                at: $0["updatedAt"] as? Double)
        }
        // `machines` is the relay's own JSON key. It names a paired daemon —
        // a runner — and stays spelled that way because the relay's API is a
        // contract, not a word this app gets to choose.
        let runners = (body["machines"] as? [[String: Any]] ?? []).map {
            Registration(
                id: $0["id"] as? String ?? "",
                label: $0["label"] as? String ?? "Runner",
                detail: "Paired",
                version: $0["version"] as? String,
                at: ($0["lastSeenAt"] as? Double) ?? ($0["createdAt"] as? Double))
        }
        return Registrations(devices: devices, runners: runners)
    }

    /// Stop notifying a device, or stop a runner notifying anything.
    ///
    /// Revoking here rather than on the runner is the case that matters: a
    /// laptop you no longer have is exactly the one you cannot run a command on.
    public func revoke(_ registration: Registration, kind: RegistrationKind) async -> Bool {
        let path = kind == .device ? "/v1/devices/revoke" : "/v1/daemons/revoke"
        return await authenticatedPost(path, ["id": registration.id]) != nil
    }

    // MARK: - Plumbing

    private func exchange(code: String, verifier: String) async throws {
        let body = try await post(
            "/v1/auth/token", ["code": code, "verifier": verifier])
        store(body)
    }

    private func store(_ body: [String: Any]) {
        if let access = body["accessToken"] as? String {
            TokenStore.write(Self.accessKey, access)
        }
        if let refresh = body["refreshToken"] as? String {
            TokenStore.write(Self.refreshKey, refresh)
        }
        if let id = body["userId"] as? String, !id.isEmpty {
            userId = id
            defaults.set(id, forKey: "account.userId")
        }
        if let mail = body["email"] as? String, !mail.isEmpty {
            email = mail
            defaults.set(mail, forKey: "account.email")
        }
    }

    /// Make one authenticated call, refreshing once if the relay rejects the
    /// cached access token.
    ///
    /// Expiry is only the common reason a bearer stops working. The relay can
    /// reject one earlier after key rotation, revocation, or stricter claim
    /// validation. Treating that 401 like a dead server produced the Devices
    /// screen's "Couldn’t load devices and runners" error while the relay was healthy,
    /// and kept sending the same rejected token on every retry.
    private func authenticatedPost(
        _ path: String, _ body: [String: Any]
    ) async -> [String: Any]? {
        guard let token = await accessToken() else { return nil }
        return await Self.retryingUnauthorized(
            token: token,
            refresh: { await self.accessToken(forceRefresh: true) },
            request: { try await self.post(path, body, bearer: $0) })
    }

    /// Retry an authenticated operation once with a freshly minted bearer.
    /// Kept separate from HTTP so the retry limit and session behavior are
    /// regression-testable without a live identity provider.
    static func retryingUnauthorized<Value>(
        token: String,
        refresh: () async -> String?,
        request: (String) async throws -> Value
    ) async -> Value? {
        do {
            return try await request(token)
        } catch AccountError.unauthorized {
            guard let refreshed = await refresh() else { return nil }
            do {
                return try await request(refreshed)
            } catch AccountError.unauthorized {
                // The relay still refused the request, but it does not own the
                // app's local identity. Keep the session intact so a transient
                // account or configuration problem cannot sign the user out.
                return nil
            } catch {
                return nil
            }
        } catch {
            return nil
        }
    }

    @discardableResult
    private func post(
        _ path: String, _ body: [String: Any], bearer: String? = nil
    ) async throws -> [String: Any] {
        // Not force-unwrapped: `relay` is a setting anyone can type into, and
        // a stray space in it would crash the app on the next sign-in rather
        // than surface as a relay that would not answer.
        guard let url = URL(string: relay + path) else { throw AccountError.relayRefused }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        if let bearer { request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization") }
        request.httpBody = try JSONSerialization.data(withJSONObject: body)

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let statusCode = (response as? HTTPURLResponse)?.statusCode else {
            throw AccountError.relayRefused
        }
        switch Self.responseAction(for: statusCode) {
        case .accept:
            guard let parsed = try JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { throw AccountError.relayRefused }
            return parsed
        case .refreshSession:
            throw AccountError.unauthorized
        case .fail:
            throw AccountError.relayRefused
        }
    }

    /// The only HTTP failure the app can repair by changing its request.
    static func responseAction(for statusCode: Int) -> AccountResponseAction {
        switch statusCode {
        case 200: return .accept
        case 401: return .refreshSession
        default: return .fail
        }
    }

    private func clearSession() {
        userId = ""
        email = ""
        defaults.removeObject(forKey: "account.userId")
        defaults.removeObject(forKey: "account.email")
        TokenStore.delete(Self.accessKey)
        TokenStore.delete(Self.refreshKey)
    }

    private func authenticate(_ url: URL) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(
                url: url, callbackURLScheme: Self.scheme,
                completionHandler: Self.authenticationCompletion(for: continuation))
            session.presentationContextProvider = self
            // Deliberately NOT ephemeral: someone signing in on their Mac and
            // then their phone should meet a browser that already knows them.
            session.prefersEphemeralWebBrowserSession = false
            session.start()
            self.session = session
        }
    }

    /// Authentication Services replies on Safari's XPC queue, not the actor
    /// that started the session. Building this closure inside `authenticate`
    /// made it inherit `Account`'s main-actor isolation, so Swift trapped before
    /// the closure body could resume the continuation. Construct it from a
    /// nonisolated context; checked continuations are safe to resume there.
    nonisolated static func authenticationCompletion(
        for continuation: CheckedContinuation<URL, any Error>
    ) -> ASWebAuthenticationSession.CompletionHandler {
        { callback, error in
            if let callback {
                continuation.resume(returning: callback)
            } else {
                continuation.resume(throwing: error ?? AccountError.relayRefused)
            }
        }
    }

    /// Read a JWT's `exp` without verifying it.
    ///
    /// Verification is the relay's job — it has the JWKS. This only decides
    /// whether to bother sending a token that is already stale, and a forged
    /// expiry buys nothing but an extra refresh.
    static func jwtExpiry(_ token: String) -> Date? {
        let parts = token.split(separator: ".")
        guard parts.count == 3 else { return nil }
        var payload = String(parts[1]).replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        while payload.count % 4 != 0 { payload += "=" }
        guard let data = Data(base64Encoded: payload),
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let exp = json["exp"] as? Double
        else { return nil }
        return Date(timeIntervalSince1970: exp)
    }

    /// The scheme this build registered, read back from its own bundle.
    ///
    /// Not a constant, and not the channel mapped a second time here: the
    /// Info.plist is what the OS actually routes on, so asking it is the only
    /// way this cannot disagree with where a callback will arrive. The mapping
    /// itself lives in `scripts/version.sh`, and the plists are stamped from
    /// it — see there for why one shared scheme across channels is a collision
    /// rather than a shared name.
    ///
    /// The fallback cannot be reached by a real build, and would not matter if
    /// it were: an app that registered no scheme cannot receive a callback at
    /// all, so what it asks WorkOS to redirect to changes nothing.
    private static let scheme: String = {
        let types = Bundle.main.object(forInfoDictionaryKey: "CFBundleURLTypes") as? [[String: Any]]
        return (types?.first?["CFBundleURLSchemes"] as? [String])?.first ?? "farcooler"
    }()

    private static var redirectURI: String { "\(scheme)://auth" }

    /// Nil rather than zeros if the system has no randomness for us.
    ///
    /// The discarded status was the bug: on failure `bytes` stays the all-zero
    /// buffer it was initialized with, the verifier becomes a fixed publicly
    /// known constant, sign-in still appears to work, and PKCE silently
    /// protects nothing — which is the one thing standing between a custom URL
    /// scheme any app can claim and account takeover.
    static func randomVerifier() -> String? {
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
            return nil
        }
        return Data(bytes).base64URLEncoded
    }

    static func challenge(for verifier: String) -> String? {
        guard let data = verifier.data(using: .ascii) else { return nil }
        return Data(SHA256.hash(data: data)).base64URLEncoded
    }
}

enum AccountResponseAction: Equatable {
    case accept
    case refreshSession
    case fail
}

public enum AccountError: Error {
    case relayRefused
    case unauthorized
}

/// One row on the management screen.
public struct Registration: Identifiable, Hashable, Sendable {
    public let id: String
    public let label: String
    public let detail: String
    /// What this thing last reported running, or nil if it never has.
    ///
    /// The point of the whole devices screen once there is more than one
    /// runner: seeing which one is behind without opening each of them. Nil is
    /// not an error — a paired runner that has never had an agent get stuck
    /// has never had a reason to talk to the relay.
    public let version: String?
    /// Last heard from, as a Unix millisecond stamp — the only thing that tells
    /// you whether a row is a runner you still use or one you forgot about.
    public let at: Double?

    /// The second line: what it is, what it runs, when it last spoke.
    public var subtitle: String {
        [detail, version, lastSeen.isEmpty ? nil : lastSeen]
            .compactMap { $0 }
            .joined(separator: " · ")
    }

    public var lastSeen: String {
        guard let at else { return "" }
        let date = Date(timeIntervalSince1970: at / 1000)
        return date.formatted(.relative(presentation: .named))
    }
}

public struct Registrations: Sendable {
    public let devices: [Registration]
    public let runners: [Registration]
}

public enum RegistrationKind: Sendable {
    case device
    case runner
}

extension Account: ASWebAuthenticationPresentationContextProviding {
    public nonisolated func presentationAnchor(for session: ASWebAuthenticationSession)
        -> ASPresentationAnchor
    {
        MainActor.assumeIsolated {
            #if os(macOS)
                return NSApplication.shared.keyWindow ?? NSApplication.shared.windows.first
                    ?? ASPresentationAnchor()
            #else
                let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene
                return scene?.keyWindow ?? ASPresentationAnchor()
            #endif
        }
    }
}

extension Data {
    /// base64url, which is what OAuth means every time it says base64.
    fileprivate var base64URLEncoded: String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
