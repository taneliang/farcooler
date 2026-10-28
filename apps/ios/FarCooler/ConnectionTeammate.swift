import Foundation

extension Connection {
    /// Add a teammate's device to this runner (ov-854): its key written
    /// `for_teammate` at the scope chosen (never a shell line), bound to their
    /// member, and taken off again if either step fails. One call, because
    /// `crates/client/src/session/teammate.rs` holds the order once for both
    /// phones; this sends the words and reads the answer.
    ///
    /// Asked of a runner this phone administers only (`mayAdministerRunner`):
    /// binding a member and taking a key off are `host_admin`, and the client
    /// core answers `needs_admin` without sending anything otherwise.
    func enrollTeammate(
        publicKey: String, label: String, clientId: String, nodeKey: String, teammate: TeammateChoice
    ) async -> TeammateAddOutcome {
        guard phase == .connected else { return .notReached }
        do {
            let answered = try await core.call(
                TeammateAddRequest.method,
                TeammateAddRequest.arguments(
                    publicKey: publicKey, label: label, clientId: clientId, nodeKey: nodeKey,
                    teammate: teammate))
            return TeammateAddOutcome.decode(answered)
        } catch {
            return .notReached
        }
    }
}
