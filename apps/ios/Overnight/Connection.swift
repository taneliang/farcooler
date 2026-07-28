import Foundation

/// One host's session and the state a view renders from it.
///
/// The rule the whole product rests on holds here too: this never computes a
/// terminal's state. It asks, and shows what the daemon derived. A phone that
/// re-derived could disagree with the daemon and with the Mac about the same
/// terminal, which is exactly the confusion the design removed everywhere else.
@MainActor
final class Connection: ObservableObject {
    enum Phase: Equatable {
        case connecting
        /// First contact: the host's fingerprint, awaiting a human.
        case needsApproval(String)
        case failed(String)
        case connected
    }

    enum Action {
        case restart, stop, dismissLost
    }

    @Published private(set) var phase: Phase = .connecting
    @Published private(set) var fleet: Fleet = .empty
    @Published private(set) var repositories: [Repository] = []

    private let core = ClientCore()
    private var poller: Task<Void, Never>?

    deinit { poller?.cancel() }

    func start(host: Host) async {
        poller?.cancel()
        phase = .connecting

        guard let key = Identity.privateKey() else {
            phase = .failed("This device has no SSH key and one could not be generated.")
            return
        }

        do {
            _ = try await core.connect(config: host.config(privateKey: key))
        } catch {
            phase = classify(error)
            return
        }

        phase = .connected
        await refresh()
        await loadRepositories()
        startPolling()
    }

    /// Turn the core's message into a phase a view can act on.
    ///
    /// The unknown-host case is not a failure — it is a question — and it has
    /// to be told apart from one, or the user is shown "try again" for
    /// something retrying will never fix.
    private func classify(_ error: Error) -> Phase {
        let message = error.localizedDescription
        if let fingerprint = fingerprint(in: message) {
            return .needsApproval(fingerprint)
        }
        return .failed(message)
    }

    private func fingerprint(in message: String) -> String? {
        guard message.contains("is unknown") else { return nil }
        return message
            .split(separator: " ")
            .first { $0.hasPrefix("SHA256:") }
            .map(String.init)
    }

    func refresh() async {
        guard phase == .connected else { return }
        do {
            let data = try await core.call("fleet")
            fleet = try JSONDecoder().decode(Fleet.self, from: data)
        } catch {
            // A failed poll is not a disconnection. Keep showing the last known
            // fleet rather than blanking the screen someone is reading.
            return
        }
    }

    private func loadRepositories() async {
        guard let data = try? await core.call("repositories") else { return }
        repositories = (try? JSONDecoder().decode(RepositoryList.self, from: data))?.repositories ?? []
    }

    /// Poll while the view is up.
    ///
    /// Three seconds, not sub-second: every poll is an SSH round trip, and this
    /// is a phone with a battery. The states that matter here change on the
    /// scale of an agent finishing a task, not a keystroke.
    private func startPolling() {
        poller = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(3))
                await self?.refresh()
            }
        }
    }

    func act(_ action: Action, on terminal: Terminal) async {
        let method: String
        switch action {
        case .restart: method = "terminal.restart"
        case .stop: method = "terminal.stop"
        case .dismissLost: method = "terminal.dismiss_lost"
        }
        _ = try? await core.call(method, ["terminal": terminal.id])
        await refresh()
    }

    func createWorkspace(repository: String, task: String, branch: String) async {
        _ = try? await core.call(
            "workspace.create",
            ["repository": repository, "task": task, "branch": branch])
        await refresh()
    }
}
