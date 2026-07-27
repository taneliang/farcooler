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

    /// Where the built CLI lives. Overridable so the app runs from a checkout.
    private var binary: String {
        if let override = ProcessInfo.processInfo.environment["OVERNIGHT_BIN"] {
            return override
        }
        return "/usr/local/bin/overnight"
    }

    private var environment: [String: String] {
        var env = ProcessInfo.processInfo.environment
        // Keep the app pointed at the same runtime the CLI uses.
        if env["OVERNIGHT_HOME"] == nil, let home = env["HOME"] {
            env["OVERNIGHT_HOME"] =
                "\(home)/Library/Application Support/Overnight"
        }
        return env
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

        let bin = binary
        guard FileManager.default.isExecutableFile(atPath: bin) else {
            lastError =
                "overnight CLI not found at \(bin). Set OVERNIGHT_BIN to the built binary."
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
