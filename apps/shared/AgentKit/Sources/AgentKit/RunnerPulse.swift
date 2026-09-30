import Foundation
import Security

/// One runner's heartbeat, as the relay last heard it (ov-53).
///
/// A suspended app can't see its links, so the widget couldn't tell a runner
/// that stopped hours ago from a live one: both read "from notifications". A
/// paired runner now beats at the relay every few minutes, and the widget asks
/// the relay (`/v1/pulse`) how long ago each was heard. See
/// docs/superpowers/specs/2026-09-30-runner-heartbeat-design.md.
///
/// **An age, not a date.** The relay stamped the beat and measured from it on
/// its own clock, so this phone's clock never enters the answer.
public struct RunnerPulse: Codable, Sendable, Equatable {
    /// The runner's pairing label: its ssh target, or "This Mac".
    public var label: String
    /// What the runner calls itself: the computer's name on a Mac, the short
    /// hostname elsewhere. Nil from a runner that beat before it sent one.
    public var name: String?
    /// How long ago the relay last heard this runner, in milliseconds.
    public var heardAgo: Double
    /// How often the runner promised to beat, in seconds.
    public var beatEvery: Double

    public init(label: String, name: String? = nil, heardAgo: Double, beatEvery: Double) {
        self.label = label
        self.name = name
        self.heardAgo = heardAgo
        self.beatEvery = beatEvery
    }

    /// What a phone calls this runner: its own name before the pairing
    /// label, which is "This Mac" for every Mac's own runner and names
    /// nothing on a phone.
    public var displayName: String {
        if let name, !name.isEmpty { return name }
        return label
    }

    /// How long a runner may be silent before it's quiet: two missed beats and
    /// five minutes' slack for a slow request. Fifteen minutes at the shipped
    /// five-minute beat (`push::BEAT_EVERY`). The rule for every surface that
    /// asks `/v1/pulse`, which reports ages and decides nothing. The relay
    /// states it once more, `quietAfterMs` in `services/relay/src/index.ts`,
    /// for the Live Activity, which can't ask and redraws only when pushed
    /// (ov-71); the two are pinned at fifteen minutes by tests on each side.
    public static func quietAfter(beatEvery: TimeInterval) -> TimeInterval {
        2 * beatEvery + 5 * 60
    }

    /// Whether this runner has gone quiet: silent past its own promise.
    public var isQuiet: Bool {
        heardAgo / 1000 > Self.quietAfter(beatEvery: beatEvery)
    }

    /// The quiet runners' names, once each, in the order the relay listed them.
    public static func quiet(_ pulses: [RunnerPulse]) -> [String] {
        var seen = Set<String>()
        return pulses.filter(\.isQuiet).map(\.displayName).filter { seen.insert($0).inserted }
    }

    /// How often the widget asks to be looked at again while a runner beats,
    /// or after a fetch that failed.
    ///
    /// Thirty minutes: 48 asks a day, inside WidgetKit's reload budget of
    /// roughly 40 to 70, which the reloads from the app and the notification
    /// extension share. The system throttles past it anyway. So a runner that
    /// stops reads as quiet within about 45 minutes (its 15 plus a look),
    /// where before it never did.
    public static let lookEvery: TimeInterval = 30 * 60

    /// What asking the relay came to.
    public enum Reading: Sendable, Equatable {
        /// No pulse credential on this phone: never registered, or signed out.
        case noCredential
        /// The relay answered, naming every runner that beats.
        case answered([RunnerPulse])
        /// The relay doesn't know this token: signed out elsewhere, the device
        /// revoked, or moved to another account. Asking again won't help.
        case refused
        /// No answer: offline, a timeout, a relay too old for the route, a
        /// server error. Asking again later might.
        case failed
    }

    /// What the widget draws from a reading and a snapshot, and when to look
    /// again.
    public struct Plan: Sendable, Equatable {
        /// The runners to name as lost touch with, beside the app's own.
        public var quiet: [String]
        /// When to rebuild the timeline, or nil for never.
        public var nextLook: Date?
    }

    /// The widget's whole decision, kept here where it can be tested, so
    /// `getTimeline` is glue.
    ///
    /// - **A fresh, complete app snapshot wins.** If the app polled every
    ///   runner more recently than a runner could have gone quiet, the phone
    ///   heard from its runners over its own links and a relay that hasn't
    ///   (a LAN-only runner, a relay outage) is not the better witness.
    /// - **A failed fetch looks again**, a look later, rather than parking
    ///   the widget on `.never` until an alert happens to reload it — the
    ///   quiet night is the case this exists for. A refused token doesn't:
    ///   the next registration files a new one and reloads.
    /// - Nothing beating, no credential, or refused: today's hedge and
    ///   `.never`, exactly as before.
    ///
    /// `every` is how long the surface waits between looks: `lookEvery` for
    /// the phone's widget, longer for a watch complication, whose budget is
    /// tighter and shared with every reload the watch app asks for.
    public static func plan(
        snapshot: FleetSnapshot, reading: Reading, at now: Date,
        every: TimeInterval = lookEvery
    ) -> Plan {
        switch reading {
        case .noCredential, .refused:
            return Plan(quiet: [], nextLook: nil)
        case .failed:
            return Plan(quiet: [], nextLook: now.addingTimeInterval(every))
        case .answered(let pulses):
            let heardByApp = snapshot.complete && (snapshot.lostRunners ?? []).isEmpty
            let stale = pulses.filter { pulse in
                !(heardByApp
                    && snapshot.age(at: now) < quietAfter(beatEvery: pulse.beatEvery))
            }
            return Plan(
                quiet: quiet(stale),
                nextLook: pulses.isEmpty ? nil : now.addingTimeInterval(every))
        }
    }

    /// Ask with `credential`, if there is one, and plan: the whole of what a
    /// surface does with the pulse, so a watch and a widget can't read it
    /// two ways. No credential asks nothing and plans today's hedge.
    ///
    /// `held` is a credential the vault holds but can't read yet: the watch's
    /// file, protected until the watch is unlocked. That's a look that
    /// failed, and asks again, rather than a watch with no credential.
    public static func look(
        snapshot: FleetSnapshot, credential: PulseCredential?, held: Bool = false, at now: Date,
        every: TimeInterval = lookEvery, session: URLSession = .shared
    ) async -> Plan {
        plan(
            snapshot: snapshot,
            reading: await read(credential, held: held, session: session),
            at: now, every: every)
    }

    /// What asking with `credential` came to. See `look`.
    public static func read(
        _ credential: PulseCredential?, held: Bool = false, session: URLSession = .shared
    ) async -> Reading {
        if let credential { return await fetch(credential, session: session) }
        return held ? .failed : .noCredential
    }

    /// What `/v1/pulse` answered, or nil for anything else.
    public static func decode(_ data: Data) -> [RunnerPulse]? {
        struct Answer: Decodable { var runners: [RunnerPulse] }
        return try? JSONDecoder().decode(Answer.self, from: data).runners
    }

    /// Ask the relay who's beating.
    ///
    /// A short timeout, because this runs inside a widget's timeline request,
    /// which the system gives a few seconds and no more.
    public static func fetch(
        _ credential: PulseCredential, session: URLSession = .shared
    ) async -> Reading {
        guard let url = URL(string: credential.relay + "/v1/pulse") else { return .failed }
        var request = URLRequest(url: url, timeoutInterval: 8)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("Bearer \(credential.token)", forHTTPHeaderField: "Authorization")
        request.httpBody = Data("{}".utf8)
        guard let (data, response) = try? await session.data(for: request),
            let status = (response as? HTTPURLResponse)?.statusCode
        else { return .failed }
        if status == 401 { return .refused }
        guard status == 200, let pulses = decode(data) else { return .failed }
        return .answered(pulses)
    }
}

extension FleetSnapshot {
    /// This snapshot, with no working agent stated as now while any runner is
    /// quiet (ov-71 review M3).
    ///
    /// The relay names a quiet runner by the name it beats with, and a row
    /// here names its runner by the phone's label; the two don't reliably
    /// match, so the watch can't tell which agents are the quiet runner's.
    /// It says less rather than more: every agent that isn't latched is
    /// marked not answering, which is `confidence`'s "last seen", the
    /// glance's uncounted, and no staleness moment ahead. Blocked and done
    /// hold at any age. The same reading the card makes when its lines are
    /// gone (`AgentCardState.unvouched`). Nobody quiet is `self`.
    public func quietened(_ quiet: [String]) -> FleetSnapshot {
        guard !quiet.isEmpty else { return self }
        var copy = self
        copy.agents = agents.map { agent in
            guard !agent.isLatched else { return agent }
            var agent = agent
            agent.runnerAnswering = false
            return agent
        }
        return copy
    }
}

/// What a widget reads `/v1/pulse` with: the relay, the phone's pulse token,
/// and the account it was made for.
///
/// **The phone makes the token**, once per account, and sends it on every
/// registration; the relay keeps its hash. It used to be minted by the relay
/// per registration, and two registrations at launch answered out of order
/// left the widget holding a token whose hash was already gone.
public struct PulseCredential: Codable, Sendable, Equatable {
    public var relay: String
    public var token: String
    /// The account this token was made for. A different account signing in
    /// gets a new token, so the old account's runners aren't readable with
    /// what the widget holds.
    public var account: String

    public init(relay: String, token: String, account: String) {
        self.relay = relay
        self.token = token
        self.account = account
    }

    /// Where the phone files this in the watch's application context, beside
    /// the snapshot (ov-71).
    ///
    /// **The watch can't make its own.** A token is filed at `/v1/devices`,
    /// which takes a WorkOS session, and the session lives in the phone app's
    /// keychain group; a watch has no sign-in of its own, and it has no push
    /// registration either. So the phone hands the watch the token it already
    /// made, over WatchConnectivity, which is encrypted between the paired
    /// devices. It reads runner names and ages on this account and nothing
    /// else, and the relay can't tell the watch's asks from the widget's.
    public static let watchContextKey = "pulse"

    /// This, as the context carries it: JSON, which `WCSession` takes as `Data`.
    public var contextValue: Data? { try? JSONEncoder().encode(self) }

    /// The credential a context carries, or nil for none: a phone signed out,
    /// one older than this, or bytes this build can't read.
    public static func carried(in context: [String: Any]) -> PulseCredential? {
        guard let data = context[watchContextKey] as? Data else { return nil }
        return try? JSONDecoder().decode(PulseCredential.self, from: data)
    }
}

/// Where the pulse credential's bytes live. The Keychain on a phone; memory
/// in a test.
public protocol PulseVault: Sendable {
    func read() -> Data?
    @discardableResult func write(_ data: Data) -> Bool
    func delete()
    /// Whether there's a credential here at all, readable or not.
    var holds: Bool { get }
}

extension PulseVault {
    public var holds: Bool { read() != nil }
}

/// The pulse credential in the Keychain, in the App Group's access group, so
/// the app writes it and the widget extension reads it.
///
/// The App Group rather than the app's own keychain group, which the widget
/// can't read. An App Group identifier is a keychain access group on iOS
/// without a keychain-access-groups entitlement, and this item is the only
/// thing in it: the session stays in the app's own group (`TokenStore`).
/// After first unlock, this device only: a widget on a locked phone reads it.
public struct KeychainPulseVault: PulseVault {
    let group: String
    private static let service = "farcooler.pulse"

    public init(group: String) { self.group = group }

    private var query: [CFString: Any] {
        [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: Self.service,
            kSecAttrAccount: "pulse",
            kSecAttrAccessGroup: group,
        ]
    }

    public func read() -> Data? {
        var result: CFTypeRef?
        var ask = query
        ask[kSecReturnData] = true
        ask[kSecMatchLimit] = kSecMatchLimitOne
        guard SecItemCopyMatching(ask as CFDictionary, &result) == errSecSuccess else { return nil }
        return result as? Data
    }

    @discardableResult
    public func write(_ data: Data) -> Bool {
        delete()
        var add = query
        add[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        add[kSecValueData] = data
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    public func delete() {
        SecItemDelete(query as CFDictionary)
    }
}

/// The pulse credential as a file in an App Group container: the watch's
/// vault (ov-71).
///
/// A file rather than the Keychain on the watch, because the App Group's
/// keychain access group is what the phone relies on, and nothing here has
/// shown that holds on watchOS; a container file does, since the snapshot
/// already lives beside it. The container is the watch's own: an App Group
/// is per device, written by the watch app as contexts arrive and read by
/// the complication. The token reads runner names and ages and nothing else.
/// Completely protected: readable only while the watch is unlocked.
public struct ContainerPulseVault: PulseVault {
    let file: URL

    public init(container: URL) {
        file = container.appendingPathComponent("pulse.json")
    }

    public func read() -> Data? { try? Data(contentsOf: file) }

    /// Whether the file is there, which a locked watch can answer and can't
    /// read. See `RunnerPulse.look`'s `held`.
    public var holds: Bool { FileManager.default.fileExists(atPath: file.path) }

    @discardableResult
    public func write(_ data: Data) -> Bool {
        // Complete protection: unreadable while the watch is locked (ov-71
        // review M1). A complication drawing on a locked watch then finds the
        // file and can't open it, which `holds` turns into a look that failed.
        #if os(watchOS) || os(iOS)
            let options: Data.WritingOptions = [.atomic, .completeFileProtection]
        #else
            let options: Data.WritingOptions = [.atomic]
        #endif
        return (try? data.write(to: file, options: options)) != nil
    }

    public func delete() { try? FileManager.default.removeItem(at: file) }
}

/// The phone's pulse credential: made once, read by the widget, gone at
/// sign-out. On the watch, what the phone last sent.
public enum PulseStore {
    /// Posted whenever this device's credential changes: made, moved to
    /// another relay or account, or cleared at sign-out. The phone's watch
    /// link sends the watch a context at once, with the new credential or
    /// without one, rather than at its next poll (ov-71 review M1).
    public static let changed = Notification.Name("farcooler.pulse.changed")

    /// This build's vault, or nil in one with no App Group (the Mac, a test
    /// host), where nothing reads a pulse. A file in the container on the
    /// watch (`ContainerPulseVault`), the Keychain everywhere else.
    public static var vault: PulseVault? {
        guard let group = SnapshotStore.groupIdentifier else { return nil }
        #if os(watchOS)
            return SnapshotStore.container(forGroup: group).map { ContainerPulseVault(container: $0) }
        #else
            return KeychainPulseVault(group: group)
        #endif
    }

    /// Keep what the phone sent the watch: file a new credential, forget it
    /// when the phone sent none. Whether anything changed, which is when the
    /// complication is worth reloading.
    @discardableResult
    public static func adopt(_ credential: PulseCredential?, in vault: PulseVault) -> Bool {
        let held = read(from: vault)
        guard held != credential else { return false }
        guard let credential, let data = credential.contextValue else {
            vault.delete()
            return held != nil
        }
        return vault.write(data)
    }

    public static func read(from vault: PulseVault) -> PulseCredential? {
        vault.read().flatMap { try? JSONDecoder().decode(PulseCredential.self, from: $0) }
    }

    /// The token to send on this registration: the one already made for this
    /// account, or a new one. The relay is kept current either way, since
    /// it's a setting. Nil only when the vault won't take the write: better
    /// to register without a token than with one the widget can't find.
    public static func token(relay: String, account: String, in vault: PulseVault) -> String? {
        if var held = read(from: vault), held.account == account {
            if held.relay != relay {
                held.relay = relay
                guard let data = try? JSONEncoder().encode(held), vault.write(data) else { return nil }
                NotificationCenter.default.post(name: changed, object: nil)
            }
            return held.token
        }
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
            return nil
        }
        let token = bytes.map { String(format: "%02x", $0) }.joined()
        let credential = PulseCredential(relay: relay, token: token, account: account)
        guard let data = try? JSONEncoder().encode(credential), vault.write(data) else { return nil }
        NotificationCenter.default.post(name: changed, object: nil)
        return token
    }

    /// This build's credential, or nil.
    public static func read() -> PulseCredential? {
        vault.flatMap { read(from: $0) }
    }

    /// Forget the credential. At sign-out: the next sign-in makes a new one.
    public static func clear() {
        vault?.delete()
        NotificationCenter.default.post(name: changed, object: nil)
    }
}
