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
    ///
    /// It DOES set PATH, once the login shell has been asked what it is. See
    /// `warmSearchPath`: without it every `farcooler` this app spawns pays for
    /// that answer itself, and pane switching spawns several at a time.
    static var environment: [String: String] {
        var environment = ProcessInfo.processInfo.environment
        if let path = searchPath.value { environment["PATH"] = path }
        return environment
    }

    /// Ask the login shell what `PATH` is, once, in the background.
    ///
    /// A double-clicked app inherits launchd's `/usr/bin:/bin:/usr/sbin:/sbin`
    /// and hands exactly that to every CLI it spawns. Homebrew's tmux is not on
    /// it, so `programs::find("tmux")` inside the CLI fell through to asking a
    /// login shell itself — `$SHELL -lc 'printf %s "$PATH"'`, which reads
    /// `.zprofile`, nvm, rbenv and everything else someone has accumulated —
    /// and that answer is cached per PROCESS.
    ///
    /// `terminal stream` and `terminal input` are a fresh process per pane, per
    /// attach. So switching to a four-pane layout paid EIGHT login shells, each
    /// one in front of the first byte of the pane it belonged to, every time.
    /// The daemon pays it once and forgets; the short-lived CLI processes on
    /// the pane-open path pay it every time.
    ///
    /// Asked here instead, once per launch, and handed down. The CLI then finds
    /// tmux on the PATH it inherited — step one of its own search — and spawns
    /// nothing.
    ///
    /// Off the main thread and never waited on. Until it lands, `environment`
    /// yields exactly what it did before, so the worst case is the old
    /// behavior for the first moments of a launch rather than a beachball
    /// during it.
    static func warmSearchPath() {
        guard searchPath.value == nil else { return }
        DispatchQueue.global(qos: .userInitiated).async {
            guard let resolved = loginShellPath() else { return }
            searchPath.value = resolved
        }
    }

    private static let searchPath = Atomic<String?>(nil)

    /// The login shell's `PATH`, with anything this process inherited that the
    /// shell did not mention kept on the end.
    ///
    /// Union rather than replacement: a profile is the authority on where a
    /// user's tools are, so it goes first, but dropping an inherited entry
    /// outright would be this app quietly narrowing the search for a program
    /// it does not know the CLI is about to look for.
    private static func loginShellPath() -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: loginShell())
        // `-l` for the profile, `-c` for the one command, and `printf $PATH`
        // rather than `command -v tmux` so this one spawn answers for every
        // program the CLI is ever asked to find. The same call, spelled the
        // same way, as `programs::login_shell_path` — which is the thing it
        // exists to stop the CLI from making.
        process.arguments = ["-lc", "printf %s \"$PATH\""]
        let out = Pipe()
        process.standardOutput = out
        // A profile that prints a banner or a warning is extremely common and
        // none of it is this function's news.
        process.standardError = FileHandle.nullDevice
        process.standardInput = FileHandle.nullDevice

        guard (try? process.run()) != nil else { return nil }
        let data = out.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return nil }

        let reported = String(decoding: data, as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !reported.isEmpty else { return nil }

        var seen = Set<String>()
        var entries: [String] = []
        for entry in reported.split(separator: ":").map(String.init)
            + (ProcessInfo.processInfo.environment["PATH"] ?? "")
            .split(separator: ":").map(String.init)
        {
            guard !entry.isEmpty, seen.insert(entry).inserted else { continue }
            entries.append(entry)
        }
        return entries.joined(separator: ":")
    }

    /// The shell the account is configured with, read the way the CLI reads it
    /// (`farcooler_core::shell::login_shell`): the passwd entry first, then
    /// `SHELL`. A double-clicked app frequently has no `SHELL` at all, which is
    /// the case that made the passwd read necessary rather than merely tidier.
    private static func loginShell() -> String {
        if let entry = getpwuid(getuid()), let shell = entry.pointee.pw_shell {
            let path = String(cString: shell)
            if !path.isEmpty { return path }
        }
        if let shell = ProcessInfo.processInfo.environment["SHELL"], !shell.isEmpty {
            return shell
        }
        return "/bin/sh"
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

/// One value, readable and writable from any thread.
///
/// `CLI.searchPath` is written by a background resolve and read by every
/// process spawn in the app, which happen on several queues.
final class Atomic<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: Value

    init(_ value: Value) {
        stored = value
    }

    var value: Value {
        get {
            lock.lock()
            defer { lock.unlock() }
            return stored
        }
        set {
            lock.lock()
            stored = newValue
            lock.unlock()
        }
    }
}
