import Foundation

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
    /// How long ago the relay last heard this runner, in milliseconds.
    public var heardAgo: Double
    /// How often the runner promised to beat, in seconds.
    public var beatEvery: Double

    public init(label: String, heardAgo: Double, beatEvery: Double) {
        self.label = label
        self.heardAgo = heardAgo
        self.beatEvery = beatEvery
    }

    /// How long a runner may be silent before it's quiet: two missed beats and
    /// five minutes' slack for a slow request. Fifteen minutes at the shipped
    /// five-minute beat (`push::BEAT_EVERY`). The one place this rule lives;
    /// the relay reports ages and decides nothing.
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
        return pulses.filter(\.isQuiet).map(\.label).filter { seen.insert($0).inserted }
    }

    /// How often the widget asks to be looked at again while a runner beats.
    ///
    /// Thirty minutes: 48 asks a day, inside WidgetKit's reload budget of
    /// roughly 40 to 70, which the reloads from the app and the notification
    /// extension share. The system throttles past it anyway. So a runner that
    /// stops reads as quiet within about 45 minutes (its 15 plus a look),
    /// where before it never did.
    public static let lookEvery: TimeInterval = 30 * 60

    /// When the widget's timeline should be rebuilt, or nil for never.
    ///
    /// Nil unless the relay named a runner that beats: with none (no token, a
    /// failed fetch, only runners too old to beat) there's nothing a later
    /// look could learn, and the widget keeps its `.never`.
    public static func nextLook(after now: Date, pulses: [RunnerPulse]?) -> Date? {
        guard let pulses, !pulses.isEmpty else { return nil }
        return now.addingTimeInterval(lookEvery)
    }

    /// What `/v1/pulse` answered, or nil for anything else.
    public static func decode(_ data: Data) -> [RunnerPulse]? {
        struct Answer: Decodable { var runners: [RunnerPulse] }
        return try? JSONDecoder().decode(Answer.self, from: data).runners
    }

    /// Ask the relay who's beating. Nil when it couldn't say: offline, a relay
    /// too old for the route, a token it no longer knows. Nil means "not
    /// told", and the widget keeps the hedge it had before any of this.
    ///
    /// A short timeout, because this runs inside a widget's timeline request,
    /// which the system gives a few seconds and no more.
    public static func fetch(
        _ credential: PulseCredential, session: URLSession = .shared
    ) async -> [RunnerPulse]? {
        guard let url = URL(string: credential.relay + "/v1/pulse") else { return nil }
        var request = URLRequest(url: url, timeoutInterval: 8)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("Bearer \(credential.token)", forHTTPHeaderField: "Authorization")
        request.httpBody = Data("{}".utf8)
        guard let (data, response) = try? await session.data(for: request),
            (response as? HTTPURLResponse)?.statusCode == 200
        else { return nil }
        return decode(data)
    }
}

/// What a widget reads `/v1/pulse` with: the relay, and the device's pulse
/// token. The token reads runner labels and ages on its own account and
/// nothing else, which is why it may sit in the App Group rather than the
/// keychain the session lives in — a widget extension can't read that
/// keychain, and sharing it would widen what every extension holds.
public struct PulseCredential: Codable, Sendable, Equatable {
    public var relay: String
    public var token: String

    public init(relay: String, token: String) {
        self.relay = relay
        self.token = token
    }
}

/// The pulse credential's file in the App Group, beside `fleet.json`.
///
/// Written by the app when the relay hands a token over, removed at sign-out,
/// and only read by the widget.
public enum PulseStore {
    private static let fileName = "pulse.json"

    public static func read(fromContainer container: URL) -> PulseCredential? {
        guard let data = try? Data(contentsOf: container.appendingPathComponent(fileName))
        else { return nil }
        return try? JSONDecoder().decode(PulseCredential.self, from: data)
    }

    public static func write(_ credential: PulseCredential, toContainer container: URL) throws {
        let data = try JSONEncoder().encode(credential)
        try data.write(to: container.appendingPathComponent(fileName), options: .atomic)
    }

    public static func clear(inContainer container: URL) {
        try? FileManager.default.removeItem(at: container.appendingPathComponent(fileName))
    }

    /// The same three, in this build's own App Group. No group is a no-op:
    /// the Mac has none, and nothing there reads a pulse.
    public static func read() -> PulseCredential? {
        guard let container = groupContainer else { return nil }
        return read(fromContainer: container)
    }

    public static func write(_ credential: PulseCredential) {
        guard let container = groupContainer else { return }
        try? write(credential, toContainer: container)
    }

    public static func clear() {
        guard let container = groupContainer else { return }
        clear(inContainer: container)
    }

    private static var groupContainer: URL? {
        guard let group = SnapshotStore.groupIdentifier else { return nil }
        return SnapshotStore.container(forGroup: group)
    }
}
