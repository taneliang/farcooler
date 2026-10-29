import Foundation

extension Connection {
    /// What this phone is on this runner and who is there already (ov-856):
    /// what Add Teammate offers, and whether the runner is asked at all. Nil
    /// when the runner can't be asked or doesn't record the people's access.
    func teammateFacts(account: String) async -> TeammateRunnerFacts? {
        guard phase == .connected, daemon?.can(.personAccess) == true,
            let members = try? await rpc("member.list"),
            var facts = TeammateRunnerFacts(members: members, account: account, thisDevice: ownClientId ?? "")
        else { return nil }
        // The step-up key is read from the device rows only when the runner
        // lists this device, which a phone below Control isn't told about.
        if facts.caller == .owner || facts.caller == .admin { facts.signs = facts.signs && daemon?.can(.stepUp) == true }
        return facts
    }

    /// Add a teammate's device to this runner (ov-856): one request that
    /// writes the line `for_teammate` at the access's scope and binds it to the
    /// person, in one write, signed with Face ID for exactly that person, access
    /// and device. The runner holds the rules: only an Owner makes an Admin.
    ///
    /// A phone that holds no trusted key there sends it plain and is told to
    /// allow admin actions from a Mac first, in a sentence.
    func enrollTeammate(
        publicKey: String, label: String, clientId: String, nodeKey: String, teammate: TeammateChoice,
        facts: TeammateRunnerFacts?
    ) async -> TeammateAddOutcome {
        guard phase == .connected else { return .notReached }
        let runner = hostLabel
        let args = TeammateAccessRules.arguments(
            publicKey: publicKey, label: label, clientId: clientId, nodeKey: nodeKey, teammate: teammate, facts: facts)
        let bound = args["teammate"] as? [String: Any] ?? [:]
        let reason = TeammateAccessRules.reason(device: label, runner: runner, access: bound["access"] as? String ?? "member")
        guard let stepUp = stepUp() else {
            return .declined(StepUpWords.sentence(for: .keyUnavailable, runner: runner, device: HardwareKind.kind) ?? "")
        }
        do {
            let answered = try await stepUp.perform("client.enroll", args, reason: reason, hasTrustedKey: facts?.signs == true)
            let object = (try? JSONSerialization.jsonObject(with: answered)) as? [String: Any]
            return .written(tunnel: object?["connBlob"] as? String ?? "")
        } catch let error as StepUpError {
            return .declined(StepUpWords.sentence(for: error, runner: runner, device: HardwareKind.kind) ?? "")
        } catch {
            return .declined(TeammateAccessRules.refusal(StepUpRefusals.word(error), runner: runner, device: HardwareKind.kind))
        }
    }
}
