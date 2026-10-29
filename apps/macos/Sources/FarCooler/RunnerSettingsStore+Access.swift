import AgentKit
import Foundation

/// The People screen's Access, devices and keys on this Mac (ov-856, §8).
///
/// The runner's own socket makes the changes (`farcooler people access`,
/// `people limit`, `people approve`, `client revoke`), so none of them asks
/// for a face: this Mac is where the runner's owner is. A runner reached over
/// ssh as the owner needs none either. A Mac that is not the runner and has no
/// shell on it (a teammate's, paired at `control`) steps up like a phone does,
/// with Touch ID once for each action (`MacRemoteStepUp`).
extension RunnerSettingsStore {
    /// Read the devices behind `members` (`member list --json`) when the
    /// runner records the people's access.
    func loadAccess(members: Data) async {
        guard let text = String(data: members, encoding: .utf8), text.contains("\"device_access\"") else {
            setAccess(nil)
            return
        }
        let clients = await cli(hostArguments + ["--json", "client", "list"])
        setAccess(PeopleAccess(members: members, clients: clients.ok ? Data(clients.output.utf8) : nil, isSocket: true))
    }

    /// Read the people as this Mac's paired device, over the client core: what a
    /// Mac without a shell on the runner can see and sign for.
    func loadAsPairedDevice() async {
        guard let paired = await stepUpOpener(target) else { return }
        do {
            let members = try await paired.core.call("member.list", [:])
            let clients = try? await paired.core.call("client.list", [:])
            guard let read = PeopleAccess(members: members, clients: clients, thisDevice: paired.clientID), read.recorded else { return }
            remote = paired
            setPeople(RunnerPeople(json: members))
            setAccess(read)
        } catch {
            return
        }
    }

    /// What the section can do here: everything through the socket or the
    /// owner's ssh, or what this Mac can sign for as a paired device.
    func accessActions() -> PeopleAccessActions {
        if let remote { return signedActions(remote) }
        return PeopleAccessActions(
            removeDevice: { [weak self] device, withAdded in
                await self?.change(["client", "revoke", device.id] + (withAdded ? ["--with-added"] : []))
            },
            limitToViewer: { [weak self] device in await self?.change(["people", "limit", device.id, "viewer"]) },
            setAccess: { [weak self] person, access in await self?.change(["people", "access", person.handle, access.rawValue]) },
            approve: { [weak self] device in await self?.change(["people", "approve", device.id, device.code]) })
    }

    /// The actions a paired Mac signs, each with Touch ID for exactly that
    /// action. Limit and Access are offered once this Mac's key is trusted.
    private func signedActions(_ remote: RemoteStepUp) -> PeopleAccessActions {
        let trusted = access?.thisDevice?.stepUp == .trusted
        let runner = target.isEmpty ? "this Mac" : target
        return PeopleAccessActions(
            removeDevice: { [weak self] device, withAdded in
                let reason = StepUpReason.removeDevice(device.name, alsoRemoving: withAdded ? device.added : 0, runner: runner)
                await self?.sign("client.revoke", ["clientId": device.id, "withAdded": withAdded], reason, remote)
            },
            limitToViewer: trusted
                ? { [weak self] device in
                    await self?.sign("client.set_cap", ["clientId": device.id, "cap": "viewer"], StepUpReason.limit(device.name, runner: runner), remote)
                } : nil,
            setAccess: trusted
                ? { [weak self] person, new in
                    await self?.sign("member.set_access", ["handle": person.handle, "access": new.rawValue], StepUpReason.setAccess(person.name, to: new, runner: runner), remote)
                } : nil,
            approve: { [weak self] device in
                await self?.sign("client.approve_step_up", ["clientId": device.id, "code": device.code], StepUpReason.approve(device: device.name), remote)
            },
            allowAdminActions: { [weak self] in await self?.offerKey(remote) })
    }

    /// One signed action, then the list as the runner has it now. A refusal is a
    /// sentence in `failure`, never the runner's own words.
    private func sign(_ method: String, _ args: [String: Any], _ reason: String, _ remote: RemoteStepUp) async {
        let runner = target.isEmpty ? "this Mac" : target
        do {
            let trusted = access?.thisDevice?.stepUp == .trusted
            let call = Unchecked((remote.stepUp, args))
            // Off the main actor: the prompt waits for a finger, and the lists are read again after.
            _ = try await Task.detached { try await call.value.0.perform(method, call.value.1, reason: reason, hasTrustedKey: trusted) }.value
            failure = nil
            await loadPeople()
        } catch let error as StepUpError {
            failure = StepUpWords.sentence(for: error, runner: runner, device: "Mac")
        } catch {
            let word = remote.core.refusalWord(error) ?? Self.word(of: error.localizedDescription)
            failure = StepUpWords.sentence(forRefusal: word, runner: runner, device: "Mac")
        }
    }

    /// Offer this Mac's key for another of the person's devices to approve.
    private func offerKey(_ remote: RemoteStepUp) async {
        let runner = target.isEmpty ? "this Mac" : target
        do {
            let stepUp = Unchecked(remote.stepUp)
            shownCode = try await Task.detached { try await stepUp.value.register() }.value
            failure = nil
            await loadPeople()
        } catch let error as StepUpError {
            failure = StepUpWords.sentence(for: error, runner: runner, device: "Mac")
        } catch {
            let word = remote.core.refusalWord(error) ?? Self.word(of: error.localizedDescription)
            failure = StepUpWords.sentence(forRefusal: word, runner: runner, device: "Mac")
        }
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
    static func word(of said: String) -> String {
        for word in ["step_up_needed", "key_untrusted", "socket_only", "person_only", "wrong_code", "proposal_expired", "not_yours", "not_admin", "step_up_failed", "challenge_expired", "defensive_only"] where said.contains(word) {
            return word
        }
        return ""
    }
}

/// A value handed to a task that uses it alone: `StepUp` and its JSON arguments
/// are plain values the Mac builds for one action and then forgets.
struct Unchecked<Value>: @unchecked Sendable {
    let value: Value
    init(_ value: Value) { self.value = value }
}
