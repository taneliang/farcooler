import SwiftUI

/// One device key the directory holds for the signed-in account (ov-480 §3):
/// a device that signed in to Far Cooler and registered its Key A.
public struct DeviceKeyRow: Identifiable, Equatable, Sendable {
    public let id: String
    /// What the registering session called it. Any session on the account
    /// chooses this, an impostor's included, so it is never the only thing a
    /// screen shows (review M2): the key words and the date go beside it.
    public let label: String
    /// `ssh-ed25519 AAAA…`, as the device registered it.
    public let keyA: String
    /// `SHA256:…`, the relay's fingerprint of `keyA`, or "" when it sent none.
    public let fingerprint: String
    /// `trusted`, `certified`, `pending` or `revoked`.
    public let state: String
    public let createdAt: Date?
    public let revokedAt: Date?
    /// When it was trusted on first use: its arrival for an account's first
    /// device, later for one trusted again because none of the account's
    /// devices was confirmed any more (ruling R-75, ov-561). Nil when it
    /// never was, or the relay doesn't say.
    public let trustedAt: Date?
    /// Whether that trust was R-75's reset rather than the account's first
    /// device: a new device trusted with none of the others confirmed, or one
    /// already on the account trusted when it signed in again.
    public let trustReset: Bool
    /// When your teams' admins are sent its key, unless one confirms it with
    /// you in person first (ruling R-76, ov-565): for a device trusted by
    /// R-75's reset, or confirmed by one, until its 72-hour hold ends. Nil
    /// for any other device, or when the relay doesn't say. The relay's
    /// clock decides it; this only shows it.
    public let heldUntil: Date?
    /// Each team whose admin lifted its hold, and when (ov-565 fix round 1,
    /// M1): news for the banner, since lifting a hold is what a stolen
    /// session would want.
    public let holdConfirmations: [HoldConfirmation]
    /// The device whose certificate confirmed this one, or nil. A phone's
    /// "Make a New Key" names its new key as the old one's successor only when
    /// the old key confirmed it (ov-569).
    public let certifiedBy: String?

    public init(
        id: String, label: String, keyA: String, state: String, fingerprint: String = "",
        createdAt: Date? = nil, revokedAt: Date? = nil, trustedAt: Date? = nil, trustReset: Bool = false,
        heldUntil: Date? = nil, holdConfirmations: [HoldConfirmation] = [], certifiedBy: String? = nil
    ) {
        self.id = id
        self.label = label
        self.keyA = keyA
        self.state = state
        self.fingerprint = fingerprint
        self.createdAt = createdAt
        self.revokedAt = revokedAt
        self.trustedAt = trustedAt
        self.trustReset = trustReset
        self.heldUntil = heldUntil
        self.holdConfirmations = holdConfirmations
        self.certifiedBy = certifiedBy
    }

    /// Trusted by R-75's reset, as the relay says (`trustReset`), whether it
    /// arrived then or earlier: never "the first device". What the banner
    /// announces.
    public var wasTrustedAgain: Bool { trustReset }

    public var isConfirmed: Bool { state == "trusted" || state == "certified" }
    public var isRemoved: Bool { state == "revoked" }

    /// Whether this row is the device holding `ownKeyA`, compared by the key
    /// itself, never its comment. Worked out here, from the key this device
    /// holds, so the relay can't move the "This device" mark onto another row.
    public func isHeld(by ownKeyA: String?) -> Bool {
        guard let ownKeyA, let mine = Self.blob(ownKeyA) else { return false }
        return Self.blob(keyA) == mine
    }

    static func blob(_ line: String) -> String? {
        let fields = line.split(separator: " ")
        return fields.count >= 2 ? String(fields[1]) : nil
    }

    /// The label as a screen may quote it (review M2, ov-560 R1): without
    /// format or control characters (bidi controls that could reverse the
    /// line, zero-width characters, line breaks), letters that draw as
    /// nothing, opening and closing quotes, or double-quote lookalikes, any of
    /// which could close the quotes around it and go on with a sentence of its
    /// own; the apostrophe ’ only once between two letters; runs of space made
    /// one; at most 40 characters. The relay, the web and Android use the same
    /// rule, all against `test/fixtures/device-labels.json`.
    public var shownLabel: String { Self.shown(label) }

    /// `shownLabel`'s rule for any name a screen quotes: a label, or a team's
    /// name in a hold's confirmation.
    public static func shown(_ label: String) -> String {
        let kept = label.unicodeScalars.filter { !Self.unshown($0) }
        var scalars = String.UnicodeScalarView()
        var index = kept.startIndex
        while index < kept.endIndex {
            guard kept[index] == "\u{2019}" else {
                scalars.append(kept[index])
                index = kept.index(after: index)
                continue
            }
            var end = index
            while end < kept.endIndex, kept[end] == "\u{2019}" { end = kept.index(after: end) }
            let before = index > kept.startIndex ? kept[kept.index(before: index)] : nil
            let after = end < kept.endIndex ? kept[end] : nil
            if let before, let after, Self.isLetter(before), Self.isLetter(after) { scalars.append("\u{2019}") }
            index = end
        }
        let plain = String(scalars)
            .split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let cut = String(plain.prefix(40)).trimmingCharacters(in: .whitespaces)
        return cut.isEmpty ? "a device" : cut
    }

    /// Whether `shownLabel` drops this character.
    /// The apostrophe isn't: `shownLabel` keeps it only between letters.
    static func unshown(_ scalar: Unicode.Scalar) -> Bool {
        if scalar == "\u{2019}" { return false }
        switch scalar.properties.generalCategory {
        case .format, .control, .lineSeparator, .paragraphSeparator, .initialPunctuation, .finalPunctuation:
            return true
        default: break
        }
        return Self.lookalikes.contains(scalar)
    }

    static func isLetter(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.properties.generalCategory {
        case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter: return true
        default: return false
        }
    }

    /// The double quotes and their lookalikes that are neither an opening nor
    /// a closing quote (" ` „ ″ ‶ ＂ 〝 〞 〟 ❝ ❞ ˮ ״ 〃), and the letters that
    /// draw as nothing (U+3164, U+115F, U+2800).
    private static let lookalikes: Set<Unicode.Scalar> = [
        "\"", "`", "\u{201E}", "\u{2033}", "\u{2036}", "\u{FF02}", "\u{301D}", "\u{301E}", "\u{301F}",
        "\u{275D}", "\u{275E}", "\u{02EE}", "\u{05F4}", "\u{3003}", "\u{3164}", "\u{115F}", "\u{2800}",
    ]

    /// What tells two rows with the same label apart: the key words, the same
    /// ones the ceremony shows, and the day it was added.
    public var identity: String {
        let words = fingerprint.isEmpty ? nil : KeyFingerprint.short(fingerprint)
        let added = createdAt.map { "Added \($0.formatted(date: .abbreviated, time: .omitted))" }
        return [words, added].compactMap { $0 }.joined(separator: " · ")
    }

    /// What the row says under the device's name. `anyConfirmed` is whether
    /// any of the account's devices is confirmed: with none, there is nothing
    /// to scan its code from (review L4).
    public func detail(isThisDevice: Bool, anyConfirmed: Bool = true, now: Date = Date()) -> String {
        var state: String
        switch self.state {
        case "trusted":
            state = wasTrustedAgain
                ? "Confirmed when it signed in, since none of your devices was confirmed"
                : "Confirmed as the first device on your account"
        case "certified": state = "Confirmed by another of your devices"
        default:
            state = anyConfirmed
                ? "Not confirmed yet. Scan its code from one of your confirmed devices."
                : "Not confirmed. None of your devices is confirmed, so whichever opens Far Cooler or "
                    + "signs in next becomes confirmed. Do that on the device you want to keep, then confirm "
                    + "the others from it."
        }
        // Waiting out its hold (R-76): what that means for the person's teams.
        if isConfirmed, let until = waitingOutHold(at: now) { state += ". " + Self.holdSentence(until: until) }
        return isThisDevice ? "This device. \(state)" : state
    }

    /// Whether this device should register again now, after a removal left
    /// `rows` with no confirmed device and this one pending (fix round 1,
    /// Low 4): the person removing is on this device, so it's the one they're
    /// keeping, and R-75 makes it the confirmed one at once rather than
    /// whichever of their devices opens Far Cooler first.
    public static func startsOverHere(_ rows: [DeviceKeyRow], ownKeyA: String?) -> Bool {
        guard !rows.contains(where: \.isConfirmed) else { return false }
        return rows.contains { $0.isHeld(by: ownKeyA) && $0.state == "pending" }
    }

    /// What the Remove dialog says, warning when `row` is the last confirmed
    /// device among `rows` (review M2): removing it leaves nothing that can
    /// confirm the others.
    public static func removalWarning(for row: DeviceKeyRow, among rows: [DeviceKeyRow]) -> String {
        let base = "Its key can’t be added back, and devices it confirmed will need confirming again."
        guard row.isConfirmed, rows.filter(\.isConfirmed).count == 1 else { return base }
        return "This is the only device that can confirm your others. Removing it leaves them unconfirmed, "
            + "and whichever opens Far Cooler or signs in next becomes confirmed in its place. Its key can’t be added back."
    }
}

extension DirectoryClient {
    /// The signed-in account's device keys, oldest first; the removed ones
    /// only when asked for, since a removal is news too (`DeviceNotice`).
    public func deviceKeys(includingRemoved: Bool = false) async -> Result<[DeviceKeyRow], DirectoryFailure> {
        switch await post("/v1/directory/me", [:]) {
        case .failure(let why):
            return .failure(.account(why))
        case .success(let reply):
            if reply.status == 404 { return .failure(.account(.endpointMissing)) }
            guard reply.status == 200 else { return .failure(Self.failure(reply)) }
            guard let rows = reply.body["devices"] as? [[String: Any]] else {
                return .failure(.account(.malformedResponse))
            }
            func date(_ value: Any?) -> Date? {
                (value as? NSNumber).map { Date(timeIntervalSince1970: $0.doubleValue / 1000) }
            }
            return .success(
                rows.compactMap { row in
                    guard let id = row["id"] as? String, let label = row["label"] as? String,
                        let keyA = row["keyA"] as? String, let state = row["state"] as? String,
                        includingRemoved || state != "revoked"
                    else { return nil }
                    return DeviceKeyRow(
                        id: id, label: label, keyA: keyA, state: state,
                        fingerprint: row["keyAFingerprint"] as? String ?? "",
                        createdAt: date(row["createdAt"]), revokedAt: date(row["revokedAt"]),
                        trustedAt: date(row["trustedAt"]), trustReset: row["trustReset"] as? Bool ?? false,
                        heldUntil: date(row["heldUntil"]),
                        holdConfirmations: (row["holdConfirmations"] as? [[String: Any]] ?? []).compactMap { entry in
                            guard let team = entry["team"] as? String, let at = date(entry["at"]) else { return nil }
                            return HoldConfirmation(team: team, at: at)
                        },
                        certifiedBy: row["certifiedBy"] as? String)
                })
        }
    }

    /// Remove one of the signed-in account's device keys, for good: a removed
    /// key can never be registered again, and every device it confirmed goes
    /// back to unconfirmed (R-52). With a `successor`, a phone's new key that
    /// the removed one confirmed (ov-569), those devices move to the successor
    /// instead.
    public func removeDeviceKey(id: String, successor: String? = nil) async -> Result<Void, DirectoryFailure> {
        var body: [String: Any] = ["device": id]
        if let successor { body["successor"] = successor }
        switch await post("/v1/directory/devices/revoke", body) {
        case .failure(let why):
            return .failure(.account(why))
        case .success(let reply):
            guard reply.status == 200 else { return .failure(Self.failure(reply)) }
            return .success(())
        }
    }
}

/// The account's device keys, each with a Remove button but this device's
/// own (ov-549 review M1).
///
/// The first device an account registers is trusted on first use, and a
/// stolen session that registers first would be that device. Each app shows a
/// banner for every device added or removed (`DeviceNotice`); this is where
/// the person acts on it, telling rows apart by their key words and date.
struct AccountDeviceKeysSection: View {
    @ObservedObject private var account = Account.shared
    @State private var rows: [DeviceKeyRow] = []
    @State private var failure: DirectoryFailure?
    @State private var removing: DeviceKeyRow?

    var body: some View {
        Section {
            if let failure {
                Text("Couldn’t load your device keys. \(failure.message)")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else if rows.isEmpty {
                Text("No device keys yet. Each device adds its key when it signs in.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else {
                let anyConfirmed = rows.contains(where: \.isConfirmed)
                ForEach(rows) { row in
                    let mine = row.isHeld(by: DeviceKeyRegistration.shared.publicKey(account.userId))
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(row.label)
                            Text(row.identity)
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.secondary)
                            Text(row.detail(isThisDevice: mine, anyConfirmed: anyConfirmed))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        if !mine {
                            Button("Remove", role: .destructive) { removing = row }
                                .buttonStyle(.plain)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        } header: {
            Text("Device Keys")
        } footer: {
            Text(
                "Each device that signs in to Far Cooler adds its key here, so your team’s admins can add it "
                    + "to runners as yours. To tell two devices apart, compare the words under each with the "
                    + "ones that device shows for itself here. Remove any device you don’t recognize.")
        }
        .confirmationDialog(
            removing.map { "Remove “\($0.shownLabel)”?" } ?? "",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            titleVisibility: .visible
        ) {
            Button("Remove Device", role: .destructive) {
                if let row = removing { Task { await remove(row) } }
            }
            Button("Cancel", role: .cancel) { removing = nil }
        } message: {
            if let row = removing { Text(DeviceKeyRow.removalWarning(for: row, among: rows)) }
        }
        .task { await load() }
    }

    private func load() async {
        guard account.isSignedIn else { return }
        switch await account.directoryClient.deviceKeys() {
        case .success(let loaded):
            rows = loaded
            failure = nil
        case .failure(let why):
            failure = why
        }
    }

    private func remove(_ row: DeviceKeyRow) async {
        removing = nil
        rows.removeAll { $0.id == row.id }
        let outcome = await account.directoryClient.removeDeviceKey(id: row.id)
        // The reload puts a refused row back; this says why.
        await load()
        if case .failure(let why) = outcome { failure = why }
        // That was the last confirmed device: this one, still pending, is the
        // one being kept (R-75).
        if case .success = outcome,
            DeviceKeyRow.startsOverHere(rows, ownKeyA: DeviceKeyRegistration.shared.publicKey(account.userId))
        {
            await DeviceKeyRegistration.shared.registerAgain(account: account.userId, directory: account.directoryClient)
            await load()
        }
    }
}
