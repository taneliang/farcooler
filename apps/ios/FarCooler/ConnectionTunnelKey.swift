import Foundation

// A runner this phone was paired with before the tunnel existed learns this
// phone's node key, over the access it already has (ov-485,
// `client.set_node_key`), so the fleet gains Tailcat without pairing again.
//
// **Kept, not switched.** The runner's answer is its tunnel token, and it goes
// beside the runner as its other route (`RunnerRoutes`), not in place of the
// address that is working right now: a connection that works is not dropped
// for a route nobody has tried. The runner's settings show it, and one tap
// moves it.

extension Connection {
    /// Ask once per runner and key per launch, after a connection comes up on
    /// an address, until the runner answers with its token. Nothing when this
    /// phone holds no node key, when the runner predates the tunnel, or when
    /// Tailcat is already beside it.
    func registerTunnelKey(for runner: Runner, routes: RunnerRoutes = .shared) {
        guard case .direct = runner.reach, routes.alternative(for: runner.id) == nil,
            // No pin, no tunnel: the tunnel route is held to the key this
            // phone already trusts (R-48), and an unpinned runner has none.
            let pin = runner.fingerprint, pin.hasPrefix("SHA256:"),
            let key = NodeIdentity.offeredPublicKey, !key.isEmpty
        else { return }
        let mark = "\(runner.id.uuidString)\n\(key)"
        guard TunnelKeyMarks.shared.begin(mark) else { return }
        Task { [weak self] in
            guard let self else { return }
            await self.loadDaemonBuild()
            guard self.daemon?.can(.tunnel) == true,
                let answer = try? await self.core.call("client.set_node_key", ["nodeKey": key]),
                let reply = try? JSONSerialization.jsonObject(with: answer) as? [String: Any]
            else { return }
            // Empty when the runner isn't serving a tunnel yet: the line is
            // written either way, and the next launch asks again (the runner
            // writes a key it already has once) until a token comes back.
            guard let token = reply["connBlob"] as? String, !token.isEmpty else { return }
            TunnelKeyMarks.shared.done(mark)
            routes.keep(.tailcat(token: token), for: runner.id)
        }
    }
}

/// Runner and key pairs whose token is kept, and the ones tried this launch.
@MainActor
final class TunnelKeyMarks {
    static let shared = TunnelKeyMarks()
    private let defaultsKey = "registeredNodeKeys"
    private var tried: Set<String> = []

    func begin(_ mark: String) -> Bool {
        let kept = Set(UserDefaults.standard.stringArray(forKey: defaultsKey) ?? [])
        return !kept.contains(mark) && tried.insert(mark).inserted
    }

    func done(_ mark: String) {
        var kept = Set(UserDefaults.standard.stringArray(forKey: defaultsKey) ?? [])
        kept.insert(mark)
        UserDefaults.standard.set(kept.sorted(), forKey: defaultsKey)
    }
}
