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

    public init(
        id: String, label: String, keyA: String, state: String, fingerprint: String = "",
        createdAt: Date? = nil, revokedAt: Date? = nil
    ) {
        self.id = id
        self.label = label
        self.keyA = keyA
        self.state = state
        self.fingerprint = fingerprint
        self.createdAt = createdAt
        self.revokedAt = revokedAt
    }

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

    /// The label as a screen may quote it: no double quote marks, so it can't
    /// close the quotes around it and go on with a sentence of its own, and at
    /// most 40 characters (review M2). The relay's notices use the same rule.
    public var shownLabel: String {
        let quotes: Set<Character> = ["\"", "`", "\u{201C}", "\u{201D}", "\u{201E}", "\u{201F}", "«", "»", "‹", "›"]
        let plain = label.filter { !quotes.contains($0) }
            .split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let cut = String(plain.prefix(40)).trimmingCharacters(in: .whitespaces)
        return cut.isEmpty ? "a device" : cut
    }

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
    public func detail(isThisDevice: Bool, anyConfirmed: Bool = true) -> String {
        let state: String
        switch self.state {
        case "trusted": state = "Confirmed as the first device on your account"
        case "certified": state = "Confirmed by another of your devices"
        default:
            state = anyConfirmed
                ? "Not confirmed yet. Scan its code from one of your confirmed devices."
                : "Not confirmed. None of your devices is confirmed, so none can confirm it yet. "
                    + "Ask the Far Cooler team to confirm it."
        }
        return isThisDevice ? "This device. \(state)" : state
    }

    /// What the Remove dialog says, warning when `row` is the last confirmed
    /// device among `rows` (review M2): removing it leaves nothing that can
    /// confirm the others.
    public static func removalWarning(for row: DeviceKeyRow, among rows: [DeviceKeyRow]) -> String {
        let base = "Its key can’t be added back, and devices it confirmed will need confirming again."
        guard row.isConfirmed, rows.filter(\.isConfirmed).count == 1 else { return base }
        return "This is the only device that can confirm your others. Removing it leaves them unconfirmed, "
            + "and none of your devices can confirm them again. Its key can’t be added back."
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
                        createdAt: date(row["createdAt"]), revokedAt: date(row["revokedAt"]))
                })
        }
    }

    /// Remove one of the signed-in account's device keys, for good: a removed
    /// key can never be registered again, and every device it confirmed goes
    /// back to unconfirmed (R-52).
    public func removeDeviceKey(id: String) async -> Result<Void, DirectoryFailure> {
        switch await post("/v1/directory/devices/revoke", ["device": id]) {
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
    }
}
