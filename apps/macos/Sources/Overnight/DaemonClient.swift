import Foundation

/// Talks to the daemon through the `overnight` CLI.
///
/// DEVIATION FROM THE DESIGN, recorded honestly: the accepted architecture has
/// the Mac app speak the protobuf protocol over the daemon's Unix socket. Night
/// one drives the CLI as a subprocess instead, because the socket transport was
/// still being built. The boundary is the same either way: the app never calls
/// git, SQLite, or tmux itself, and it never derives state. Swapping this one
/// file for a socket client is the whole migration.
@MainActor
final class DaemonClient: ObservableObject {
    @Published var fleet: Fleet = .empty
    @Published var lastError: String?
    @Published var busy = false

    /// Where the CLI lives.
    ///
    /// The bundled copy comes first, because an app launched from the Dock
    /// inherits no shell environment and cannot find something that only exists
    /// on your PATH. The env override and PATH lookups are for running from a
    /// checkout during development.
    private var binary: String? {
        var candidates: [String] = []

        if let bundled = Bundle.main.resourceURL?
            .appendingPathComponent("overnight").path
        {
            candidates.append(bundled)
        }
        if let override = ProcessInfo.processInfo.environment["OVERNIGHT_BIN"] {
            candidates.append(override)
        }
        candidates += ["/usr/local/bin/overnight", "\(NSHomeDirectory())/.local/bin/overnight"]

        return candidates.first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// Deliberately does NOT set OVERNIGHT_HOME.
    ///
    /// The CLI already resolves its own runtime directory. Setting a second
    /// guess here would point the app at a different database than the CLI uses
    /// from your shell, and the two would silently disagree about what exists.
    private var environment: [String: String] {
        ProcessInfo.processInfo.environment
    }

    // MARK: - Commands

    func refresh() async {
        guard let data = await run(["workspace", "list", "--json"]) else { return }
        do {
            fleet = try JSONDecoder().decode(Fleet.self, from: data)
            lastError = nil
        } catch {
            lastError = "could not read fleet: \(error.localizedDescription)"
        }
    }

    func capture(terminal: String, lines: Int = 400) async -> String {
        guard let data = await run(["terminal", "read", terminal, "--lines", "\(lines)"])
        else { return "" }
        return String(data: data, encoding: .utf8) ?? ""
    }

    func send(terminal: String, text: String) async {
        _ = await run(["terminal", "send", terminal, text])
    }

    func createTerminal(workspace: String, preset: String) async {
        _ = await run(["terminal", "create", workspace, "--preset", preset])
        await refresh()
    }

    func restart(terminal: String) async {
        _ = await run(["terminal", "restart", terminal])
        await refresh()
    }

    func dismissLost(terminal: String) async {
        _ = await run(["terminal", "dismiss-lost", terminal])
        await refresh()
    }

    func stop(terminal: String) async {
        _ = await run(["terminal", "stop", terminal])
        await refresh()
    }

    // MARK: - Subprocess

    @discardableResult
    private func run(_ args: [String]) async -> Data? {
        busy = true
        defer { busy = false }

        guard let bin = binary else {
            lastError =
                "The overnight CLI was not found. Rebuild the app with "
                + "apps/macos/build-app.sh so it bundles one, or set OVERNIGHT_BIN."
            return nil
        }

        let env = environment
        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: bin)
                process.arguments = args
                process.environment = env

                let out = Pipe()
                let err = Pipe()
                process.standardOutput = out
                process.standardError = err

                do {
                    try process.run()
                } catch {
                    Task { @MainActor in self.lastError = error.localizedDescription }
                    continuation.resume(returning: nil)
                    return
                }

                let stdout = out.fileHandleForReading.readDataToEndOfFile()
                let stderr = err.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()

                if process.terminationStatus != 0 {
                    let message =
                        String(data: stderr, encoding: .utf8)?
                        .trimmingCharacters(in: .whitespacesAndNewlines) ?? "command failed"
                    Task { @MainActor in self.lastError = message }
                    continuation.resume(returning: nil)
                    return
                }

                continuation.resume(returning: stdout)
            }
        }
    }
}
