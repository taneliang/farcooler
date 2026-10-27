import AgentKit
import Foundation
import SwiftUI

/// The native view for terminal-mode claude panes, app-wide (ov-372): the
/// one connection per runner it reads through, and each pane's model.
///
/// **No setting** (ov-830): the view is an option on each pane already, so
/// it's offered wherever a runner serves it. A runner's projector opens a
/// pane's rows only while a view asks for them, so offering costs a runner
/// nothing until somebody looks.
///
/// **Every runner, each over its own connection.** This Mac's runner over
/// its socket. A remote runner over the client core's own ssh session
/// (ov-408), signed in with this Mac's conversation key, which
/// `RemotePairing` pairs with the runner the way a phone is paired, once
/// the person pairs it in Settings. The rest of the Mac still reaches a
/// remote runner by the CLI over ssh, a process at a time; reading rows
/// without a process per pane is why this one doesn't.
///
/// Wherever the view can't be offered (the runner without `agent_rows`, a
/// remote runner that isn't paired or couldn't be reached, a pane that isn't
/// claude in a terminal), the terminal shows, and an agent's pane says why.
/// Never a blank pane.
@MainActor
final class NativeAgents: ObservableObject {
    static let shared = NativeAgents()

    /// One runner's connection, as its last hello left it.
    struct Link {
        var core: RunnerCore?
        /// What the runner offered in its last hello.
        var offered: Set<String> = []
        /// Whether it serves rows and compose (`serves`).
        var rowsServed = false
    }

    /// Each runner's connection, by target: empty for this Mac's.
    @Published private var links: [String: Link] = [:]
    /// Whether this Mac's runner serves rows, as its last hello said.
    var rowsServed: Bool { links[""]?.rowsServed ?? false }
    /// This Mac's runner's connection.
    var core: RunnerCore? { links[""]?.core }

    /// The remote runners the fleet brought up, connected where paired.
    @Published private(set) var remotes: Set<String> = []
    private var panes: [String: NativePaneModel] = [:]
    /// Which runner each pane is on.
    private var paneTargets: [String: String] = [:]
    private let defaults: UserDefaults
    private var connecting: [String: Task<Void, Never>] = [:]
    /// The retry waiting out its backoff, while a kept view's runner doesn't answer.
    private var retrying: [String: Task<Void, Never>] = [:]
    /// Reconnects in a row the runner didn't answer.
    private var failures: [String: Int] = [:]
    /// How many reconnects in a row `target`'s runner didn't answer.
    func failedConnects(_ target: String = "") -> Int { failures[target] ?? 0 }
    /// How many of them a view is kept through before it's dropped for the
    /// terminal, and how long the first retry waits (doubling, to a minute).
    /// A test sets these near zero.
    var keepThroughFailures = 4
    var retryDelay: Duration = .seconds(2)

    /// Where this Mac's runner listens.
    var socket: () -> String = { RunnerCore.localSocket() }
    /// How a remote runner is paired and reached.
    let pairing: RemotePairing

    init(defaults: UserDefaults = .standard, pairing: RemotePairing? = nil) {
        self.defaults = defaults
        self.pairing = pairing ?? .shared
        // Photos kept past their age, or past the caps, go once per launch (ov-622).
        DraftImages.shared.prune()
        let cache = PromptImageStore.disk.files
        Task.detached(priority: .utility) { cache.prune() }
    }

    /// Start reading this Mac's runner. Called at launch, and when its
    /// daemon comes back.
    func start() {
        reconnect()
    }

    /// A remote runner the fleet brought up or reconnected to: read it too,
    /// unless it's already connected or on its way. A runner this Mac isn't
    /// paired with answers that it needs pairing, without a key being added
    /// (`RemotePairing.connect`). A link that drops asks again by itself
    /// (`linkLost`).
    func start(target: String) {
        guard !target.isEmpty else { return start() }
        remotes.insert(target)
        guard connecting[target] == nil, links[target]?.rowsServed != true else { return }
        reconnect(target)
    }

    /// A runner removed from the fleet: its connection and panes go.
    func forget(_ target: String) {
        guard !target.isEmpty else { return }
        remotes.remove(target)
        pairing.halt(target)
        connecting[target]?.cancel()
        connecting[target] = nil
        retrying[target]?.cancel()
        retrying[target] = nil
        drop(target)
        links[target] = nil
    }

    /// A fresh connection to `target`'s runner (this Mac's when empty), so
    /// the hello says what the runner offers now.
    func reconnect(_ target: String = "") {
        connecting[target]?.cancel()
        retrying[target]?.cancel()
        retrying[target] = nil
        let socket = target.isEmpty ? socket() : ""
        connecting[target] = Task { [weak self, pairing] in
            let outcome: RemotePairing.Outcome
            if target.isEmpty {
                let core = RunnerCore()
                do {
                    outcome = .connected(core, try await core.connect(socket: socket))
                } catch {
                    outcome = .unreachable
                }
            } else {
                outcome = await pairing.connect(target: target)
            }
            guard let self, !Task.isCancelled else { return }
            self.connecting[target] = nil
            self.settle(target, outcome)
        }
    }

    /// What a connect came to, for `target`'s link and its panes.
    private func settle(_ target: String, _ outcome: RemotePairing.Outcome) {
        let link = links[target] ?? Link()
        switch outcome {
        case .connected(let core, let offered):
            failures[target] = 0
            // Rows to read and a way to send: a runner with rows from before
            // `terminal.compose` gets the terminal, not a view whose every
            // send would fail.
            links[target] = Link(core: core, offered: offered, rowsServed: Self.serves(offered))
            // Panes opened on the old connection follow on the new one.
            if Self.serves(offered) {
                for (terminal, pane) in panes where paneTargets[terminal] == target {
                    follow(pane, on: core, target: target)
                }
            }
        case .unavailable:
            // A person has to do something first (`RemotePairing.State`
            // says what): the terminal, and no retries.
            failures[target] = 0
            drop(target)
        case .unreachable:
            let failed = (failures[target] ?? 0) + 1
            failures[target] = failed
            // The runner didn't answer this once, but it did before: keep
            // what that hello said for a few tries. Tearing the view down for
            // a blink would put the terminal up, with the keyboard, and bring
            // the view back a retry later; the panes' own banner says the
            // runner isn't answering (`AgentRowStore.isStale`). A kept view
            // is retried here, with backoff, rather than waiting for
            // something to call `start()`, and past `keepThroughFailures`
            // it's dropped: a connection nobody can reach shows the terminal.
            if link.rowsServed, link.core != nil, failed <= keepThroughFailures {
                scheduleRetry(target)
                return
            }
            drop(target)
        }
    }

    /// `target`'s view gone: its panes show the terminal.
    private func drop(_ target: String) {
        links[target] = Link()
        for (terminal, pane) in panes where paneTargets[terminal] == target {
            pane.source = nil
            pane.sink = nil
            pane.keys = nil
            pane.drafts = nil
            pane.store.stop()
        }
    }

    /// Try again after a backoff: `retryDelay`, doubling per failure.
    private func scheduleRetry(_ target: String) {
        let failed = max(failures[target] ?? 1, 1)
        let wait = min(retryDelay * (1 << min(failed - 1, 5)), .seconds(60))
        retrying[target] = Task { [weak self] in
            try? await Task.sleep(for: wait)
            guard let self, !Task.isCancelled else { return }
            self.retrying[target] = nil
            self.reconnect(target)
        }
    }

    /// A pane's call found `core`'s link gone: reconnect now, rather than
    /// when the fleet next retries, so a lost runner is said (the stale
    /// banner) and, past its retries, shows the terminal.
    func linkLost(_ target: String, core: RunnerCore) {
        guard links[target]?.core === core, connecting[target] == nil, retrying[target] == nil else { return }
        Task { [weak self] in
            guard await !core.isConnected, let self, self.links[target]?.core === core, self.connecting[target] == nil,
                self.retrying[target] == nil
            else { return }
            self.reconnect(target)
        }
    }

    /// Whether `terminal`, on the runner `target` names, gets the native
    /// view: that runner serving rows over a connection this Mac holds, and
    /// claude running in a terminal-mode pane, or codex where that runner
    /// projects and composes into it (`codex_view`, ov-416).
    func offers(_ terminal: Terminal, target: String) -> Bool {
        guard let link = links[target] else { return false }
        return link.rowsServed && link.core != nil && Self.isAgentInATerminal(terminal, offered: link.offered)
    }

    /// Whether a hello offers the view: rows to read and compose to send.
    static func serves(_ offered: Set<String>) -> Bool {
        offered.contains("agent_rows") && offered.contains("agent_compose")
    }

    static func isAgentInATerminal(_ terminal: Terminal, offered: Set<String>) -> Bool {
        let mode = terminal.paneMode ?? "terminal"
        let program = agent(of: terminal)
        guard mode == "terminal" else { return false }
        return program.hasPrefix("claude") || (program.hasPrefix("codex") && offered.contains("codex_view"))
    }

    /// The agent `terminal` runs, as the view is offered by: the one the
    /// runner sees running, so a claude typed into a shell counts (ov-443).
    static func agent(of terminal: Terminal) -> String {
        AgentConversation.agent(running: terminal.runningAgent, program: terminal.program, preset: terminal.preset)
    }

    /// Why `terminal` isn't offered the view, nil where it is (ov-443): what
    /// the pane's switch and the menu item say.
    func unavailable(_ terminal: Terminal, target: String) -> AgentConversation.Unavailable? {
        let link = links[target]
        let offered = link?.core == nil ? nil : link?.offered
        if let pane = AgentConversation.unavailable(paneMode: terminal.paneMode, agent: Self.agent(of: terminal)) { return pane }
        if !target.isEmpty, link?.core == nil {
            switch pairing.states[target] {
            case .removed?, .unpaired?: return .pairingNeeded
            case .unavailable(let words)?: return .said(words)
            case .paired?: break
            case nil where pairing.waitsForConsent(target): return .pairingNeeded
            case nil: break
            }
        }
        if offered == nil, connecting[target] != nil || retrying[target] != nil { return .said(Self.connectingSentence) }
        return AgentConversation.unavailable(paneMode: terminal.paneMode, agent: Self.agent(of: terminal), offered: offered)
    }

    static let connectingSentence = "Far Cooler is connecting to this runner."

    /// The pane's model, made once and kept, following its runner's rows.
    /// `program` names the agent, for its words and its keys.
    func model(for terminal: String, target: String = "", program: String? = nil) -> NativePaneModel {
        if let model = panes[terminal] {
            if let program { model.program = program }
            return model
        }
        let store = AgentRowStore(key: target.isEmpty ? "local-\(terminal)" : "remote-\(terminal)")
        let core = links[target]?.core
        let model = NativePaneModel(terminal: terminal, store: store, sink: core, draftKeeper: DraftKeeper(terminal: terminal, defaults: defaults), draftImages: .shared)
        if let program { model.program = program }
        model.viewer = { target.isEmpty ? "" : FleetStore.shared.clients[target]?.talk.viewer }
        model.refreshViewer()
        // `TalkRoster.load` tells the client before it changes `people`, so look on the next turn.
        if !target.isEmpty {
            model.viewerWatch = FleetStore.shared.clients[target]?.objectWillChange.sink { [weak model] _ in
                Task { @MainActor in model?.refreshViewer() }
            }
        }
        panes[terminal] = model
        paneTargets[terminal] = target
        if let core { follow(model, on: core, target: target) }
        return model
    }

    /// Give `model` this connection. It follows only while its view shows
    /// (`NativePaneModel.showsNative`): a pane on its terminal holds no
    /// follow on the runner.
    private func follow(_ model: NativePaneModel, on core: RunnerCore, target: String) {
        let offered = links[target]?.offered ?? []
        model.source = CoreRowSource(core: core, terminal: model.terminal, opensAgents: offered.contains(Capability.subagentRows.rawValue)) {
            [weak self] in
            Task { @MainActor in self?.linkLost(target, core: core) }
        }
        model.sink = core
        // Line breaks, images and commands where the runner takes them.
        model.rich = offered.contains(Capability.compose.rawValue)
        // Any file, not only images, where the runner takes them (ov-454).
        model.takesFiles = offered.contains(Capability.composeFiles.rawValue)
        // A prompt's images as thumbnails, where the runner serves them (ov-454).
        model.promptImages.source = offered.contains(Capability.agentImages.rawValue) ? core : nil
        // Stop and Send Now where the runner presses them (ov-368).
        model.keys = offered.contains(Capability.terminalInterrupt.rawValue) ? core : nil
        // Bring Here where the runner reads and clears claude's box (ov-369).
        model.drafts = offered.contains(Capability.bringDraft.rawValue) ? core : nil
        // A held ask's buttons (ov-370). A runner from before them holds no
        // ask on a row, so none is offered there.
        model.answers = core
        model.followIfShown()
    }

    /// The pane's model if one was made, without making it.
    func model(ifMade terminal: String) -> NativePaneModel? { panes[terminal] }

    /// The Switch Between Terminal and Conversation command: flip `terminal`'s
    /// view, where the conversation is offered. False, changing nothing,
    /// where it isn't.
    @discardableResult
    func toggleView(of terminal: Terminal, target: String) -> Bool {
        guard offers(terminal, target: target) else { return false }

        let model = model(for: terminal.id, target: target, program: Self.agent(of: terminal))
        model.showsNative.toggle()
        return true
    }

    /// Settings' Unpair: this Mac's key off `target`'s runner, and its panes
    /// back to the terminal. Nil when done; a sentence when not.
    func unpair(_ target: String) async -> String? {
        if let trouble = await pairing.unpair(target: target) { return trouble }
        connecting[target]?.cancel()
        connecting[target] = nil
        retrying[target]?.cancel()
        retrying[target] = nil
        drop(target)
        return nil
    }

    /// Settings' Try Again: connect to `target`'s runner again. Never pairs
    /// a runner that was unpaired or whose key was removed; only Pair Again
    /// does.
    func retry(_ target: String) {
        reconnect(target)
    }

    /// Settings' Pair and Pair Again: pair `target`'s runner on the next
    /// connect, now.
    func pairAgain(_ target: String) {
        pairing.allowPairing(target: target)
        reconnect(target)
    }

    /// A model a test made, under the registry's rules.
    func adopt(_ model: NativePaneModel, target: String = "") {
        panes[model.terminal] = model
        paneTargets[model.terminal] = target
    }

    /// For tests: `target`'s runner as if connected, against `core`.
    func pretend(rowsServed: Bool, core: RunnerCore?, offered: Set<String> = [], target: String = "") {
        links[target] = Link(core: core, offered: offered, rowsServed: rowsServed)
    }
}
