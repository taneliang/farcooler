import Foundation

/// A Mac that isn't the runner has an admin action approved on the person's
/// iPhone (ov-858, option (c) of the card; §3 of the ov-856 design).
///
/// **Why:** the Mac app is signed ad hoc, so it can't keep a Secure Enclave
/// key, and its own step-up fails closed. The Mac asks instead, the runner
/// issues a challenge for that exact action and pushes the person's iPhone,
/// the iPhone shows the action and which Mac asked, asks for Face ID, and signs
/// with its own trusted step-up key. The Mac's request then completes.
///
/// **What this file owns:** the Mac's side (`PhoneApproval`: ask, wait, send)
/// and the iPhone's reading of a request (`PhoneApprovalItem`: what to show,
/// and the statement to sign). The bytes are the client core's
/// (`crates/client/src/session/approval.rs`), the same on every platform.

/// The Mac's side: one admin action, asked of the person's iPhone.
public struct PhoneApproval {
    public let core: StepUpCalling
    /// How long to wait between two looks at the approval.
    public let pause: UInt64
    /// The longest the Mac waits, counted in looks; the runner's own window
    /// ends it sooner.
    public let looks: Int

    public init(core: StepUpCalling, pause: UInt64 = 1_000_000_000, looks: Int = 150) {
        self.core = core
        self.pause = pause
        self.looks = looks
    }

    /// `method` with `args` (the ones the plain call takes): ask, wait until the
    /// iPhone answers, then send the approved request and answer what the
    /// plain call answers. `waiting` is told when the wait begins and ends, for
    /// the Mac to say so.
    public func perform(_ method: String, _ args: [String: Any], waiting: (Bool) -> Void) async throws -> Data {
        let asked = try await core.call("approval.ask", ["method": method, "args": args])
        guard let id = Self.field(asked, "id") else { throw StepUpError.keyUnavailable }
        waiting(true)
        defer { waiting(false) }
        for _ in 0..<looks {
            try Task.checkCancellation()
            let state = Self.field(try await core.call("approval.status", ["id": id]), "state")
            switch state {
            case "granted": return try await core.call("approval.finish", ["id": id])
            case "denied": throw StepUpError.refused("approval_denied")
            case "expired": throw StepUpError.refused("approval_expired")
            default: try await Task.sleep(nanoseconds: pause)
            }
        }
        throw StepUpError.refused("approval_expired")
    }

    private static func field(_ data: Data, _ key: String) -> String? {
        (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?[key] as? String
    }
}

/// One request the iPhone is asked to approve, as `approval.list` answers it:
/// what the bytes say, which Mac asked, and the statement to sign.
public struct PhoneApprovalItem: Identifiable, Equatable, Sendable {
    public let id: String
    /// The wire method decoded from the bytes that are signed.
    public let method: String
    public let clientId: String
    public let handle: String
    public let access: String
    public let cap: String
    public let role: String
    public let label: String
    public let withAdded: Bool
    /// The asking Mac's name, in the runner's words.
    public let deviceName: String
    /// What the action acts on (a device or a person), in the runner's words.
    public let targetName: String
    /// Milliseconds since the epoch when the runner stops waiting.
    public let expiresAt: Int64
    /// What the iPhone's key signs, for the Mac's id and this action.
    public let statement: Data

    /// The items in `list` (`{"items": [...]}`), leaving out any that can't be read.
    public static func items(in list: Data) -> [PhoneApprovalItem] {
        guard let object = try? JSONSerialization.jsonObject(with: list) as? [String: Any],
            let items = object["items"] as? [[String: Any]]
        else { return [] }
        return items.compactMap(PhoneApprovalItem.init(json:))
    }

    init?(json: [String: Any]) {
        guard let id = json["id"] as? String, let action = json["action"] as? [String: Any],
            let method = action["method"] as? String, let encoded = json["statement"] as? String,
            let statement = Data(base64Encoded: encoded)
        else { return nil }
        let detail = action["detail"] as? [String: Any] ?? [:]
        let text = { (key: String) in detail[key] as? String ?? "" }
        self.id = id
        self.method = method
        self.clientId = text("clientId")
        self.handle = text("handle")
        self.access = text("access")
        self.cap = text("cap")
        self.role = text("role")
        self.label = text("label")
        self.withAdded = detail["withAdded"] as? Bool ?? false
        self.deviceName = json["deviceName"] as? String ?? ""
        self.targetName = json["targetName"] as? String ?? ""
        self.expiresAt = (json["expiresAt"] as? NSNumber)?.int64Value ?? 0
        self.statement = statement
    }

    /// The Mac, as the sheet names it.
    public var mac: String { deviceName.isEmpty ? "A Mac" : deviceName }

    /// What approving does, in a sentence, for `runner`: the action and its
    /// target. The prompt for Face ID says it too.
    public func sentence(runner: String) -> String {
        let target = targetName.isEmpty ? "that device" : targetName
        switch method {
        case "client.revoke":
            return withAdded ? "Remove \(target) and the devices it added." : StepUpReason.removeDevice(target, alsoRemoving: 0, runner: runner)
        case "client.set_cap":
            return cap == "viewer" ? StepUpReason.limit(target, runner: runner)
                : cap.isEmpty ? "Lift the limit on \(target) on \(runner)." : "Limit \(target) on \(runner) to Member."
        case "member.set_access":
            guard let new = PersonAccess(rawValue: access) else { return "Change \(target)’s access on \(runner)." }
            return StepUpReason.setAccess(target, to: new, runner: runner)
        case "client.approve_step_up":
            return StepUpReason.approve(device: target)
        case "client.enroll":
            return "Add \(label.isEmpty ? "a new device" : label) to \(runner)."
        case "member.set_role":
            return "Change \(target)’s role on \(runner)."
        default:
            return "Change who has access to \(runner)."
        }
    }
}

/// What the Mac and the iPhone say about an approval.
public enum PhoneApprovalWords {
    public static let waiting = "Waiting for your iPhone…"
    public static let title = "Approve This Change?"
    public static let approve = "Approve"
    public static let decline = "Don’t Allow"

    /// "Dan’s Mac asked for this. Only approve it if that was you."
    public static func asked(by mac: String) -> String {
        "\(mac) asked for this. Only approve it if that was you."
    }
}
