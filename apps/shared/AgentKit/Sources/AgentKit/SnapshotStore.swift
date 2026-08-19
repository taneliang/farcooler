import Foundation

/// Where the snapshot lives, and the only code that reads or writes it.
///
/// A file in the App Group container rather than `UserDefaults`. An atomic
/// replace either lands or does not, so a widget reads the previous snapshot or
/// the next one and never half of one — and a half-written snapshot is a
/// surface showing a state that was never true, which is the one thing none of
/// these surfaces may do.
///
/// Every read that can fail returns nil rather than throwing. The callers are a
/// widget's timeline provider and a notification service extension, and neither
/// has anywhere useful to put an error: the honest response to an unreadable
/// snapshot is the same as to an absent one — say nothing is known.
public enum SnapshotStore {
    /// The Info.plist key each target carries, filled from the
    /// `FARCOOLER_APP_GROUP` build setting. Read rather than hardcoded because
    /// there are four channels and each has its own group; a literal here would
    /// be a second list to keep in step with `generate-project.py`.
    private static let infoKey = "FarCoolerAppGroup"

    private static let fileName = "fleet.json"

    /// This build's App Group, or nil in a target that declares none.
    public static var groupIdentifier: String? {
        guard
            let value = Bundle.main.object(forInfoDictionaryKey: infoKey) as? String,
            !value.isEmpty
        else { return nil }
        return value
    }

    /// The shared container, or nil when the entitlement is missing — which on
    /// a device means the profile did not grant the group, and is worth failing
    /// visibly in testing rather than falling back to a private directory that
    /// silently nobody else can see.
    public static func container(forGroup group: String) -> URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
    }

    public static func read(fromContainer container: URL) -> FleetSnapshot? {
        guard let data = try? Data(contentsOf: container.appendingPathComponent(fileName))
        else { return nil }
        return try? decoder.decode(FleetSnapshot.self, from: data)
    }

    public static func write(_ snapshot: FleetSnapshot, toContainer container: URL) throws {
        let data = try encoder.encode(snapshot)
        try data.write(to: container.appendingPathComponent(fileName), options: .atomic)
    }

    /// The same pair, resolving the container from this build's own group.
    /// Nil group or nil container is "nothing known", not a crash.
    public static func read() -> FleetSnapshot? {
        guard let group = groupIdentifier, let container = container(forGroup: group)
        else { return nil }
        return read(fromContainer: container)
    }

    public static func write(_ snapshot: FleetSnapshot) {
        guard let group = groupIdentifier, let container = container(forGroup: group)
        else { return }
        try? write(snapshot, toContainer: container)
    }

    /// Dates as seconds since 1970 on both sides.
    ///
    /// Pinned rather than left to the default so the app and the extension
    /// cannot be built with different strategies and write files neither can
    /// read — a failure that shows up as a permanently empty widget.
    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        return encoder
    }()

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return decoder
    }()
}
