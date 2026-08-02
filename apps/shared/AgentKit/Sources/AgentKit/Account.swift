import AuthenticationServices
import CryptoKit
import Foundation
import SwiftUI

/// Who you are, so a machine you own can reach a phone you carry.
///
/// This is the ONLY place in Overnight that knows about WorkOS, and it lives in
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

    public static let defaultRelay = "https://relay.overnight.sh"

    /// The AuthKit client id for this project. Public: it names the app, not the
    /// bearer, and every OAuth public client ships one.
    public var clientID: String {
        defaults.string(forKey: "account.clientID") ?? Account.bundledClientID
    }

    /// Overridable through Info.plist so a fork can point at its own WorkOS
    /// project without editing source.
    private static var bundledClientID: String {
        Bundle.main.object(forInfoDictionaryKey: "OvernightWorkOSClientID") as? String ?? ""
    }

    private let defaults = UserDefaults.standard
    private var session: ASWebAuthenticationSession?

    private override init() {
        super.init()
        userId = defaults.string(forKey: "account.userId") ?? ""
        email = defaults.string(forKey: "account.email") ?? ""
    }

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
            lastError = "This build has no WorkOS client id. Set OvernightWorkOSClientID."
            return
        }
        signingIn = true
        lastError = nil
        defer { signingIn = false }

        // PKCE, because the redirect returns through a custom URL scheme that
        // any app on the device can claim. Without it, an app that registered
        // the same scheme could take the code and become you.
        let verifier = Self.randomVerifier()
        guard let challenge = Self.challenge(for: verifier) else {
            lastError = "Could not start sign-in."
            return
        }

        var components = URLComponents(string: "https://api.workos.com/user_management/authorize")
        components?.queryItems = [
            .init(name: "client_id", value: clientID),
            .init(name: "redirect_uri", value: Self.redirectURI),
            .init(name: "response_type", value: "code"),
            .init(name: "provider", value: "authkit"),
            .init(name: "code_challenge", value: challenge),
            .init(name: "code_challenge_method", value: "S256"),
        ]
        guard let url = components?.url else { return }

        do {
            let callback = try await authenticate(url)
            guard
                let code = URLComponents(url: callback, resolvingAgainstBaseURL: false)?
                    .queryItems?.first(where: { $0.name == "code" })?.value
            else {
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
    /// Paired machines keep working: they hold tokens, not your session, and
    /// signing out of a phone should not silence the fleet. `overnight push
    /// forget` or the app's revoke button is how a machine is unpaired.
    public func signOut() {
        userId = ""
        email = ""
        for key in ["account.userId", "account.email", "account.access", "account.refresh"] {
            defaults.removeObject(forKey: key)
        }
    }

    // MARK: - Talking to the relay

    /// A valid access token, refreshing first if the stored one has expired.
    ///
    /// Refresh happens through the relay for the same reason the exchange does:
    /// WorkOS wants the API key on that call, and an app that could refresh
    /// alone would be an app carrying the key.
    public func accessToken() async -> String? {
        guard let refresh = defaults.string(forKey: "account.refresh") else { return nil }
        if let access = defaults.string(forKey: "account.access"),
            let expiry = jwtExpiry(access), expiry.timeIntervalSinceNow > 60
        {
            return access
        }
        let body = try? await post("/v1/auth/refresh", ["refreshToken": refresh], authorised: false)
        guard let body else {
            // A refresh token that no longer works means the session is over,
            // and leaving a dead one in place makes every later call fail
            // silently instead of showing a sign-in button.
            signOut()
            return nil
        }
        store(body)
        return body["accessToken"] as? String
    }

    /// Tell the relay where to reach this device.
    public func registerDevice(pushToken: String, platform: String, label: String) async {
        guard let token = await accessToken() else { return }
        _ = try? await post(
            "/v1/devices",
            ["pushToken": pushToken, "platform": platform, "label": label],
            bearer: token)
    }

    /// Ask for a token that lets one machine notify this account.
    ///
    /// Returned once and stored on the relay only as a hash, so this is the only
    /// moment it exists in readable form — hand it straight to the machine.
    public func pairDaemon(label: String) async -> String? {
        guard let token = await accessToken() else { return nil }
        let body = try? await post("/v1/daemons", ["label": label], bearer: token)
        return body?["token"] as? String
    }

    /// Everything this account has registered, for the management screen.
    public func fetchRegistrations() async -> Registrations? {
        guard let token = await accessToken() else { return nil }
        guard let body = try? await post("/v1/account", [:], bearer: token) else { return nil }
        let devices = (body["devices"] as? [[String: Any]] ?? []).map {
            Registration(
                id: $0["id"] as? String ?? "",
                label: $0["label"] as? String ?? "Device",
                detail: ($0["platform"] as? String) == "fcm" ? "Android" : "Apple",
                at: $0["updatedAt"] as? Double)
        }
        let machines = (body["machines"] as? [[String: Any]] ?? []).map {
            Registration(
                id: $0["id"] as? String ?? "",
                label: $0["label"] as? String ?? "Machine",
                detail: "Paired",
                at: ($0["lastSeenAt"] as? Double) ?? ($0["createdAt"] as? Double))
        }
        return Registrations(devices: devices, machines: machines)
    }

    /// Stop notifying a device, or stop a machine notifying anything.
    ///
    /// Revoking here rather than on the machine is the case that matters: a
    /// laptop you no longer have is exactly the one you cannot run a command on.
    public func revoke(_ registration: Registration, kind: RegistrationKind) async -> Bool {
        guard let token = await accessToken() else { return false }
        let path = kind == .device ? "/v1/devices/revoke" : "/v1/daemons/revoke"
        return (try? await post(path, ["id": registration.id], bearer: token)) != nil
    }

    // MARK: - Plumbing

    private func exchange(code: String, verifier: String) async throws {
        let body = try await post(
            "/v1/auth/token", ["code": code, "verifier": verifier], authorised: false)
        store(body)
    }

    private func store(_ body: [String: Any]) {
        if let access = body["accessToken"] as? String {
            defaults.set(access, forKey: "account.access")
        }
        if let refresh = body["refreshToken"] as? String {
            defaults.set(refresh, forKey: "account.refresh")
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

    @discardableResult
    private func post(
        _ path: String, _ body: [String: Any], bearer: String? = nil, authorised: Bool = true
    ) async throws -> [String: Any] {
        var request = URLRequest(url: URL(string: relay + path)!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        if let bearer { request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization") }
        request.httpBody = try JSONSerialization.data(withJSONObject: body)

        let (data, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200,
            let parsed = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            throw AccountError.relayRefused
        }
        _ = authorised
        return parsed
    }

    private func authenticate(_ url: URL) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(
                url: url, callbackURLScheme: Self.scheme
            ) { callback, error in
                if let callback {
                    continuation.resume(returning: callback)
                } else {
                    continuation.resume(throwing: error ?? AccountError.relayRefused)
                }
            }
            session.presentationContextProvider = self
            // Deliberately NOT ephemeral: someone signing in on their Mac and
            // then their phone should meet a browser that already knows them.
            session.prefersEphemeralWebBrowserSession = false
            session.start()
            self.session = session
        }
    }

    /// Read a JWT's `exp` without verifying it.
    ///
    /// Verification is the relay's job — it has the JWKS. This only decides
    /// whether to bother sending a token that is already stale, and a forged
    /// expiry buys nothing but an extra refresh.
    private func jwtExpiry(_ token: String) -> Date? {
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

    private static let scheme = "overnight"
    private static let redirectURI = "overnight://auth"

    private static func randomVerifier() -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        _ = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        return Data(bytes).base64URLEncoded
    }

    private static func challenge(for verifier: String) -> String? {
        guard let data = verifier.data(using: .ascii) else { return nil }
        return Data(SHA256.hash(data: data)).base64URLEncoded
    }
}

public enum AccountError: Error {
    case relayRefused
}

/// One row on the management screen.
public struct Registration: Identifiable, Hashable, Sendable {
    public let id: String
    public let label: String
    public let detail: String
    /// Last heard from, as a Unix millisecond stamp — the only thing that tells
    /// you whether a row is a machine you still use or one you forgot about.
    public let at: Double?

    public var lastSeen: String {
        guard let at else { return "" }
        let date = Date(timeIntervalSince1970: at / 1000)
        return date.formatted(.relative(presentation: .named))
    }
}

public struct Registrations: Sendable {
    public let devices: [Registration]
    public let machines: [Registration]
}

public enum RegistrationKind: Sendable {
    case device
    case machine
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
