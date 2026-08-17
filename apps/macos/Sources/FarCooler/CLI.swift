import Foundation

/// Where the `farcooler` CLI is, and what to run it with.
///
/// Extracted from `DaemonClient` when a second caller appeared: `Hosts` runs
/// `host probe` and `host install` from the Settings window, which has no
/// `DaemonClient` of its own. Two copies of this lookup would be two answers to
/// "which binary is this app driving" — and the whole reason the app bundles the
/// CLI is so that answer is never in doubt.
enum CLI {
    /// The bundled CLI, or the nearest thing to it.
    ///
    /// The bundle comes first on purpose. A double-clicked app inherits no
    /// shell environment, so a binary that only exists on your `PATH` is a
    /// binary this process cannot find; and the bundled one is guaranteed to
    /// match the daemon shipped beside it.
    static var binary: String? {
        var candidates: [String] = []

        // Bare inside the bundle, because `build-app.sh` copies what cargo
        // produced and cargo produces `farcooler` on every channel. There is
        // only one CLI in here, so there is nothing to tell apart.
        if let bundled = Bundle.main.resourceURL?
            .appendingPathComponent("farcooler").path
        {
            candidates.append(bundled)
        }
        if let override = ProcessInfo.processInfo.environment["FARCOOLER_BIN"] {
            candidates.append(override)
        }
        // Channel-named outside it, because those directories are shared. A
        // Mac with the release app installed has `/usr/local/bin/farcooler`,
        // and a canary app falling through to it would run the release CLI
        // against the release daemon — the app would look like it was working,
        // on the wrong fleet, with nothing anywhere saying so.
        let onPath = CommandLineTools.channelName(for: "farcooler")
        candidates += ["/usr/local/bin/\(onPath)", "\(NSHomeDirectory())/.local/bin/\(onPath)"]

        return candidates.first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// Deliberately does NOT set FARCOOLER_HOME.
    ///
    /// The CLI already resolves its own runtime directory. Setting a second
    /// guess here would point the app at a different database than the CLI uses
    /// from your shell, and the two would silently disagree about what exists.
    static var environment: [String: String] {
        ProcessInfo.processInfo.environment
    }

    /// Run the CLI and hand back everything it said.
    ///
    /// Both streams, and the exit status, because the callers here are about
    /// INSTALLING software on someone else's runner: an installer that failed
    /// halfway is exactly when its own words matter, and they arrive on stderr.
    /// - Parameter stdin: fed to the process and closed. For credentials: an
    ///   argument is visible in `ps` to every process on the machine, and a
    ///   pipe is not.
    static func run(_ args: [String], stdin: String? = nil) async -> (ok: Bool, output: String) {
        guard let binary else {
            return (false, "The farcooler CLI was not found.")
        }
        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: binary)
                process.arguments = args
                process.environment = environment

                let out = Pipe()
                let err = Pipe()
                process.standardOutput = out
                process.standardError = err

                let input = stdin.map { _ in Pipe() }
                if let input { process.standardInput = input }

                do {
                    try process.run()
                } catch {
                    continuation.resume(returning: (false, error.localizedDescription))
                    return
                }

                // Written and closed before reading stdout, because the child
                // waits on EOF and we are about to wait on the child.
                //
                // The throwing spellings, not `write(_:)` and `closeFile()`.
                // Those two report failure by raising an Objective-C exception,
                // which Swift cannot catch — so an installer that exited early,
                // leaving nothing on the other end of this pipe, would take the
                // app down while we were writing its password to it. (The
                // signal that used to do the same job is ignored process-wide;
                // see `Entry.ignoreSIGPIPE`.) A child that is already gone is
                // reported by `waitUntilExit` below, in its own words, which is
                // what the caller is here for.
                if let input, let stdin {
                    try? input.fileHandleForWriting.write(contentsOf: Data(stdin.utf8))
                    try? input.fileHandleForWriting.close()
                }

                let stdout = out.fileHandleForReading.readDataToEndOfFile()
                let stderr = err.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()

                let text = [stdout, stderr]
                    .compactMap { String(data: $0, encoding: .utf8) }
                    .joined()
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                continuation.resume(returning: (process.terminationStatus == 0, text))
            }
        }
    }
}
