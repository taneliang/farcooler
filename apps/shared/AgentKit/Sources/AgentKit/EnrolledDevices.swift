import Foundation

/// The devices that may log in to one runner, who added each, and what
/// revoking one takes with it (ov-855).
///
/// Decoded from `client.list`'s answer as both roads carry it: the Mac reads
/// `farcooler client list --json`, the phones read the bridge, and both print
/// `farcooler_client::enrolled_json`'s shape.
///
/// **One row per device, not per line.** A Mac holds two lines under one client
/// id (Far Cooler access, and shell access), and they are removed together, so
/// a list drawing both would show the Mac twice. A foreign line has no client
/// id and Far Cooler never touches it, so it has no row here either.
///
/// **Who added a device is the runner's answer** (`added_by`, ov-838), by
/// client id. A device whose adder is no longer enrolled says so rather than
/// naming nobody: "Added by a device that was removed".
public struct EnrolledDevices: Equatable, Sendable {
    public struct Device: Equatable, Identifiable, Sendable {
        public let clientId: String
        public let label: String
        public let scope: String
        /// The client id of the device that added this one, or "" for one the
        /// runner's own socket added and for lines from before runners kept it.
        public let addedBy: String
        public var id: String { clientId }

        public init(clientId: String, label: String, scope: String = "", addedBy: String = "") {
            self.clientId = clientId
            self.label = label
            self.scope = scope
            self.addedBy = addedBy
        }

        /// What a row calls it: the name the device gave, or its id when it gave none.
        public var title: String { label.isEmpty ? clientId : label }
    }

    public let devices: [Device]

    public init(devices: [Device]) {
        self.devices = devices
    }

    /// `{"clients": [...]}`, or nil for anything else.
    public init?(json: Data) {
        guard let body = try? JSONSerialization.jsonObject(with: json) as? [String: Any],
            let lines = body["clients"] as? [[String: Any]]
        else { return nil }
        var seen = Set<String>()
        var devices: [Device] = []
        for line in lines {
            guard line["foreign"] as? Bool != true,
                let id = line["clientId"] as? String, !id.isEmpty, seen.insert(id).inserted
            else { continue }
            devices.append(
                Device(
                    clientId: id, label: line["label"] as? String ?? "",
                    scope: line["scope"] as? String ?? "", addedBy: line["addedBy"] as? String ?? ""))
        }
        self.devices = devices
    }

    /// "Added by Alexandra’s iPhone", or nil for a device nobody's recorded as
    /// having added.
    public func addedByLine(for device: Device) -> String? {
        guard !device.addedBy.isEmpty else { return nil }
        guard let adder = devices.first(where: { $0.clientId == device.addedBy }) else {
            return "Added by a device that was removed"
        }
        return "Added by \(adder.title)"
    }

    /// Every device the chain of `added_by` from `device` reaches: what it
    /// added, and what those added in turn. The same set `ClientRevoke.with_added`
    /// takes. Never contains `device` itself, and ends on a loop.
    public func added(by device: Device) -> [Device] {
        var reached = Set([device.clientId])
        var frontier = [device.clientId]
        var found: [Device] = []
        while let parent = frontier.popLast() {
            for child in devices where child.addedBy == parent && reached.insert(child.clientId).inserted {
                found.append(child)
                frontier.append(child.clientId)
            }
        }
        return found
    }

    /// What Revoke offers for `device`.
    public func revokeChoices(for device: Device) -> RevokeChoices {
        RevokeChoices(device: device, added: added(by: device))
    }

    /// The two ways to revoke, or the one when the device added nothing.
    public struct RevokeChoices: Equatable, Sendable {
        public let device: Device
        public let added: [Device]

        public static let justThisTitle = "Revoke This"

        /// "Revoke This and the 2 It Added", or nil when it added none: a
        /// second choice that does the same thing is not a choice.
        public var withAddedTitle: String? {
            added.isEmpty ? nil : "Revoke This and the \(added.count) It Added"
        }

        public var title: String { "Revoke \(device.title)?" }

        /// What the confirmation says is at stake. Names what the second choice
        /// takes, and what neither does.
        public var message: String {
            var lines: [String] = []
            if !added.isEmpty {
                let names = added.map(\.title).joined(separator: ", ")
                lines.append(
                    "\(device.title) added \(added.count == 1 ? "1 device" : "\(added.count) devices"): \(names).")
            }
            lines.append("A shell the device already has open stays open.")
            return lines.joined(separator: " ")
        }
    }

    /// `client.revoke`'s arguments, as the phones' bridge takes them. `withAdded`
    /// is present only for the choice that takes what the device added too: the
    /// other sends exactly what every build before this sent.
    public static func revokeArguments(for device: Device, withAdded: Bool) -> [String: Any] {
        withAdded ? ["clientId": device.clientId, "withAdded": true] : ["clientId": device.clientId]
    }

    /// What a list read came to: the list, or why there isn't one (ov-855).
    public struct ListRead: Equatable {
        public let list: EnrolledDevices?
        public let failure: String?
    }

    /// Runs `client.list` and keeps a failure instead of dropping it. `sentence`
    /// turns the thrown error into this app's words.
    public static func read(
        _ call: () async throws -> Data, sentence: (Error) -> String
    ) async -> ListRead {
        do {
            guard let list = EnrolledDevices(json: try await call()) else {
                return ListRead(list: nil, failure: listNotRead)
            }
            return ListRead(list: list, failure: nil)
        } catch {
            return ListRead(list: nil, failure: sentence(error))
        }
    }

    /// What a revoke came to: the list as it now stands, or why it didn't take.
    public struct RevokeResult: Equatable {
        public let list: EnrolledDevices?
        public let failure: String?
    }

    /// Runs `client.revoke` and says why it failed rather than that it did.
    public static func revoke(
        _ call: () async throws -> Data, sentence: (Error) -> String
    ) async -> RevokeResult {
        do {
            guard let list = EnrolledDevices(json: try await call()) else {
                return RevokeResult(list: nil, failure: notRevoked)
            }
            return RevokeResult(list: list, failure: nil)
        } catch {
            return RevokeResult(list: nil, failure: sentence(error))
        }
    }

    /// A list read that failed. No cause invented, no raw error.
    public static let listNotRead = "Couldn’t read the list of devices on this runner."
    /// The runner has no one else to list.
    public static let emptySentence = "No other devices have access to this runner."
    /// A revoke the runner didn't take. No cause invented, no raw error.
    public static let notRevoked = "Couldn’t revoke that device. Check the runner is reachable, then try again."
    public static let sectionTitle = "Devices on This Runner"
}
