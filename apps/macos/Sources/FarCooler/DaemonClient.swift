import Foundation

/// Talks to the daemon through the `farcooler` CLI.
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

    /// What to put in front of every CLI invocation to aim it at another
    /// machine. Empty when this window is driving the machine it is running on.
    ///
    /// Before the subcommand, not after: `--host` is a top-level option, and
    /// clap will not see it once a subcommand has been named.
    var cliHostArguments: [String] {
        let target = Preferences.shared.remoteHost.trimmingCharacters(in: .whitespaces)
        return target.isEmpty ? [] : ["--host", target]
    }

    private var binary: String? { CLI.binary }

    private var environment: [String: String] { CLI.environment }

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
                    self?.layouts[event.workspace] = event.groups
                }
            },
            onFleet: { [weak self] in
                Task { @MainActor in await self?.refresh() }
            },
            onEnd: { [weak self] in
                Task { @MainActor in
                    self?.eventStream = nil
                    // The daemon restarted, or the CLI died. Reconnect after a
                    // pause rather than spinning, and re-read once connected:
                    // anything that changed while we were deaf is only visible
                    // in a full read.
                    try? await Task.sleep(for: .seconds(2))
                    // A daemon going away is also the moment another build
                    // could take the socket, so the app claims it back before
                    // reading anything through it. See `LocalDaemon`.
                    await LocalDaemon.shared.ensure()
                    await self?.refresh()
                    self?.startEvents()
                }
            })
        stream.start(binary: binary, environment: environment, host: cliHostArguments)
        eventStream = stream
    }

    func stopEvents() {
        eventStream?.stop()
        eventStream = nil
    }

    /// Point every live connection at whichever machine `remoteHost` now names.
    ///
    /// The subprocess-per-command calls already read the preference fresh on
    /// every call, but the event stream is a long-lived process started once —
    /// changing the preference under it would leave it listening to the machine
    /// it was pointed at when it started. Only the stream needs restarting, but
    /// the fleet it was populating belongs to the old machine too, so that goes
    /// back to empty rather than showing stale rows until the next event lands.
    func hostChanged() async {
        stopEvents()
        fleet = .empty
        await refresh()
        startEvents()
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
            // What is RUNNING, which is also what the terminal is CALLED.
            //
            // This was missed, and the omission was invisible until the name
            // started being derived from it: the daemon broadcasts the moment a
            // pane's command changes, the app applied the state and the activity
            // out of that event and dropped the command — so a shell you had just
            // run `node` in stayed labelled `shell` until something forced a full
            // re-read. The whole point of pushing events is not needing one.
            fleet.workspaces[w].terminals[t].preset = event.preset
            // What can be switched to a chat, which is also what `⌃B a`
            // checks before it will even try.
            //
            // This was missed the same way `preset` was missed above, and
            // the omission was the branch's own headline bug surviving its
            // own fix: the daemon pushes `chatCapable` the instant a shell
            // pane's foreground process becomes `codex`, but until this line
            // existed the app applied everything else out of that event and
            // dropped this field — so `canSwitchPaneMode` stayed false
            // forever, since this app is push-only and never re-fetches a
            // terminal it already knows.
            fleet.workspaces[w].terminals[t].chatCapable = event.chatCapable

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
    /// `lost` deliberately does not. That is the one state where Far Cooler does
    /// not know what happened, and quietly deleting the evidence is the
    /// opposite of what it should do.
    /// Terminals already offered the chat, so the offer is made once each.
    private var openedAsChat: Set<String> = []

    /// Open a detected agent as a chat, if that is what the user prefers.
    ///
    /// Once per terminal, tracked by id: a user who switches straight back to
    /// the terminal must not be dragged into the chat again on the next refresh
    /// two hundred milliseconds later. The preference sets a default, and a
    /// default that cannot be overruled is a policy.
    private func openAsChatIfPreferred(_ terminal: Terminal) {
        guard Preferences.shared.preferChatMode else { return }
        guard terminal.canSwitchPaneMode, !terminal.isAgentPane else { return }
        guard !openedAsChat.contains(terminal.id) else { return }
        openedAsChat.insert(terminal.id)

        Task {
            _ = await setPaneMode(terminal.short, mode: "agent")
            await refresh()
        }
    }

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
                for terminal in workspace.terminals {
                    reapIfExited(terminal)
                    openAsChatIfPreferred(terminal)
                }
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

    // MARK: - Layout commands
    //
    // One method per CLI subcommand, and nothing more. Each is a single line over
    // `layout(_:_:_:)`, which exists so the reply — always the workspace's whole
    // layout — is applied in exactly one place. The value of naming them anyway is
    // that the argument order and the flag spellings live here rather than being
    // written out at each call site, which is where the last set of them drifted.

    /// A new terminal beside an existing pane. tmux's `split-window`.
    @discardableResult
    func split(
        _ workspace: Workspace, beside terminal: String?, side: TileDirection,
        preset: String = "shell"
    ) async -> [PaneGroup] {
        var rest = terminal.map { [$0] } ?? []
        rest += ["--side", side.rawValue, "--preset", preset]
        return await layout(workspace, ["split"], rest)
    }

    /// Move a pane against another, on an edge. The drag and drop.
    ///
    /// Works across layouts — the pane leaves the one it was in — which is why
    /// this single call covers both halves of the gesture: rearranging panes
    /// within a layout and pulling a terminal in from another one.
    @discardableResult
    func movePane(
        _ terminal: String, onto target: String, side: TileDirection, in workspace: Workspace
    ) async -> [PaneGroup] {
        await layout(workspace, ["move"], [terminal, target, "--side", side.rawValue])
    }

    @discardableResult
    func applyPreset(_ preset: TilePreset, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["preset"], [preset.rawValue])
    }

    @discardableResult
    func cycleLayout(_ workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["cycle"])
    }

    /// Focus a pane, which also brings its layout to the front.
    @discardableResult
    func focusPane(_ terminal: String, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["focus"], [terminal])
    }

    @discardableResult
    func focusPane(step: String, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["focus"], [step])
    }

    @discardableResult
    func focusPane(number: Int, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["focus"], ["--pane", "\(number)"])
    }

    @discardableResult
    func zoomPane(_ terminal: String?, in workspace: Workspace, off: Bool = false)
        async -> [PaneGroup]
    {
        await layout(workspace, ["zoom"], (terminal.map { [$0] } ?? []) + (off ? ["--off"] : []))
    }

    @discardableResult
    func swapPanes(_ a: String, _ b: String, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["swap"], [a, b])
    }

    @discardableResult
    func resizePane(
        _ terminal: String, side: TileDirection, cells: Int, in workspace: Workspace
    ) async -> [PaneGroup] {
        await layout(
            workspace, ["resize"], [terminal, "--side", side.rawValue, "--cells", "\(cells)"])
    }

    /// Pull a pane into a layout of its own. tmux's `break-pane`.
    @discardableResult
    func breakPane(_ terminal: String?, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["break"], terminal.map { [$0] } ?? [])
    }

    @discardableResult
    func renameLayout(_ name: String, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["rename"], [name])
    }

    /// Tell tmux how big the view showing this layout is, in cells.
    ///
    /// The one thing the app is authoritative about, because it is the only thing
    /// tmux cannot see: how much screen there is. Everything else flows back the
    /// other way — tmux lays out into this and reports where the panes landed.
    @discardableResult
    func viewport(columns: Int, rows: Int, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["viewport"], ["\(columns)", "\(rows)"])
    }

    /// Show a different layout: by tmux window id, by name, by number, or `--next`.
    @discardableResult
    func selectLayout(_ group: String, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["select"], [group])
    }

    func refreshLayout(_ workspace: Workspace) async {
        guard let data = await run(["layout", "show", workspace.short, "--json"]) else { return }
        guard let list = try? JSONDecoder().decode(PaneGroupList.self, from: data) else { return }
        layouts[workspace.id] = list.groups
    }

    /// Every layout the fleet has, read once.
    ///
    /// One call per workspace rather than one for the fleet: `layout show` is
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
    /// rather than inserted at a fixed index — a previous version inserted it at
    /// position 2, which is right for a one-word subcommand and wrong for a
    /// two-word one, so half the commands silently acted on the wrong thing.
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

    func hideWorkspace(_ workspace: String) async {
        _ = await run(["workspace", "hide", workspace])
        await refresh()
    }

    func unhideWorkspace(_ workspace: String) async {
        _ = await run(["workspace", "unhide", workspace])
        await refresh()
    }

    /// What asking the daemon to remove a worktree came back with.
    enum RemoveWorktreeResult {
        case ok
        /// The daemon needs the typed workspace name because the worktree is
        /// dirty (`DomainError::ConfirmationRequired`). Carries no message:
        /// the sheet has fixed wording for this one specific refusal.
        case confirmationRequired
        /// Refused for any other reason — running terminals, tmux
        /// unavailable, a failed `git worktree remove` — carrying the
        /// daemon's own message so the sheet can show what actually went
        /// wrong instead of guessing "uncommitted work".
        case failed(String)
    }

    /// Remove a worktree. `confirm` must be the workspace's exact task name,
    /// unless the worktree is clean (or its directory is already gone), in
    /// which case it may be empty and is omitted entirely — Task 8 made
    /// `--confirm` optional on the CLI side for exactly this.
    ///
    /// Forwarded rather than checked only here: the daemon refuses a mismatch
    /// itself, so the dialog is a courtesy and the daemon's check is the one
    /// that actually protects the files.
    ///
    /// Distinguishes "confirmation required" from every other refusal the
    /// same way `setPaneMode` distinguishes its own: a non-zero exit alone
    /// does not mean the worktree is dirty, and reporting every refusal —
    /// `RunningProcesses`, `TmuxUnavailable`, a failed `git worktree remove`
    /// — as "there is uncommitted work here" tells the user to type a name
    /// that will never make the real problem go away.
    @discardableResult
    func removeWorktree(_ workspace: String, confirm: String) async -> RemoveWorktreeResult {
        var args = ["workspace", "remove-worktree", workspace]
        if !confirm.isEmpty { args += ["--confirm", confirm] }

        let before = lastError
        guard await run(args) != nil else {
            let message = lastError ?? "command failed"
            if message.localizedCaseInsensitiveContains("confirmation") {
                // Leave the banner clean: this refusal becomes the sheet's
                // own field and callout, not a banner behind it.
                lastError = before
                return .confirmationRequired
            }
            return .failed(message)
        }
        await refresh()
        return .ok
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

    /// Size one terminal to a viewer's grid.
    ///
    /// Only for a terminal being shown ALONE. A pane's size is a property of the
    /// layout it is in, so calling this on one pane of several resizes the whole
    /// window to that pane's grid and the other panes shrink to fit inside it —
    /// which is what happened, once, when each pane reported its own geometry.
    /// The tiled view calls `viewport` instead, exactly once for the whole view.
    func resize(terminal: String, columns: Int, rows: Int) async {
        _ = await run(["terminal", "resize", terminal, "\(columns)", "\(rows)"])
    }

    /// Files in a worktree matching a partial path, for the composer's `@`
    /// picker.
    ///
    /// DEVIATION, recorded the same way `AgentStream` records its own:
    /// `crates/cli` has no `workspace file-search` subcommand yet, so this is
    /// written against the shape the feature needs — a query in, matching
    /// paths out — and is inert until that subcommand lands. Empty on any
    /// failure rather than surfacing `lastError`: a mention picker that
    /// cannot search is a picker with nothing to show, not a reason to put a
    /// banner over someone's half-typed message.
    func searchFiles(in workspace: Workspace, query: String) async -> [String] {
        guard !query.isEmpty else { return [] }
        guard
            let data = await run([
                "workspace", "file-search", workspace.short, query, "--json",
            ])
        else { return [] }
        struct Result: Decodable { var paths: [String] }
        return (try? JSONDecoder().decode(Result.self, from: data))?.paths ?? []
    }

    /// What asking the daemon to switch a pane's mode came back with.
    enum PaneModeResult {
        case ok
        /// A turn is in flight; switching would cancel it. Carries the
        /// daemon's own description of what that turn is, so the confirmation
        /// shown for it says something true rather than a generic warning.
        case confirmationRequired(String)
        case failed(String)
    }

    /// Switch a pane between showing its terminal and showing its agent chat.
    ///
    /// DEVIATION, same shape as `searchFiles` above: `crates/cli` has no
    /// `terminal set-pane-mode` subcommand yet. This is written against the
    /// daemon's own contract for it — a plain success, or a refusal naming
    /// what is in flight — so the confirmation flow above it (`ContentView`)
    /// can be built and reviewed now rather than after the CLI catches up.
    /// The exact wording the daemon will use for a refusal is not settled
    /// either, so the detection here is a case-insensitive substring match
    /// against "confirmation" — a best guess against an undefined wire
    /// format, not a parsed error code, and worth tightening once the real
    /// shape exists.
    @discardableResult
    func setPaneMode(_ terminal: String, mode: String, force: Bool = false) async -> PaneModeResult {
        var args = ["terminal", "set-pane-mode", terminal, mode]
        if force { args.append("--force") }

        let before = lastError
        guard await run(args) != nil else {
            let message = lastError ?? "command failed"
            if message.localizedCaseInsensitiveContains("confirmation") {
                // Leave the banner clean: this refusal becomes a sheet, not a
                // banner, in `ContentView`.
                lastError = before
                return .confirmationRequired(message)
            }
            return .failed(message)
        }
        await refresh()
        return .ok
    }

    /// Create a terminal and return it, identified by difference.
    ///
    /// The create call does not report which record it made, and comparing whole
    /// `Terminal` values does not work — any of them changing activity between the
    /// two reads also looks new. Ids are stable, so the id set is the diff.
    @discardableResult
    func createTerminal(
        in workspace: Workspace, preset: String, title: String
    ) async -> Terminal? {
        let before = Set(workspace.terminals.map(\.id))
        await createTerminal(workspace: workspace.short, preset: preset, title: title)

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

    /// A terminal, which is a tmux window, which is a layout of its own.
    ///
    /// Deliberately NOT `--tile`. A new terminal gets its own layout tab;
    /// putting a pane into an existing arrangement is what `layout split` and
    /// `⌃B %` are for, and they say so at the point of use. Tiling here made
    /// every new terminal a split of whatever happened to be focused, which
    /// also hid the tab strip — with one layout there are no tabs to show —
    /// so the pane appeared to arrive nowhere at all.
    func createTerminal(workspace: String, preset: String, title: String) async {
        _ = await run(["terminal", "create", workspace, "--preset", preset, "--title", title])
        await refresh()
    }

    func restart(terminal: String) async {
        _ = await run(["terminal", "restart", terminal])
        await refresh()
    }

    /// Be rid of a terminal whose pane cannot be found.
    ///
    /// The daemon deletes the record — a lost terminal has no pane, no output
    /// and no exit code, so once it has been acknowledged there is nothing left
    /// for the row to say. Its notification and its place in the switcher go
    /// with it: both point at something that no longer exists.
    func dismissLost(_ terminal: Terminal) async {
        _ = await run(["terminal", "dismiss-lost", terminal.short])
        Notifier.shared.forget(terminal.id)
        VisitLog.shared.forget(terminal.id)
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
                "The farcooler CLI was not found. Rebuild the app with "
                + "apps/macos/build-app.sh so it bundles one, or set FARCOOLER_BIN."
            return nil
        }

        let env = environment
        let host = cliHostArguments
        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: bin)
                process.arguments = host + args
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
