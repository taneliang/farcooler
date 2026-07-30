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
            onLayout: { [weak self] event in
                Task { @MainActor in
                    self?.layouts[event.workspace] = event.groups.map(\.group)
                }
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

    /// Remove a terminal whose process is gone.
    ///
    /// A terminal IS its process. When that exits — cleanly or not — there is
    /// nothing left to show, so the row goes rather than becoming a dead entry
    /// you have to dismiss. `error` counts too: a terminal that never started
    /// has even less to look at than one that stopped.
    ///
    /// `lost` deliberately does not. That is the one state where Overnight does
    /// not know what happened, and quietly deleting the evidence is the
    /// opposite of what it should do.
    private func reapIfExited(_ terminal: Terminal) {
        guard Preferences.shared.autoRemoveExited else { return }
        let kind = StateKind.parse(terminal.state)
        guard kind == .exited || kind == .error else { return }
        guard !reaped.contains(terminal.id) else { return }
        reaped.insert(terminal.id)

        Task {
            _ = await run(["terminal", "remove", terminal.short], background: true)
            Notifier.shared.forget(terminal.id)
            VisitLog.shared.forget(terminal.id)
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

    /// Everything a task needs, from one sentence.
    ///
    /// Creates the worktree, launches an agent in it, waits for the agent to
    /// actually be ready, and hands it the description as its first message.
    ///
    /// The waiting is the interesting part. An agent takes several seconds to
    /// boot, and text typed into it before then is swallowed by whatever it
    /// draws over the top. Rather than guessing a delay, this waits for the
    /// daemon to report the agent as IDLE — the same activity detection the
    /// sidebar uses. "Ready for input" is exactly what idle means, so the
    /// signal already existed.
    ///
    /// Returns the new workspace, so the caller can select it immediately
    /// rather than after the agent has finished starting.
    // MARK: - Tiling

    /// Each workspace's groups, keyed by workspace id.
    ///
    /// Held here rather than in a view, because the daemon owns it and three
    /// things change it: this app, the CLI, and agents driving the CLI. A layout
    /// in view state would be a fourth opinion.
    @Published var layouts: [String: [PaneGroup]] = [:]

    func activeGroup(_ workspace: String) -> PaneGroup? {
        let groups = layouts[workspace] ?? []
        return groups.first { $0.isActive } ?? groups.first
    }

    /// Which group holds a terminal, if any.
    func group(holding terminal: String, in workspace: String) -> PaneGroup? {
        (layouts[workspace] ?? []).first { $0.terminals.contains(terminal) }
    }

    /// Put terminals in a group of their own, leaving the existing one alone.
    ///
    /// The two-sets-of-tiles operation. `tile` replaces what is on screen; this
    /// adds a second arrangement beside it.
    @discardableResult
    func splitOff(_ terminals: [String], in workspace: Workspace, name: String = "")
        async -> [PaneGroup]
    {
        var rest = terminals
        if !name.isEmpty { rest += ["--name", name] }
        return await layout(workspace, ["split"], rest)
    }

    /// Set a group's members, in this exact order.
    ///
    /// One call for both halves of a drag: reordering panes inside a layout and
    /// pulling an outside terminal into one are the same operation once you have
    /// the list you want, and `tile` takes a list.
    @discardableResult
    func retile(_ ordered: [String], in workspace: Workspace, groupPosition: Int)
        async -> [PaneGroup]
    {
        await layout(workspace, ["tile"], ordered + ["--group", "\(groupPosition)"])
    }

    /// Move a terminal into a group. A terminal is in at most one, so this moves it.
    @discardableResult
    func move(_ terminal: String, toGroup position: Int, in workspace: Workspace)
        async -> [PaneGroup]
    {
        await layout(workspace, ["add"], [terminal, "--group", "\(position)"])
    }

    func refreshLayout(_ workspace: Workspace) async {
        guard let data = await run(["layout", "show", workspace.short, "--json"]) else { return }
        guard let list = try? JSONDecoder().decode(PaneGroupList.self, from: data) else { return }
        layouts[workspace.id] = list.groups
    }

    /// Every layout the fleet has, read once.
    ///
    /// One call per workspace rather than one for the fleet: `layout.list` is
    /// scoped to a workspace, and a fleet-wide read would be a method that exists
    /// only for a first paint. Events carry every change after this.
    func refreshLayouts() async {
        for workspace in fleet.workspaces {
            await refreshLayout(workspace)
        }
    }

    /// Run a layout command and apply the groups it returns.
    ///
    /// `path` is the subcommand, `rest` its arguments; the workspace goes between
    /// them, which is where every one of these commands wants it. Spelled out
    /// rather than inserted at a fixed index — the previous version inserted it at
    /// position 2, which is right for `layout zoom <ws>` and wrong for
    /// `layout group new <ws>`, so every group command silently failed.
    ///
    /// The reply is the workspace's whole layout, so the local copy is replaced
    /// rather than patched — and the event that follows says the same thing,
    /// which is what keeps a second client in step.
    @discardableResult
    func layout(
        _ workspace: Workspace, _ path: [String], _ rest: [String] = []
    ) async -> [PaneGroup] {
        let command = ["layout"] + path + [workspace.short] + rest
        guard let data = await run(command + ["--json"]) else { return layouts[workspace.id] ?? [] }
        guard let list = try? JSONDecoder().decode(PaneGroupList.self, from: data) else {
            return layouts[workspace.id] ?? []
        }
        layouts[workspace.id] = list.groups
        return list.groups
    }

    /// Worktrees on disk that Overnight does not know about yet.
    func existingWorktrees(project: String) async -> [ExistingWorktree] {
        guard let data = await run(["workspace", "discover", project, "--json"]) else { return [] }
        return (try? JSONDecoder().decode(WorktreeList.self, from: data))?.worktrees ?? []
    }

    /// Register worktrees that already exist. Returns how many were taken.
    ///
    /// Not all-or-nothing: importing six where one has been deleted underneath
    /// us should still import five, so the count comes from re-reading rather
    /// than from assuming the request succeeded.
    func importWorktrees(_ worktrees: [ExistingWorktree], project: String) async -> Int {
        guard !worktrees.isEmpty else { return 0 }
        let before = fleet.workspaces.count
        _ = await run(["workspace", "import", project] + worktrees.map(\.path))
        await refresh()
        return max(fleet.workspaces.count - before, 0)
    }

    /// Branches in a project that work could be resumed on.
    func branches(project: String) async -> [BranchInfo] {
        guard let data = await run(["workspace", "branches", project, "--json"]) else { return [] }
        return (try? JSONDecoder().decode(BranchList.self, from: data))?.branches ?? []
    }

    /// Pick up work that already exists on a branch.
    ///
    /// A branch that is only on a remote gets a local tracking branch, which is
    /// the whole point when the work came from another machine or another
    /// person: pushing back has to go where it came from.
    @discardableResult
    func adoptBranch(project: String, branch: String, agent: String) async -> String? {
        let before = Set(fleet.workspaces.map(\.id))
        _ = await run(["workspace", "adopt", project, branch])
        await refresh()

        guard let workspace = fleet.workspaces.first(where: { !before.contains($0.id) })
        else { return nil }
        _ = await run([
            "terminal", "create", workspace.short, "--preset", agent, "--title", "Agent",
        ])
        await refresh()
        return workspace.id
    }

    func startTask(project: String, description: String, agent: String) async -> String? {
        let branch = await MainActor.run { Branch.slug(from: description) }
        let title = await MainActor.run { Branch.title(from: description) }

        let before = Set(fleet.workspaces.map(\.id))
        _ = await run(["workspace", "create", project, title, "--branch", branch])
        await refresh()

        guard let workspace = fleet.workspaces.first(where: { !before.contains($0.id) }) else {
            return nil
        }

        _ = await run([
            "terminal", "create", workspace.short, "--preset", agent, "--title", agent,
        ])
        await refresh()

        guard let terminal = fleet.workspaces
            .first(where: { $0.id == workspace.id })?
            .terminals.first(where: { $0.preset == agent || $0.title == agent })
        else { return workspace.id }

        // Up to a minute: a cold agent on a slow machine is not a failure.
        for _ in 0..<120 {
            try? await Task.sleep(for: .milliseconds(500))
            await refresh()
            let current = fleet.workspaces
                .first(where: { $0.id == workspace.id })?
                .terminals.first(where: { $0.id == terminal.id })
            guard let current else { return workspace.id }
            if current.agent == .idle {
                await send(terminal: current.short, text: description)
                return workspace.id
            }
            // It asked something before we got a word in — a trust prompt, or a
            // resume dialog. Stop rather than typing a task description into a
            // yes/no question.
            if current.agent == .blocked { return workspace.id }
        }
        return workspace.id
    }

    /// Type text into a terminal and press return.
    func send(terminal: String, text: String) async {
        _ = await run(["terminal", "send", terminal, text], background: true)
        // Return as a separate keystroke: an agent's composer treats a newline
        // inside pasted text as a line break, not as submit.
        _ = await run(["terminal", "send-hex", terminal, "0d"], background: true)
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

    /// Create a terminal and return it, identified by difference.
    ///
    /// The create call does not report which record it made, and comparing whole
    /// `Terminal` values does not work — any of them changing activity between the
    /// two reads also looks new. Ids are stable, so the id set is the diff.
    @discardableResult
    func createTerminal(
        in workspace: Workspace, preset: String, title: String, tile: Bool = false
    ) async -> Terminal? {
        let before = Set(workspace.terminals.map(\.id))
        await createTerminal(
            workspace: workspace.short, preset: preset, title: title, tile: tile)

        // Creation is a tmux window opening, so the record can lag the call.
        for _ in 0..<20 {
            if let found = fleet.workspaces.first(where: { $0.id == workspace.id })?
                .terminals.first(where: { !before.contains($0.id) })
            {
                return found
            }
            try? await Task.sleep(for: .milliseconds(150))
            await refresh()
        }
        return nil
    }

    func createTerminal(
        workspace: String, preset: String, title: String, tile: Bool = false
    ) async {
        var command = ["terminal", "create", workspace, "--preset", preset, "--title", title]
        // One call rather than create-then-add: `prefix %` is one keystroke and
        // a pane that appears a beat after the split reads as a stutter.
        if tile { command.append("--tile") }
        _ = await run(command)
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
