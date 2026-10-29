import AgentKit
import Foundation

/// A runner this Mac reaches as a paired device, not as its owner (ov-856,
/// §3 of the design): the way a teammate's Mac, enrolled at `control`, asks the
/// runner for an admin action.
///
/// **This Mac keeps no step-up key (ov-858).** It is signed ad hoc, so macOS
/// refuses it a Secure Enclave key, and one it could make would not be hardware
/// bound. It asks the person's iPhone instead: the runner issues the challenge
/// for the exact action, the iPhone shows it and asks for Face ID, and the
/// request completes here once the iPhone has signed (`PhoneApproval`). Where
/// the iPhone has no trusted key the runner refuses (`no_approver`), and the
/// action fails closed.
///
/// The owner's own Mac is not this. It reaches a runner through ssh as the
/// owner, or through the runner's socket on its own machine, and needs no
/// approval.
struct RemoteStepUp {
    let approval: PhoneApproval
    let core: any StepUpCalling
    /// This Mac's id on the runner, derived from its paired key.
    let clientID: String
}

/// The client core, as `PhoneApproval` calls it.
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
    /// A paired connection to `target` that can ask the person's iPhone, or nil
    /// when this Mac isn't paired with it (the owner's ssh is how it talks to
    /// it then).
    static func open(target: String, pairing: RemotePairing = .shared) async -> RemoteStepUp? {
        guard case .connected(let core, _) = await pairing.connect(target: target), let clientID = pairing.key.clientID
        else { return nil }
        let calls = RunnerCoreStepUpCalls(core: core)
        return RemoteStepUp(approval: PhoneApproval(core: calls), core: calls, clientID: clientID)
    }
}
