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
    var cliPath: String? { binary }

    var cliEnvironment: [String: String] { environment }

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

    // MARK: - Live updates

    private var eventStream: EventStream?
    /// Terminals whose clean exit we have already acted on, so a burst of
    /// events for the same one does not queue several removals.
    private var reaped: Set<String> = []

    /// Start receiving pushed changes.
    ///
    /// Replaces polling. A poll has to choose between noticing an agent's
    /// question late and burning cycles on a fleet where nothing is happening;
    /// pushed changes have neither problem, and a quiet host sends nothing.
    func startEvents() {
        guard eventStream == nil, let binary else { return }
        let stream = EventStream(
            onEvent: { [weak self] event in
                Task { @MainActor in self?.apply(event) }
            },
            onEnd: { [weak self] in
                Task { @MainActor in
                    self?.eventStream = nil
                    // The daemon restarted, or the CLI died. Reconnect after a
                    // pause rather than spinning, and re-read once connected:
                    // anything that changed while we were deaf is only visible
                    // in a full read.
                    try? await Task.sleep(for: .seconds(2))
                    await self?.refresh()
                    self?.startEvents()
                }
            })
        stream.start(binary: binary, environment: environment)
        eventStream = stream
    }

    func stopEvents() {
        eventStream?.stop()
        eventStream = nil
    }

    /// Fold one pushed change into the fleet.
    ///
    /// Applied in place rather than triggering a full re-read: a re-read per
    /// event would make a busy fleet slower than the polling this replaced.
    private func apply(_ event: TerminalEvent) {
        for w in fleet.workspaces.indices {
            guard
                let t = fleet.workspaces[w].terminals.firstIndex(where: { $0.id == event.id })
            else { continue }

            fleet.workspaces[w].terminals[t].state = event.state
            fleet.workspaces[w].terminals[t].activity = event.activity

            let terminal = fleet.workspaces[w].terminals[t]
            Notifier.shared.report(terminal: terminal, workspace: fleet.workspaces[w].task)
            reapIfExited(terminal)
            return
        }

        // A terminal we have never seen: created elsewhere, or created here
        // before the first read finished. Only a full read can place it in a
        // workspace, so ask for one.
        Task { await refresh() }
    }

    /// Remove a terminal whose command exited cleanly.
    ///
    /// A terminal you closed should leave nothing behind. A terminal that
    /// FAILED is never removed automatically, whatever the preference says:
    /// a crashed agent is the one row in the list you needed to see, and
    /// tidying it away would hide the only evidence that something went wrong.
    private func reapIfExited(_ terminal: Terminal) {
        guard Preferences.shared.autoRemoveExited else { return }
        guard StateKind.parse(terminal.state) == .exited else { return }
        guard !reaped.contains(terminal.id) else { return }
        // The daemon reports a non-zero exit as `error`, not `exited`, so
        // reaching here already means a clean one.
        reaped.insert(terminal.id)

        Task {
            _ = await run(["terminal", "remove", terminal.short], background: true)
            Notifier.shared.forget(terminal.id)
            await refresh()
        }
    }

    // MARK: - Commands

    /// Has a fleet ever been read successfully?
    ///
    /// Without this, "we could not read the fleet" and "there are no
    /// workspaces" look identical to the UI, because a failed read leaves the
    /// last value in place — and the first value is empty. A user who had just
    /// created a workspace was shown the new-user empty state, which is the
    /// most misleading thing the app could have said.
    @Published private(set) var hasLoaded = false

    func refresh() async {
        guard let data = await run(["workspace", "list", "--json"], background: true) else { return }
        do {
            fleet = try JSONDecoder().decode(Fleet.self, from: data)
            hasLoaded = true
            lastError = nil
            // Reap on every read, not only on events. A terminal that exited
            // while the app was closed produces no event to react to, so
            // without this the first thing you see on launch is exactly the
            // clutter auto-removal exists to prevent.
            for workspace in fleet.workspaces {
                for terminal in workspace.terminals { reapIfExited(terminal) }
            }
        } catch {
            // Show the daemon's own output, truncated. A decode failure is
            // almost always something unexpected on stdout, and the first line
            // of it says what.
            let sample = String(data: data.prefix(200), encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            lastError = sample.isEmpty
                ? "Could not read the fleet: \(error.localizedDescription)"
                : "Could not read the fleet. The CLI said: \(sample)"
        }
    }

    @Published var repositories: [Repository] = []

    func refreshRepositories() async {
        guard let data = await run(["repo", "list", "--json"], background: true) else { return }
        repositories = (try? JSONDecoder().decode(RepositoryList.self, from: data))?.repositories ?? []
    }

    /// Allowlisted roots, so the app can tell whether a chosen repository is
    /// already covered by one.
    @Published var roots: [RepositoryRoot] = []

    func refreshRoots() async {
        guard let data = await run(["root", "list", "--json"], background: true) else { return }
        roots = (try? JSONDecoder().decode(RootList.self, from: data))?.roots ?? []
    }

    /// Allowlist a directory. Returns the daemon's message, or nil on success.
    func addRoot(_ path: String) async -> String? {
        await runReportingError(["root", "add", path])
    }

    /// Register a git repository that sits inside an allowlisted root.
    func registerRepository(_ path: String) async -> String? {
        let error = await runReportingError(["repo", "register", path])
        await refreshRepositories()
        return error
    }

    func createWorkspace(repo: String, task: String, branch: String, base: String) async {
        _ = await run(["workspace", "create", repo, task, "--branch", branch, "--base", base])
        await refresh()
    }

    func archiveWorkspace(_ workspace: String) async {
        _ = await run(["workspace", "archive", workspace])
        await refresh()
    }

    /// Remove a worktree. `confirm` must be the workspace's exact task name.
    ///
    /// Forwarded rather than checked only here: the daemon refuses a mismatch
    /// itself, so the dialog is a courtesy and the daemon's check is the one
    /// that actually protects the files.
    func removeWorktree(_ workspace: String, confirm: String) async {
        _ = await run(["workspace", "remove-worktree", workspace, "--confirm", confirm])
        await refresh()
    }

    /// The rendered visible screen, colour escapes intact.
    func screen(terminal: String) async -> String {
        guard let data = await run(["terminal", "screen", terminal, "--json"]) else { return "" }
        struct Screen: Decodable { var screen: String }
        return (try? JSONDecoder().decode(Screen.self, from: data))?.screen ?? ""
    }

    func capture(terminal: String, lines: Int = 400) async -> String {
        guard let data = await run(["terminal", "read", terminal, "--lines", "\(lines)"])
        else { return "" }
        return String(data: data, encoding: .utf8) ?? ""
    }

    func resize(terminal: String, columns: Int, rows: Int) async {
        _ = await run(["terminal", "resize", terminal, "\(columns)", "\(rows)"])
    }

    func createTerminal(workspace: String, preset: String, title: String) async {
        _ = await run(["terminal", "create", workspace, "--preset", preset, "--title", title])
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

    /// Tell the daemon a terminal was opened.
    ///
    /// This is what ends `done`, which is defined as finished-and-unseen.
    /// Called on selection, not on appearing in a list: being listed is not
    /// being read, and clearing a notification nobody read is worse than not
    /// sending one.
    func markSeen(_ terminal: String) async {
        _ = await run(["terminal", "seen", terminal], background: true)
    }

    /// Delete a terminal's record. Refused by the daemon while it is running.
    func removeTerminal(_ terminal: String) async {
        _ = await run(["terminal", "remove", terminal])
        await refresh()
    }

    func stop(terminal: String) async {
        _ = await run(["terminal", "stop", terminal])
        await refresh()
    }

    // MARK: - Subprocess

    @discardableResult
    private func run(_ args: [String], background: Bool = false) async -> Data? {
        // A background poll must not toggle `busy`. That is a @Published change,
        // and every one of them re-evaluates the whole view tree including the
        // terminal surface, which is wasted work several times a second.
        if !background { busy = true }
        defer { if !background { busy = false } }

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

    /// Run a command and hand back its failure instead of only banner-ing it.
    ///
    /// A sheet needs to show the reason next to the field that caused it and
    /// stay open so the user can fix it. `lastError` alone would put the
    /// message in the window behind the sheet, where nobody is looking.
    private func runReportingError(_ args: [String]) async -> String? {
        let before = lastError
        let output = await run(args)
        if output == nil {
            let message = lastError ?? "command failed"
            // Leave the banner clean: the sheet is showing this one.
            lastError = before
            return message
        }
        return nil
    }
}
