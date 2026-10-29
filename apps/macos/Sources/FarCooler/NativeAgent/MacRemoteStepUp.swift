import AgentKit
import Foundation

/// A runner this Mac reaches as a paired device, not as its owner (ov-856,
/// §3 of the design): the way a teammate's Mac, enrolled at `control`, asks the
/// runner for an admin action. It signs the request with a key in this Mac's
/// Secure Enclave, behind Touch ID, once for each action; the runner holds the
/// action to exactly the person, access and device that were signed.
///
/// The owner's own Mac is not this. It reaches a runner through ssh as the
/// owner, or through the runner's socket on its own machine, and needs no step-up.
struct RemoteStepUp {
    let stepUp: StepUp
    let core: any StepUpCalling
    /// This Mac's id on the runner, derived from its paired key.
    let clientID: String
}

/// The client core, as `StepUp` calls it.
struct RunnerCoreStepUpCalls: StepUpCalling {
    let core: RunnerCore

    func call(_ method: String, _ args: [String: Any]) async throws -> Data {
        try await core.call(method, json: RunnerCore.JSONArguments(value: args))
    }

    func refusalWord(_ error: Error) -> String? {
        guard let failure = error as? RunnerCore.Failure else { return nil }
        return failure.what ?? failure.word
    }
}

@MainActor
enum MacRemoteStepUp {
    /// This Mac's one step-up key: a Secure Enclave key that asks for Touch ID
    /// (or the password) each time it signs. A test swaps it.
    static var signer: any StepUpSigning = SecureEnclaveStepUpSigner(tag: "sg.elt.farcooler.step-up.mac")

    /// A paired connection to `target` with a way to step up, or nil when this
    /// Mac isn't paired with it (the owner's ssh is how it talks to it then),
    /// or has no pinned host key to sign for.
    static func open(target: String, pairing: RemotePairing = .shared) async -> RemoteStepUp? {
        guard case .connected(let core, _) = await pairing.connect(target: target),
            let pin = pairing.pin(for: target), pin.hasPrefix("SHA256:"), let clientID = pairing.key.clientID
        else { return nil }
        let calls = RunnerCoreStepUpCalls(core: core)
        return RemoteStepUp(
            stepUp: StepUp(core: calls, signer: signer, clientId: clientID, hostPin: pin),
            core: calls, clientID: clientID)
    }
}
