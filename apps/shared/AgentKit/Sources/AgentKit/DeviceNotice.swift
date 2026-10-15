import SwiftUI

/// A device added to or removed from the signed-in account, or trusted on
/// first use again because none of its devices was confirmed (R-75).
public struct DeviceEvent: Equatable, Sendable, Identifiable {
    public enum Kind: Equatable, Sendable { case added, removed, trusted }
    public let kind: Kind
    public let row: DeviceKeyRow
    public let at: Date

    public var id: String { "\(kind)-\(row.id)" }
}

/// Telling the person, in every signed-in app, about each device added to or
/// removed from their account since that app last told them (ov-550 review
/// M1).
///
/// The relay's push is only a hint: the stolen session it exists to expose
/// can delete every push install first. This reads `directory/me`, whose rows
/// no route deletes, and keeps, per account and per app, the time of the last
/// event the person dismissed. Nothing a session can do on the relay clears
/// it: only the person, here. An app that has never shown one tells about
/// every device but itself, which is also how devices registered before the
/// notices shipped get announced.
@MainActor
public final class DeviceNotice: ObservableObject {
    public static let shared = DeviceNotice()

    /// What the banner says, oldest first. Empty when there's nothing new.
    @Published public private(set) var events: [DeviceEvent] = []

    /// Who is signed in and the account's device keys, removed ones included;
    /// nil when signed out or the relay couldn't say. A test answers it.
    public var load: @MainActor () async -> (account: String, rows: [DeviceKeyRow])? = {
        let account = Account.shared
        guard account.isSignedIn else { return nil }
        guard case .success(let rows) = await account.directoryClient.deviceKeys(includingRemoved: true) else {
            return nil
        }
        return (account.userId, rows)
    }
    /// This device's own Key A for an account, so its own arrival isn't news.
    public var ownKeyA: (_ account: String) -> String? = { DeviceKeyRegistration.shared.publicKey($0) }
    public var defaults: UserDefaults = .standard

    private var account: String?

    public init() {}

    /// The events after `seen` (every one, with nothing seen yet), but this
    /// device's own arrival.
    public static func unseen(_ rows: [DeviceKeyRow], since seen: Date?, ownKeyA: String?) -> [DeviceEvent] {
        var events: [DeviceEvent] = []
        for row in rows {
            // A device trusted by the reset as it arrived is one event, the
            // trust, not an add as well (fix round 1, M1).
            let arrivedTrusted = row.wasTrustedAgain && row.trustedAt == row.createdAt
            if let at = row.createdAt, !arrivedTrusted, seen.map({ at > $0 }) ?? true, !row.isHeld(by: ownKeyA) {
                events.append(DeviceEvent(kind: .added, row: row, at: at))
            }
            // Trusted by R-75's reset: the moment a stolen session that
            // removed every confirmed device would take over, so it's news,
            // whether the device is new or one the person already knew.
            if row.wasTrustedAgain, let at = row.trustedAt, seen.map({ at > $0 }) ?? true, !row.isHeld(by: ownKeyA) {
                events.append(DeviceEvent(kind: .trusted, row: row, at: at))
            }
            if let at = row.revokedAt, seen.map({ at > $0 }) ?? true {
                events.append(DeviceEvent(kind: .removed, row: row, at: at))
            }
        }
        return events.sorted { $0.at < $1.at }
    }

    /// Ask again: at launch, and each time the app comes forward.
    public func refresh() async {
        guard let loaded = await load() else {
            events = []
            return
        }
        account = loaded.account
        events = Self.unseen(loaded.rows, since: seen(loaded.account), ownKeyA: ownKeyA(loaded.account))
    }

    /// The person has read it: nothing up to the newest event shown is news
    /// again in this app.
    public func dismiss() {
        guard let account, let newest = events.map(\.at).max() else { return }
        defaults.set(newest.timeIntervalSince1970, forKey: Self.key(account))
        events = []
    }

    private func seen(_ account: String) -> Date? {
        (defaults.object(forKey: Self.key(account)) as? Double).map(Date.init(timeIntervalSince1970:))
    }

    static func key(_ account: String) -> String { "deviceNotice.seen.\(account)" }

    /// The banner's title.
    public static func title(_ events: [DeviceEvent]) -> String {
        if events.count == 1 {
            switch events[0].kind {
            case .added: return "A device was added to your account"
            case .removed: return "A device was removed from your account"
            case .trusted: return "A device is now confirmed on your account"
            }
        }
        return "Your account’s devices changed"
    }

    /// The banner's sentence: each event with the device's quoted label, key
    /// words and day, then what to do.
    public static func detail(_ events: [DeviceEvent]) -> String {
        let lines = events.map { event -> String in
            let words = event.row.fingerprint.isEmpty ? "" : " (\(KeyFingerprint.short(event.row.fingerprint)))"  // not a count: key words
            let day = event.at.formatted(date: .abbreviated, time: .omitted)
            switch event.kind {
            case .added: return "Added “\(event.row.shownLabel)”\(words) on \(day)."
            case .removed: return "Removed “\(event.row.shownLabel)”\(words) on \(day)."
            case .trusted:
                return "Confirmed “\(event.row.shownLabel)”\(words) on \(day), since none of your devices was "
                    + "confirmed."
            }
        }
        return (lines + ["If you don’t recognize a change, review Device Keys in Settings."]).joined(separator: " ")
    }
}

/// The banner each signed-in app shows for `DeviceNotice`, until the person
/// dismisses it.
public struct DeviceNoticeBanner: View {
    @ObservedObject private var notice: DeviceNotice
    @Environment(\.scenePhase) private var scenePhase

    public init(notice: DeviceNotice = .shared) { self.notice = notice }

    public var body: some View {
        Group {
            if !notice.events.isEmpty {
                HStack(alignment: .top, spacing: Spacing.inset) {
                    Image(systemName: "key.fill")
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: Spacing.tight) {
                        Text(DeviceNotice.title(notice.events)).font(.callout.weight(.semibold))
                        Text(DeviceNotice.detail(notice.events)).font(.callout)
                    }
                    Spacer(minLength: Spacing.group)
                    Button("Dismiss") { notice.dismiss() }
                }
                .padding(Spacing.inset)
                .surface(.floating, in: RoundedRectangle(cornerRadius: Radius.large, style: .continuous))
                .padding(.horizontal, Spacing.section)
                .padding(.top, Spacing.group)
            }
        }
        .task { await notice.refresh() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { Task { await notice.refresh() } }
        }
    }
}
