import AuthenticationServices
import CryptoKit
import Foundation
import SwiftUI
import os

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

    /// The last relay call that produced no answer, and why.
    ///
    /// Set by every authenticated call that fails and cleared by the next one
    /// that works, so what it holds is a problem still happening rather than
    /// one that happened once. Settings ▸ Account reads it; nothing else does,
    /// and it draws nothing while there is nothing wrong.
    ///
    /// See ``RelayDiagnostic`` for why it can carry no credential.
    @Published public private(set) var lastRelayFailure: RelayDiagnostic?

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
                lastError = AccountError.signInIncomplete.message
                return
            }
            guard returned?.first(where: { $0.name == "state" })?.value == state else {
                lastError = AccountError.signInIncomplete.message
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
        try? await credential(forceRefresh: forceRefresh).get()
    }

    /// The same token, or the reason there isn't one.
    ///
    /// This is the version everything inside this file uses. `nil` was the
    /// whole defect one level down: "the Keychain holds nothing", "the relay
    /// refused the refresh token" and "this laptop has no network" are three
    /// different things to tell somebody, and an Optional makes them one.
    func credential(forceRefresh: Bool = false) async -> Result<String, AccountError> {
        guard let refresh = TokenStore.read(Self.refreshKey) else { return .failure(.notSignedIn) }
        if !forceRefresh, let access = TokenStore.read(Self.accessKey),
            let expiry = Self.jwtExpiry(access), expiry.timeIntervalSinceNow > 60
        {
            return .success(access)
        }
        // One refresh at a time, however many callers want one.
        //
        // A WorkOS refresh token is SINGLE USE: the response to
        // `/v1/auth/refresh` carries a new one and retires the token that was
        // sent. So two callers arriving together — which is the normal way this
        // is reached, since a screen appearing fires several relay calls at
        // once and an expired token fails all of them — both read the same
        // stored token and both spend it. The first wins; the rest get an
        // error for a token that was valid when they read it. Observed live as
        // three `/v1/auth/refresh` inside two seconds.
        //
        // `@MainActor` is not enough on its own to prevent that, and it is
        // worth being clear why: it serializes the synchronous stretches, not
        // the awaits. Both callers can pass the expiry check above and both
        // suspend inside the request. What it DOES give is that the check and
        // the assignment below happen with no suspension between them, so this
        // needs no lock — the slot cannot be read as empty by two callers.
        if let inFlight = refreshInFlight {
            return await inFlight.value
        }
        let task = Task { @MainActor [self] () -> Result<String, AccountError> in
            // Cleared by the task itself rather than by whoever created it, so
            // the slot is empty the moment the answer exists. A later caller
            // that genuinely needs a NEW refresh — a `forceRefresh` after a 401
            // — must not join a task that has already finished with the token
            // the relay just rejected.
            defer { refreshInFlight = nil }
            return await Self.refreshingAccessToken(
                refreshToken: refresh,
                request: { try await self.post("/v1/auth/refresh", ["refreshToken": $0]) },
                store: { self.store($0) })
        }
        refreshInFlight = task
        return await task.value
    }

    /// The refresh in progress, if there is one. See ``accessToken(forceRefresh:)``.
    ///
    /// Not `@Published`: nothing draws it, and publishing it would send the
    /// object through `objectWillChange` twice per refresh for no reader.
    private var refreshInFlight: Task<Result<String, AccountError>, Never>?

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
    ) async -> Result<String, AccountError> {
        let body: [String: Any]
        do {
            body = try await request(refreshToken)
        } catch {
            // The cause travels; the session does not move. `store` is not
            // reached and nothing local is cleared, so an unreachable relay
            // still leaves a signed-in app.
            return .failure(named(error))
        }
        store(body)
        guard let access = body["accessToken"] as? String else {
            return .failure(.malformedResponse)
        }
        return .success(access)
    }

    /// Tell the relay where to reach this device, and what version is asking.
    ///
    /// Returns whether it worked, which it did not used to. The failure mode is
    /// the product's central promise going quietly missing: the device is never
    /// filed, the daemon's pushes reach zero addresses, and the settings screen
    /// goes on saying "Notifications can reach this device."
    ///
    /// And WHY it did not work, which it also did not used to: the reason is
    /// the difference between "sign in again" and "you are offline", and a Bool
    /// could say neither.
    @discardableResult
    public func registerDevice(
        pushToken: String,
        platform: String,
        label: String,
        environment: String,
        liveActivityStartToken: String? = nil,
        notifyOnDone: Bool = true
    ) async -> Result<Void, AccountError> {
        var payload: [String: Any] = [
            "pushToken": pushToken, "platform": platform, "label": label,
            // "When an agent finishes or fails", so the toggle reaches the
            // pushes too. It used to be read only by this app's own `Notifier`,
            // which runs when the app is running — the case the product is not
            // about. With the phone asleep the banner comes from the relay, and
            // the relay had never heard of the setting: silence with the app
            // open, banners with the phone in a pocket.
            //
            // Always sent, never omitted. The relay COALESCEs an absent field
            // into what it already holds, which is right for a build too old to
            // know about this and wrong for one turning the setting back ON.
            "notifyOnDone": notifyOnDone,
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
        return await authenticatedPost("/v1/devices", payload).map { _ in () }
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
    ///
    /// `dismissed` says WHO ended it, and only the phone can know. A card the
    /// person swiped away is a refusal the relay has to remember for a while: it
    /// pushes a working card every ten seconds, so a dismissal it forgot became
    /// a card back on the lock screen before the person had put the phone down.
    /// A card that merely ended — the relay's own `end`, iOS retiring a stale
    /// one — carries no such refusal and is reported as `false`.
    @discardableResult
    public func registerActivityToken(
        terminal: String, updateToken: String?, environment: String, dismissed: Bool = false
    ) async -> Result<Void, AccountError> {
        // NSNull rather than leaving the key out or passing the Optional along:
        // JSONSerialization throws on a bare `Optional.none`, and an absent key
        // reads to the relay as "no change" — but clearing is the entire point
        // of the nil case.
        let update: Any = updateToken.map { $0 as Any } ?? NSNull()
        // Nothing reads this result — the two callers in `LiveActivities` are
        // reporting a token, not asking a question. It carries the cause
        // anyway so there is ONE vocabulary here rather than a Bool for the
        // calls nobody watches and a reason for the ones somebody does; the
        // failure still reaches `lastRelayFailure` and the log either way.
        return await authenticatedPost(
            "/v1/devices/activity",
            [
                "terminal": terminal, "updateToken": update, "environment": environment,
                "dismissed": dismissed,
            ]
        ).map { _ in () }
    }

    /// Ask for a token that lets one runner notify this account.
    ///
    /// Returned once and stored on the relay only as a hash, so this is the only
    /// moment it exists in readable form — hand it straight to the runner.
    ///
    /// A `Result`, not an Optional. This call is the front door to the entire
    /// notification product — an account with no paired runner posts no agent
    /// state, so no Live Activity can ever be raised — and for as long as it
    /// answered `nil` the one sentence its caller could write was "Try signing
    /// in again", said with equal confidence to somebody signed out, somebody
    /// on a plane, and somebody whose relay was returning 500.
    public func pairDaemon(label: String) async -> Result<String, AccountError> {
        await authenticatedPost("/v1/daemons", ["label": label]).flatMap { body in
            // A 200 with no token in it is the relay answering something this
            // app cannot use. It is not a credential problem and must not be
            // reported as one.
            guard let token = body["token"] as? String, !token.isEmpty else {
                return .failure(.malformedResponse)
            }
            return .success(token)
        }
    }

    /// Everything this account has registered, for the management screen.
    public func fetchRegistrations() async -> Result<Registrations, AccountError> {
        await authenticatedPost("/v1/account", [:]).map { body in
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
    }

    /// Stop notifying a device, or stop a runner notifying anything.
    ///
    /// Revoking here rather than on the runner is the case that matters: a
    /// laptop you no longer have is exactly the one you cannot run a command on.
    @discardableResult
    public func revoke(_ registration: Registration, kind: RegistrationKind) async
        -> Result<Void, AccountError>
    {
        let path = kind == .device ? "/v1/devices/revoke" : "/v1/daemons/revoke"
        return await authenticatedPost(path, ["id": registration.id]).map { _ in () }
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
    ) async -> Result<[String: Any], AccountError> {
        let outcome = await Self.authenticating(
            credential: { await self.credential(forceRefresh: $0) },
            request: { try await self.post(path, body, bearer: $0) })
        note(outcome, path: path)
        return outcome
    }

    /// One authenticated call, from "find a credential" to "give up and say
    /// why".
    ///
    /// Static and closure-fed for the same reason ``retryingUnauthorized`` is:
    /// the ways this can fail are exactly the sentences somebody reads, and
    /// they have to be regression-testable without a relay or an identity
    /// provider. Every one of them leaves by a different door.
    static func authenticating<Value>(
        credential: (Bool) async -> Result<String, AccountError>,
        request: (String) async throws -> Value
    ) async -> Result<Value, AccountError> {
        switch await credential(false) {
        case .failure(let why):
            // Nothing to send. Told apart from a credential the relay refused,
            // because one of those is fixed by signing in and the other is
            // fixed by having a network.
            return .failure(why)
        case .success(let token):
            return await retryingUnauthorized(
                token: token,
                refresh: { await credential(true) },
                request: request)
        }
    }

    /// Remember, and log, what the relay just did.
    ///
    /// Cleared by a later success on the SAME call, so `lastRelayFailure` is a
    /// problem still happening rather than one that happened once — and, just
    /// as importantly, not erased by an unrelated call that worked. Settings
    /// ▸ Account loads through `/v1/account`; if any success cleared this, the
    /// screen would wipe the pairing failure somebody opened it to read, at the
    /// moment they opened it.
    private func note(_ outcome: Result<[String: Any], AccountError>, path: String) {
        switch outcome {
        case .success:
            if lastRelayFailure?.path == path { lastRelayFailure = nil }
        case .failure(let failure):
            let diagnostic = RelayDiagnostic(path: path, failure: failure, at: Date())
            lastRelayFailure = diagnostic
            // `.public` on purpose. Every component of `line` is a literal in
            // this file or an integer read off the transport — see
            // ``RelayDiagnostic`` — so the default redaction would hide the
            // only thing worth logging while protecting nothing.
            Self.log.error("\(diagnostic.line, privacy: .public)")
        }
    }

    /// Where a failure goes when nobody is looking at a screen.
    ///
    /// `os.Logger`, so it reaches Console and `log stream` with no build flag,
    /// no setting, and no file anybody has to remember to delete.
    private static let log = Logger(subsystem: "com.farcooler.agentkit", category: "relay")

    /// Retry an authenticated operation once with a freshly minted bearer.
    /// Kept separate from HTTP so the retry limit and session behavior are
    /// regression-testable without a live identity provider.
    ///
    /// Every exit carries its cause. It used to answer `nil` five different
    /// ways, and the caller's only honest option was to name none of them.
    static func retryingUnauthorized<Value>(
        token: String,
        refresh: () async -> Result<String, AccountError>,
        request: (String) async throws -> Value
    ) async -> Result<Value, AccountError> {
        do {
            return .success(try await request(token))
        } catch AccountError.unauthorized {
            switch await refresh() {
            case .failure(let why):
                // Why the REFRESH failed, not why the request did. A relay that
                // could not be reached during the refresh is not a session that
                // has ended, and saying "sign in again" to somebody on a train
                // sends them to a sign-in sheet that also cannot work.
                return .failure(why)
            case .success(let refreshed):
                do {
                    return .success(try await request(refreshed))
                } catch {
                    // The relay still refused, but it does not own the app's
                    // local identity. Keep the session intact so a transient
                    // account or configuration problem cannot sign the user
                    // out — nothing here touches `clearSession`.
                    return .failure(named(error))
                }
            }
        } catch {
            return .failure(named(error))
        }
    }

    /// Any thrown thing, named as precisely as this app can name it.
    ///
    /// The `catch` that used to be here threw the cause away. Anything that is
    /// not already an `AccountError` never reached the relay, so it is a
    /// transport failure — recorded by code, never by message.
    static func named(_ error: Error) -> AccountError {
        (error as? AccountError) ?? .unreachable(reason: transportReason(error))
    }

    /// A transport failure named by its code, never by its text.
    ///
    /// `localizedDescription` is prose other frameworks assemble, sometimes
    /// from the failing URL. This string is logged, so it is limited to values
    /// this file enumerates: a word, or a number.
    static func transportReason(_ error: Error) -> String {
        guard let urlError = error as? URLError else {
            return String(describing: type(of: error))
        }
        switch urlError.code {
        case .notConnectedToInternet: return "offline"
        case .cannotFindHost, .dnsLookupFailed: return "DNS"
        case .cannotConnectToHost: return "connection refused"
        case .timedOut: return "timed out"
        case .networkConnectionLost: return "connection lost"
        case .secureConnectionFailed, .serverCertificateUntrusted: return "TLS"
        case .dataNotAllowed: return "no data allowance"
        default: return "URLError \(urlError.errorCode)"
        }
    }

    @discardableResult
    private func post(
        _ path: String, _ body: [String: Any], bearer: String? = nil
    ) async throws -> [String: Any] {
        // Not force-unwrapped: `relay` is a setting anyone can type into, and
        // a stray space in it would crash the app on the next sign-in rather
        // than surface as a relay that would not answer.
        guard let url = URL(string: relay + path) else { throw AccountError.relayAddressInvalid }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        if let bearer { request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization") }
        request.httpBody = try JSONSerialization.data(withJSONObject: body)

        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch {
            // The request never completed. Told apart from every answer the
            // relay could have given, because "you are offline" and "the relay
            // refused you" are opposite instructions.
            throw AccountError.unreachable(reason: Self.transportReason(error))
        }
        guard let statusCode = (response as? HTTPURLResponse)?.statusCode else {
            throw AccountError.malformedResponse
        }
        if let failure = Self.failure(forStatus: statusCode) { throw failure }
        guard let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { throw AccountError.malformedResponse }
        return body
    }

    /// What one HTTP answer means, or nil when it is not a failure at all.
    ///
    /// The seam a test starts from, because the wire starts there too: a status
    /// code goes in and the cause a person will read about comes out, with no
    /// relay and no identity provider in between.
    static func failure(forStatus statusCode: Int) -> AccountError? {
        switch responseAction(for: statusCode) {
        case .accept: return nil
        case .refreshSession: return .unauthorized
        case .fail: return failure(for: statusCode)
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

    /// What a status code the app cannot repair actually means.
    ///
    /// The relay says almost nothing in its body, and that reticence is right:
    /// an error from D1 or WorkOS can carry a query or a token fragment, and
    /// the body goes to whoever asked. But the STATUS is not a secret, this
    /// client already has it, and it is the whole difference between "the relay
    /// is broken", "this build is asking for something that isn't there" and
    /// "you are asking too often".
    static func failure(for statusCode: Int) -> AccountError {
        switch statusCode {
        case 404: return .endpointMissing
        case 429: return .rateLimited
        case 500...599: return .relayFailed(status: statusCode)
        default: return .requestRefused(status: statusCode)
        }
    }

    private func clearSession() {
        userId = ""
        email = ""
        lastRelayFailure = nil
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
                continuation.resume(throwing: error ?? AccountError.signInIncomplete)
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

/// Why a call to the relay produced no answer.
///
/// One case per thing that can actually go wrong, because there used to be
/// none: no local token, a bearer the relay refused twice, a 500, a 404 and a
/// laptop with no network all arrived as `nil`, and the app turned that single
/// `nil` into a single sentence — "Try signing in again." That sentence was
/// right about one of the five, and there was no way, from the outside or from
/// the inside, to tell which time.
///
/// The relay itself will not say why it failed, and that is correct: its body
/// can carry a query or a token fragment out to whoever asked, so it answers a
/// bare `{"error":"internal"}` with a 500. It does not have to explain itself.
/// The CLIENT holds the status code and the transport error, and those are
/// enough to name every cause below without the relay giving anything up.
public enum AccountError: Error, Equatable, Sendable, LocalizedError {
    /// Nothing on this device to authenticate with.
    case notSignedIn
    /// The sign-in sheet closed without a callback.
    case signInIncomplete
    /// The relay setting is not a URL this app could send anything to.
    case relayAddressInvalid
    /// The request never completed: offline, DNS, TLS, a timeout. `reason` is a
    /// code for the log and never reaches a screen.
    case unreachable(reason: String)
    /// The relay answered, and would not take this credential — after a
    /// refresh. The one case where signing in again is the right advice.
    case unauthorized
    /// Too many auth requests, too fast. 429.
    case rateLimited
    /// The relay has no such endpoint. 404.
    case endpointMissing
    /// The relay broke on its own account. 5xx.
    case relayFailed(status: Int)
    /// The relay answered and refused the request itself. Any other status.
    case requestRefused(status: Int)
    /// An answer this app cannot read: not JSON, not an object, or missing the
    /// one field the call existed to fetch.
    case malformedResponse

    /// One sentence somebody can act on.
    ///
    /// No status codes and no error text — those go to ``diagnostic``. Exactly
    /// one of these says "sign in again", and it is the only one where signing
    /// in again does anything.
    public var message: String {
        switch self {
        case .notSignedIn:
            return "You’re not signed in on this device. Sign in under Settings ▸ Account."
        case .signInIncomplete:
            return "Sign-in didn’t complete."
        case .relayAddressInvalid:
            return "The relay address isn’t a valid URL. Fix it under Settings ▸ Advanced."
        case .unreachable:
            return "Couldn’t reach the relay. Check your internet connection."
        case .unauthorized:
            return "The relay wouldn’t accept your session. Try signing in again."
        case .rateLimited:
            return "The relay is turning away sign-ins right now. Wait a minute and try again."
        case .endpointMissing:
            return "This relay doesn’t offer what the app asked for. "
                + "Check the relay address under Settings ▸ Advanced."
        case .relayFailed:
            return "The relay had a problem of its own. Nothing on this device is wrong — "
                + "try again in a few minutes."
        case .requestRefused:
            return "The relay turned this request down. "
                + "Check the relay address under Settings ▸ Advanced."
        case .malformedResponse:
            return "The relay’s answer didn’t make sense. Try again in a few minutes."
        }
    }

    /// What a developer needs, in codes rather than prose.
    ///
    /// Every branch is a literal in this file or an integer read off the
    /// transport. Nothing here is copied out of a header, a request body or a
    /// response body, which is the rule that keeps a token out of the log.
    public var diagnostic: String {
        switch self {
        case .notSignedIn: return "no local credential"
        case .signInIncomplete: return "sign-in returned no callback"
        case .relayAddressInvalid: return "relay address is not a URL"
        case .unreachable(let reason): return "unreachable (\(reason))"
        case .unauthorized: return "HTTP 401 after refresh"
        case .rateLimited: return "HTTP 429"
        case .endpointMissing: return "HTTP 404"
        case .relayFailed(let status): return "HTTP \(status)"
        case .requestRefused(let status): return "HTTP \(status)"
        case .malformedResponse: return "unreadable response"
        }
    }

    /// So `localizedDescription` — which `signIn` reports — is the sentence
    /// rather than "The operation couldn’t be completed."
    public var errorDescription: String? { message }
}

/// One failed relay call, in the words a developer needs.
///
/// Deliberately small: a path that is a literal in `Account.swift`, an
/// enumerated cause, and a time. Nothing is built from a header, a request body
/// or a response body, so there is no route by which a bearer, a session token
/// or a pairing token reaches it — which matters, because this is the one thing
/// here that gets logged and copied to a clipboard. `AccountTests` asserts it.
public struct RelayDiagnostic: Sendable, Equatable {
    /// The endpoint, e.g. `/v1/daemons`.
    public let path: String
    public let failure: AccountError
    public let at: Date

    public init(path: String, failure: AccountError, at: Date) {
        self.path = path
        self.failure = failure
        self.at = at
    }

    /// The one line worth pasting into a bug report.
    public var line: String {
        "\(at.formatted(.iso8601)) POST \(path) — \(failure.diagnostic)"
    }
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
