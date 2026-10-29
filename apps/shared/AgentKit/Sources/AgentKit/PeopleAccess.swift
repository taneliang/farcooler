import Foundation

/// What a person may do on a runner (ov-856): Owner, Admin, Member or Viewer.
///
/// The UI calls this **Access**, which avoids a clash with a member's role
/// (member or maintainer, ov-757), a separate say over charters. The words
/// are here, once, so the Mac, the iPhone and Android say the same thing.
public enum PersonAccess: String, CaseIterable, Identifiable, Sendable {
    case owner, admin, member, viewer

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .owner: "Owner"
        case .admin: "Admin"
        case .member: "Member"
        case .viewer: "Viewer"
        }
    }

    /// One line on what it allows, under the name in a menu.
    public var summary: String {
        switch self {
        case .owner: "Everything, including who has access."
        case .admin: "Can add and remove people and their devices."
        case .member: "Can work with agents and terminals."
        case .viewer: "Can see the board and progress, but not terminals."
        }
    }

    /// What a menu offers `chooser`: nobody is made an Owner (there is one,
    /// the account that owns the runner), and only the Owner makes an Admin.
    public static func choices(for chooser: PersonAccess) -> [PersonAccess] {
        switch chooser {
        case .owner: [.admin, .member, .viewer]
        case .admin: [.member, .viewer]
        case .member, .viewer: []
        }
    }
}

/// Where a device's step-up key stands, as the runner tells it.
public enum StepUpKeyStanding: String, Sendable {
    /// No key: the device keeps what it had until it registers one.
    case none = ""
    /// Waiting for another device of the same person to approve it.
    case proposed
    /// Signs admin actions.
    case trusted
    /// A proposal that waited too long; the device has to ask again.
    case expired
}

/// The people on a runner and their devices, as the People screen draws them
/// (ov-856, §8 of the design): each person with their Access, under each their
/// devices, then **Check These Devices** and **Waiting for Approval** when
/// either has any.
///
/// Built from `member.list` (who, their access, each device's place and key)
/// and `client.list` (each line: its name, who added it, whether it is a
/// shell). Neither is trusted for the other's facts.
public struct PeopleAccess: Equatable, Sendable {
    public struct Device: Equatable, Identifiable, Sendable {
        public let id: String
        /// What a person would call it.
        public let name: String
        /// "Added by E-Liang’s iPhone", or nil for a device the runner added
        /// itself (its socket) or one from before runners kept it.
        public let addedBy: String?
        /// Held at Viewer by its own cap, though its person may do more.
        public let viewerOnThisDevice: Bool
        /// A plain SSH line sits beside this device's Far Cooler line: a
        /// shell, which does everything its account can.
        public let hasShell: Bool
        /// A legacy line the owner has yet to confirm.
        public let needsCheck: Bool
        public let stepUp: StepUpKeyStanding
        /// The six digits the device shows, while its key waits.
        public let code: String
        /// How many devices this one added, and those they added in turn.
        public let added: Int
        /// The person's handle, "" for the owner.
        public let person: String
        public let personName: String
        /// How many devices are in this device's removal: itself and what it added.
        public var removal: Int { 1 + added }
    }

    public struct Person: Equatable, Identifiable, Sendable {
        /// "" for the runner's owner.
        public let handle: String
        public let name: String
        public let access: PersonAccess
        public let isYou: Bool
        public let devices: [Device]
        public var id: String { handle.isEmpty ? "(owner)" : handle }
    }

    public let people: [Person]
    /// Devices to look at, whoever's.
    public let toCheck: [Device]
    /// Keys waiting for an approval.
    public let waiting: [Device]
    /// This device's own row, when the runner lists it.
    public let thisDevice: Device?
    /// Whether this is the owner's Mac, which talks to its runner through the
    /// socket and so has no device of its own. Said by the caller, never
    /// guessed from `thisDevice` being nil: a phone whose id the runner doesn't
    /// list has no `thisDevice` either, and is not the socket.
    public let isSocket: Bool
    /// Whether the runner has recorded the people's access at all. False
    /// from a runner before it, which has no Access to show.
    public let recorded: Bool

    /// The caller's own access: Owner when they are the owner's device.
    public let yourAccess: PersonAccess

    /// `member.list` and `client.list` as `farcooler_client` or the CLI
    /// prints them, with `thisDevice` the client id of the device asking
    /// (empty on the owner's Mac, which is the socket). Nil when `members`
    /// isn't a member list.
    public init?(members: Data, clients: Data? = nil, thisDevice me: String = "", ownerName: String = "You", isSocket: Bool = false) {
        self.isSocket = isSocket
        guard let wire = try? JSONDecoder().decode(MembersWire.self, from: members) else { return nil }
        let lines = clients.flatMap { try? JSONDecoder().decode(ClientsWire.self, from: $0) }?.clients ?? []
        let access: [DeviceAccessWire] = wire.device_access ?? []
        let rows = Dictionary(access.map { ($0.client_id, $0) }, uniquingKeysWith: { first, _ in first })
        self.recorded = !access.isEmpty

        // Far Cooler's own lines by device: a Mac has two, told apart by
        // `shellAccess`. The plain one is a shell.
        var byDevice: [String: [ClientsWire.Client]] = [:]
        for line in lines where !line.foreign && !line.clientId.isEmpty { byDevice[line.clientId, default: []].append(line) }
        func name(of id: String) -> String {
            byDevice[id]?.first.map { Self.tidy($0.label) } ?? id
        }
        let added = Self.addedCounts(lines)

        func device(_ id: String, person: String, personName: String, row: DeviceAccessWire?) -> Device {
            let mine = byDevice[id] ?? []
            let by = mine.compactMap { $0.addedBy.isEmpty ? nil : $0.addedBy }.first
            return Device(
                id: id,
                name: name(of: id),
                addedBy: by.map { "Added by \(byDevice[$0] == nil ? "another device" : name(of: $0))" },
                viewerOnThisDevice: row?.cap == "viewer",
                hasShell: mine.contains { $0.shellAccess },
                needsCheck: row?.check ?? false,
                stepUp: StepUpKeyStanding(rawValue: row?.step_up ?? "") ?? .none,
                code: row?.step_up_code ?? "",
                added: added[id] ?? 0,
                person: person, personName: personName)
        }

        var people: [Person] = []
        var everyDevice: [Device] = []
        let youAreOwner = wire.you.isEmpty
        // The owner: every device the runner has bound to them.
        let ownerDevices = access.filter { $0.person.isEmpty }.map {
            device($0.client_id, person: "", personName: ownerName, row: $0)
        }
        if wire.device_access != nil || youAreOwner {
            people.append(Person(handle: "", name: youAreOwner ? ownerName : "Owner", access: .owner, isYou: youAreOwner, devices: ownerDevices))
            everyDevice += ownerDevices
        }
        for member in wire.members {
            let devices = member.devices.map { id in
                device(id, person: member.handle, personName: member.name, row: rows[id])
            }
            people.append(
                Person(
                    handle: member.handle, name: member.name, access: PersonAccess(rawValue: member.access ?? "") ?? .member,
                    isYou: wire.you == member.handle, devices: devices))
            everyDevice += devices
        }
        self.people = people
        self.toCheck = everyDevice.filter(\.needsCheck)
        self.waiting = everyDevice.filter { $0.stepUp == .proposed }
        self.thisDevice = me.isEmpty ? nil : everyDevice.first { $0.id == me }
        self.yourAccess =
            youAreOwner ? .owner : (people.first { $0.handle == wire.you }?.access ?? .member)
    }

    /// Whether `device` may be asked to approve `other`'s key: another
    /// device of the same person, with a trusted key of its own.
    public func mayApprove(_ other: Device, from: Device?) -> Bool {
        guard let from, from.id != other.id else { return false }
        return from.stepUp == .trusted && from.person == other.person && other.stepUp == .proposed
    }

    /// The waiting keys this screen lists. The Mac, which is the owner's socket,
    /// lists them all; a phone lists only its own person's, since it can
    /// approve no one else's.
    public var shownWaiting: [Device] {
        isSocket ? waiting : waiting.filter { $0.person == thisDevice?.person && thisDevice != nil }
    }

    /// Whether to offer Allow on `other`'s request: on the Mac always (the
    /// runner takes the owner's socket), on a phone only from another trusted
    /// device of the viewer's own person.
    public func mayAllow(_ other: Device) -> Bool {
        isSocket ? other.stepUp == .proposed : mayApprove(other, from: thisDevice)
    }

    /// The code as it is read aloud and compared: "418 207".
    public static func spoken(_ code: String) -> String {
        guard code.count == 6 else { return code }
        return "\(code.prefix(3)) \(code.suffix(3))"
    }

    /// The approval sheet's sentence: only allow it holding the device.
    public static func approvalPrompt(for device: Device) -> String {
        "Only allow this if you’re holding \(device.name) right now. Check that it shows \(spoken(device.code))."
    }

    /// The warning on a device with a shell on `runner`.
    public static func shellWarning(runner: String) -> String {
        "This device has a shell on \(runner), so it can do everything."
    }

    public static let viewerBadge = "Viewer on This Device"
    public static let shellBadge = "Has a Shell"
    public static let checkHeader = "Check These Devices"
    public static let checkFooter =
        "A teammate’s device added these. Remove any you don’t recognize."
    public static let waitingHeader = "Waiting for Approval"
    public static func waitingFooter(isSocket: Bool) -> String {
        isSocket
            ? "Only allow a device you’re holding. Check that it shows the same code."
            : "These devices asked to approve changes to who has access. Allow one only from another device of your own, while you’re holding the one that asked."
    }
    public static let limitToViewer = "Limit to Viewer"
    public static let removeDevice = "Remove Device…"

    /// A key as Far Cooler names it in `authorized_keys`: `farcooler-iPhone-t7xq9vd8`
    /// is an iPhone. The prefix and the fingerprint's tail are the file's, not a name.
    static func tidy(_ label: String) -> String {
        var name = label
        for prefix in ["farcooler-shell-", "farcooler-"] where name.hasPrefix(prefix) {
            name.removeFirst(prefix.count)
            break
        }
        if let dash = name.lastIndex(of: "-"), name.distance(from: dash, to: name.endIndex) == 9,
            name[name.index(after: dash)...].allSatisfy({ $0.isLetter || $0.isNumber })
        {
            name = String(name[..<dash])
        }
        return name.isEmpty ? label : name
    }

    /// For each device, the devices it added and the ones they added in turn.
    static func addedCounts(_ lines: [ClientsWire.Client]) -> [String: Int] {
        var parent: [String: String] = [:]
        for line in lines where !line.addedBy.isEmpty && !line.clientId.isEmpty { parent[line.clientId] = line.addedBy }
        var counts: [String: Int] = [:]
        for child in parent.keys {
            var seen: Set<String> = [child]
            var up = parent[child]
            while let id = up, !seen.contains(id) {
                counts[id, default: 0] += 1
                seen.insert(id)
                up = parent[id]
            }
        }
        return counts
    }

    // MARK: Wire

    struct DeviceAccessWire: Decodable {
        let client_id: String
        let person: String
        let cap: String
        let check: Bool
        let step_up: String
        let step_up_code: String

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: Keys.self)
            client_id = try c.decode(String.self, forKey: .client_id)
            person = try c.decodeIfPresent(String.self, forKey: .person) ?? ""
            cap = try c.decodeIfPresent(String.self, forKey: .cap) ?? ""
            check = try c.decodeIfPresent(Bool.self, forKey: .check) ?? false
            step_up = try c.decodeIfPresent(String.self, forKey: .step_up) ?? ""
            step_up_code = try c.decodeIfPresent(String.self, forKey: .step_up_code) ?? ""
        }
        enum Keys: String, CodingKey { case client_id, person, cap, check, step_up, step_up_code }
    }

    struct MembersWire: Decodable {
        struct Member: Decodable {
            let handle: String
            let name: String
            let access: String?
            let devices: [String]

            init(from decoder: Decoder) throws {
                let c = try decoder.container(keyedBy: Keys.self)
                handle = try c.decode(String.self, forKey: .handle)
                name = try c.decode(String.self, forKey: .name)
                access = try c.decodeIfPresent(String.self, forKey: .access)
                devices = try c.decodeIfPresent([String].self, forKey: .devices) ?? []
            }
            enum Keys: String, CodingKey { case handle, name, access, devices }
        }
        let members: [Member]
        let you: String
        let device_access: [DeviceAccessWire]?

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: Keys.self)
            members = try c.decode([Member].self, forKey: .members)
            you = try c.decodeIfPresent(String.self, forKey: .you) ?? ""
            device_access = try c.decodeIfPresent([DeviceAccessWire].self, forKey: .device_access)
        }
        enum Keys: String, CodingKey { case members, you, device_access }
    }

    struct ClientsWire: Decodable {
        struct Client: Decodable {
            let clientId: String
            let label: String
            let shellAccess: Bool
            let foreign: Bool
            let addedBy: String

            init(from decoder: Decoder) throws {
                let c = try decoder.container(keyedBy: Keys.self)
                clientId = try c.decodeIfPresent(String.self, forKey: .clientId) ?? ""
                label = try c.decodeIfPresent(String.self, forKey: .label) ?? ""
                shellAccess = try c.decodeIfPresent(Bool.self, forKey: .shellAccess) ?? false
                foreign = try c.decodeIfPresent(Bool.self, forKey: .foreign) ?? false
                addedBy = try c.decodeIfPresent(String.self, forKey: .addedBy) ?? ""
            }
            enum Keys: String, CodingKey { case clientId, label, shellAccess, foreign, addedBy }
        }
        let clients: [Client]
    }
}
