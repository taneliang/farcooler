import Foundation

extension DaemonClient {
    func startRunnerHealthChecks() {
        guard runnerHealthTask == nil else { return }
        runnerHealthTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(30))
                guard !Task.isCancelled, let self else { return }
                if self.state == .connected {
                    // The event stream can stay open and silent through a runner stall.
                    _ = await self.runnerRead(["root", "list", "--json"])
                }
            }
        }
    }

    func prepareLocalRunner() async {
        let timer = runnerDeadline()
        defer { timer.cancel() }
        await LocalDaemon.shared.ensure()
    }

    private func runnerDeadline() -> Task<Void, Never> {
        let limit = runnerResponseTimeout
        let generation = runnerReadGeneration
        // The UI timer also covers a launch that has not returned from the kernel yet.
        return Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(limit))
            if !Task.isCancelled, self?.runnerReadGeneration == generation { self?.runnerDidNotRespond() }
        }
    }

    func runnerRead(_ args: [String]) async -> (data: Data?, message: String?) {
        let generation = runnerReadGeneration
        let timer = runnerDeadline()
        defer { timer.cancel() }
        let result = await runRaw(args, background: true, deadline: runnerResponseTimeout)
        if !Task.isCancelled, runnerReadGeneration == generation, result.message == HostState.runnerNotRespondingReason {
            runnerDidNotRespond()
        }
        return result
    }
}
