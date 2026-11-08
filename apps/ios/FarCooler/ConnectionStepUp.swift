import FarCoolerClient
import Foundation

/// This phone's one step-up key (ov-856): a Secure Enclave key that asks for
/// Face ID or the passcode each time it signs. One per phone, offered to each
/// runner that is asked to trust it. A harness swaps it for one that
/// answers without a face.
enum StepUpSigners {
    nonisolated(unsafe) static var shared: any StepUpSigning = SecureEnclaveStepUpSigner()
}

/// The client core, as step-up calls it.
struct CoreStepUpCalls: StepUpCalling {
    let send: (String, [String: Any]) async throws -> Data

    func call(_ method: String, _ args: [String: Any]) async throws -> Data {
        try await send(method, args)
    }

    func refusalWord(_ error: Error) -> String? {
        guard case ClientCore.CoreError.rejected(_, let word, let what) = error else { return nil }
        return what ?? word
    }
}

extension Connection {
    /// The id this phone is under on its runners: derived in Rust from its
    /// key, the way its enrollment named it.
    var ownClientId: String? {
        Identity.publicKey.flatMap { CeremonyCore.text(of: $0, farcooler_client_client_id) }
    }

    /// Step-up for this runner, or nil when the phone can't make one yet: it
    /// needs its own id and the runner's pinned host key (a runner taken on
    /// trust, `accept-any`, has none to sign).
    func stepUp() -> StepUp? {
        guard let pin = hostPin, pin.hasPrefix("SHA256:"), let id = ownClientId else { return nil }
        return StepUp(
            core: CoreStepUpCalls { [weak self] method, args in
                guard let self else { throw StepUpError.keyUnavailable }
                return try await self.rpc(method, args)
            },
            signer: StepUpSigners.shared, clientId: id, hostPin: pin)
    }

    /// Who is here and what each may do, with the devices under them: the
    /// People screen's rows, from `member.list` and `client.list`.
    func loadPeopleAccess(members: Data) async -> PeopleAccess? {
        let clients = try? await rpc("client.list")
        return PeopleAccess(members: members, clients: clients, thisDevice: ownClientId ?? "", ownerName: "Owner")
    }
}

extension Connection {
    /// "Allow cutover once" (ov-974, ov-977): signed with this phone's
    /// step-up key after Face ID or the passcode, which is what tells the
    /// runner the owner said it. Never sent plain: a plain one grants nothing,
    /// so a phone with no trusted key is told how to get one instead.
    func answerCutoverGrant(task: String) async -> String? {
        let runner = hostLabel
        let device = HardwareKind.kind
        guard let stepUp = stepUp(), stepUp.signer.publicKey != nil else {
            return StepUpWords.sentence(forRefusal: "key_untrusted", runner: runner, device: device)
        }
        var refused: String?
        do {
            _ = try await stepUp.perform(
                PhoneTaskAnswer.method, CutoverGrant.request(task: task),
                reason: StepUpReason.allowCutover(runner: runner), hasTrustedKey: true)
        } catch let error as StepUpError {
            // Closing the prompt sends nothing; the row stays to answer again.
            refused = StepUpWords.sentence(for: error, runner: runner, device: device) ?? "Your answer wasn’t sent."
        } catch {
            refused = StepUpWords.sentence(forRefusal: StepUpRefusals.word(error), runner: runner, device: device)
        }
        await loadNeedsYou()
        return refused
    }
}
