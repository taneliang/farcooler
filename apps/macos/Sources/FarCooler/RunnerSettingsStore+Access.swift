import AgentKit
import Foundation

/// The People screen's Access, devices and keys on this Mac (ov-856, §8).
///
/// The runner's own socket makes the changes (`farcooler people access`,
/// `people limit`, `people approve`, `client revoke`), so none of them asks
/// for a face: this Mac is where the runner's owner is. A runner reached over
/// ssh as a remote device steps up like a phone does, which this window does
/// not yet do.
extension RunnerSettingsStore {
    /// Read the devices behind `members` (`member list --json`) when the
    /// runner records the people's access.
    func loadAccess(members: Data) async {
        guard let text = String(data: members, encoding: .utf8), text.contains("\"device_access\"") else {
            setAccess(nil)
            return
        }
        let clients = await cli(hostArguments + ["--json", "client", "list"])
        setAccess(PeopleAccess(members: members, clients: clients.ok ? Data(clients.output.utf8) : nil))
    }

    /// What the section can do here: everything, through the socket.
    func accessActions() -> PeopleAccessActions {
        PeopleAccessActions(
            removeDevice: { [weak self] device, withAdded in
                await self?.change(["client", "revoke", device.id] + (withAdded ? ["--with-added"] : []))
            },
            limitToViewer: { [weak self] device in await self?.change(["people", "limit", device.id, "viewer"]) },
            setAccess: { [weak self] person, access in await self?.change(["people", "access", person.handle, access.rawValue]) },
            approve: { [weak self] device in await self?.change(["people", "approve", device.id, device.code]) })
    }

    /// One change, then the list as the runner has it now. A refusal is a
    /// sentence in `failure`, never the runner's own words.
    private func change(_ args: [String]) async {
        let result = await cli(hostArguments + ["--json"] + args)
        guard result.ok else {
            failure = StepUpWords.sentence(forRefusal: Self.word(of: result.said), runner: target.isEmpty ? "this Mac" : target, device: "Mac")
            return
        }
        failure = nil
        await loadPeople()
    }

    /// The runner's word in a CLI refusal, or "".
    private static func word(of said: String) -> String {
        for word in ["step_up_needed", "socket_only", "person_only", "wrong_code", "proposal_expired", "not_yours", "not_admin"] where said.contains(word) {
            return word
        }
        return ""
    }
}
