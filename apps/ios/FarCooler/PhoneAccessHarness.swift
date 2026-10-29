#if DEBUG
import AgentKit
import Foundation

/// `-phone-access` (ov-856, slice 4): the canned runner as the owner's phone
/// sees a runner that keeps the people's access. Cosmo is a Member with an
/// iPhone that added an iPad held at Viewer; Dan is an Admin whose phone
/// waits for an approval and whose Mac has a shell; a legacy line needs a
/// look. `-phone-access-trusted` makes this phone's own key trusted already;
/// `-phone-access-cancel` closes every prompt.
///
/// No face is asked: `HarnessStepUpSigner` signs at once and records what the
/// prompt would have said, which the test reads in `harness-sent`.
@MainActor
final class AccessStandIn {
    static var isOn: Bool { CommandLine.arguments.contains("-phone-access") }
    static var trusted: Bool { CommandLine.arguments.contains("-phone-access-trusted") }

    private var removed: Set<String> = []
    private var trustedKeys: Set<String> = trusted ? ["self"] : []
    private var proposedSelf = false
    private var prepared: [String: (method: String, args: [String: Any])] = [:]
    private var tokens = 0
    private let own: String

    init(own: String) {
        self.own = own
        StepUpSigners.shared = HarnessStepUpSigner()
    }

    /// The capabilities this runner adds.
    static let capabilities = ["person_access", "step_up", "enrollment", "representatives", "member_roles"]

    private func row(
        _ id: String, _ person: String, _ access: String, cap: String = "", check: Bool = false, key: String = "", code: String = ""
    ) -> [String: Any] {
        ["client_id": id, "person": person, "access": access, "cap": cap, "check": check, "step_up": key, "step_up_code": code, "step_up_at": 1]
    }

    private func selfKey() -> (String, String) {
        if trustedKeys.contains("self") { return ("trusted", "") }
        return proposedSelf ? ("proposed", "273905") : ("", "")
    }

    private func members() -> [String: Any] {
        let (state, code) = selfKey()
        var rows: [[String: Any]] = [
            row(own, "", "owner", key: state, code: code),
            row("mac-o", "", "owner", key: "trusted"),
            row("ipad-o", "", "owner", key: trustedKeys.contains("ipad-o") ? "trusted" : "proposed", code: trustedKeys.contains("ipad-o") ? "" : "418207"),
            row("legacy-t", "", "owner", cap: "member", check: true),
            row("phone-c", "cosmo", "member", key: "trusted"),
            row("pad-c", "cosmo", "member", cap: "viewer"),
            row("phone-d", "dan", "admin", key: trustedKeys.contains("phone-d") ? "trusted" : "proposed", code: trustedKeys.contains("phone-d") ? "" : "730412"),
            row("mac-d", "dan", "admin", key: "trusted"),
        ]
        rows.removeAll { removed.contains($0["client_id"] as! String) }
        func devices(_ person: String) -> [String] {
            rows.filter { $0["person"] as? String == person }.map { $0["client_id"] as! String }
        }
        return [
            "members": [
                ["handle": "cosmo", "name": "Cosmo", "devices": devices("cosmo"), "reps": 0, "rep_panes": [Any](), "access": "member", "role": "member"],
                ["handle": "dan", "name": "Dan", "devices": devices("dan"), "reps": 0, "rep_panes": [Any](), "access": "admin", "role": "member"],
            ],
            "claude_account": "owner@example.com", "you": "", "device_access": rows,
        ]
    }

    private func line(_ id: String, _ label: String, shell: Bool = false, by: String = "") -> [String: Any] {
        ["clientId": id, "fingerprint": "SHA256:x", "label": label, "scope": "control", "account": "", "enrolledAt": 0, "foreign": false, "shellAccess": shell, "addedBy": by]
    }

    private func clients() -> [String: Any] {
        let all = [
            line(own, "farcooler-iPhone-aaaa1111"), line("mac-o", "farcooler-Studio-bbbb2222"), line("ipad-o", "farcooler-iPad-hhhh8888"),
            line("legacy-t", "farcooler-Old-iPad-cccc3333"),
            line("phone-c", "farcooler-Cosmos-iPhone-dddd4444"), line("pad-c", "farcooler-Cosmos-iPad-eeee5555", by: "phone-c"),
            line("phone-d", "farcooler-Dans-iPhone-ffff6666"),
            line("mac-d", "farcooler-Dans-Mac-gggg7777"), line("mac-d", "farcooler-shell-Dans-Mac-gggg7777.mac-d", shell: true),
        ]
        return ["clients": all.filter { !removed.contains($0["clientId"] as! String) }]
    }

    /// The canned runner's answer, or nil for a call that isn't about access.
    func answer(_ method: String, _ args: [String: Any], record: (String) -> Void) throws -> Any? {
        switch method {
        case "member.list": return members()
        case "client.list": return clients()
        case "step_up.propose":
            proposedSelf = true
            record("propose key")
            return ["clientId": own, "state": "proposed", "code": "273905"]
        case "step_up.begin":
            tokens += 1
            let token = "t-\(tokens)"
            prepared[token] = (args["method"] as? String ?? "", args["args"] as? [String: Any] ?? [:])
            return ["token": token, "statement": Data("harness statement".utf8).base64EncodedString()]
        case "step_up.finish":
            guard let token = args["token"] as? String, let action = prepared.removeValue(forKey: token) else {
                throw ClientCore.CoreError.rejected("spent", word: "invalid-argument")
            }
            record("prompt \(HarnessStepUpSigner.last)")
            return try apply(action.method, action.args, record: record)
        case "client.revoke", "client.approve_step_up":
            // The plain call: a phone with a trusted key must sign.
            if trustedKeys.contains("self") {
                throw ClientCore.CoreError.rejected("step up", word: "resource-conflict", what: "step_up_needed")
            }
            return try apply(method, args, record: record)
        default: return nil
        }
    }

    private func apply(_ method: String, _ args: [String: Any], record: (String) -> Void) throws -> Any {
        switch method {
        case "client.revoke":
            let id = args["clientId"] as? String ?? ""
            removed.insert(id)
            if args["withAdded"] as? Bool == true, id == "phone-c" { removed.insert("pad-c") }
            record("revoke \(id) withAdded=\(args["withAdded"] as? Bool ?? false)")
            return clients()
        case "client.approve_step_up":
            let id = args["clientId"] as? String ?? ""
            guard args["code"] as? String == (id == "ipad-o" ? "418207" : "730412") else {
                throw ClientCore.CoreError.rejected("code", word: "resource-conflict", what: "wrong_code")
            }
            trustedKeys.insert(id)
            record("approve \(id)")
            return ["clientId": id, "state": "trusted", "code": ""]
        default:
            throw ClientCore.CoreError.rejected("no", word: "invalid-argument")
        }
    }
}

/// A signer that asks nobody and remembers what it was asked.
final class HarnessStepUpSigner: StepUpSigning, @unchecked Sendable {
    nonisolated(unsafe) static var last = ""

    var publicKey: Data? { Data(repeating: 4, count: 65) }
    func makeKey() throws -> Data { Data(repeating: 4, count: 65) }

    func sign(_ statement: Data, reason: String) async throws -> Data {
        if CommandLine.arguments.contains("-phone-access-cancel") { throw StepUpError.cancelled }
        Self.last = reason
        return Data(repeating: 1, count: 70)
    }
}
#endif
