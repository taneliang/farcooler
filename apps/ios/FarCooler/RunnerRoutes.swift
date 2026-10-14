import Foundation

/// The other route each runner has, kept beside the one it takes (ov-485).
///
/// A ceremony grants the tunnel first and keeps the runner's address as the
/// alternative; a runner paired before the tunnel gains a token through
/// `client.set_node_key`, kept here as ITS alternative. Nothing dials what is
/// kept here. It moves into the runner's `reach` only when a person answers
/// "Connect directly to … instead?" or taps the route row, and the route it
/// leaves takes its place here, so one route at a time holds both ways.
///
/// **Kept apart from ``Runner`` on purpose.** `FleetMembership.plan` rebuilds
/// a connection whose runner changed, so a token learned in the background
/// stored ON the runner would drop a working session to dial the same route
/// again. Here, it changes nothing until the route does.
@MainActor
final class RunnerRoutes: ObservableObject {
    static let shared = RunnerRoutes()

    @Published private(set) var alternatives: [UUID: Reach] = [:]
    private let key: String

    init(key: String = "runnerAlternatives") {
        self.key = key
        if let data = UserDefaults.standard.data(forKey: key),
            let stored = try? JSONDecoder().decode([String: Reach].self, from: data)
        {
            alternatives = Dictionary(
                uniqueKeysWithValues: stored.compactMap { id, reach in UUID(uuidString: id).map { ($0, reach) } })
        }
    }

    func alternative(for runner: UUID) -> Reach? { alternatives[runner] }

    func keep(_ reach: Reach?, for runner: UUID) {
        guard alternatives[runner] != reach else { return }
        alternatives[runner] = reach
        save()
    }

    /// The question to ask after `trouble`, or nil when there is none: a
    /// runner on the tunnel, a failure where nothing on it answered, and an
    /// address kept beside it.
    func offer(for runner: Runner, after trouble: RunnerTrouble?) -> DirectRouteOffer? {
        guard let trouble, case .direct(let host, let port)? = alternatives[runner.id] else { return nil }
        let onTheTunnel = { if case .tailcat = runner.reach { return true } else { return false } }()
        return TunnelRoute.offer(
            onTheTunnel: onTheTunnel, failedBeforeTheRunner: trouble.failedBeforeTheRunner,
            alternative: TunnelRoute.address(host: host, port: port))
    }

    /// The route a runner takes, when it has another to switch to.
    func route(of runner: Runner) -> TunnelRoute.Current? {
        guard alternatives[runner.id] != nil else { return nil }
        switch runner.reach {
        case .tailcat: return .tunnel
        case .direct(let host, let port): return .direct(address: TunnelRoute.address(host: host, port: port))
        }
    }

    private func save() {
        guard !RunnerStore.ephemeral else { return }
        let stored = Dictionary(uniqueKeysWithValues: alternatives.map { ($0.key.uuidString, $0.value) })
        guard let data = try? JSONEncoder().encode(stored) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }
}

extension RunnerStore {
    /// Move `host` to its other route, and keep the one it leaves as the
    /// alternative.
    ///
    /// The pinned host key stays: both routes reach the same sshd, and the key
    /// the granting device pinned is what each is held to (R-48). ``update(_:)``
    /// would drop it, because there a changed reach means somebody re-pointed
    /// the runner at a different box. Publishing the change is the reconnect:
    /// `FleetMembership.plan` rebuilds a connection whose runner changed.
    func switchRoute(_ host: Runner, routes: RunnerRoutes = .shared) {
        guard let other = routes.alternative(for: host.id) else { return }
        routes.keep(host.reach, for: host.id)
        replaceReach(of: host.id, with: other)
    }
}
