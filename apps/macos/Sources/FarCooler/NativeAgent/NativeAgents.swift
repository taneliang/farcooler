import AgentKit
import Foundation
import SwiftUI

/// The native view for terminal-mode claude panes, app-wide (ov-372): the
/// setting that gates it, the one connection per runner it reads through,
/// and each pane's model.
///
/// **Off by default**, so it can ship before it matches the terminal. Turning
/// it on turns the runner's projector on (`farcooler settings set-projector
/// on`, which writes `[agents] projector` to the runner's config.toml and
/// takes effect at once), then reconnects, since a hello says what a runner
/// offers only when it's made.
///
/// **Every runner, each over its own connection.** This Mac's runner over
/// its socket. A remote runner over the client core's own ssh session
/// (ov-408), signed in with this Mac's conversation key, which
/// `RemotePairing` pairs with the runner the way a phone is paired. The rest
/// of the Mac still reaches a remote runner by the CLI over ssh, a process at
/// a time; reading rows without a process per pane is why this one doesn't.
///
/// Wherever the view can't be offered (the setting off, the runner without
/// `agent_rows`, a remote runner that couldn't be paired or reached, a pane
/// that isn't claude in a terminal), the switch is hidden and the terminal
/// shows. Never a blank pane.
@MainActor
final class NativeAgents: ObservableObject {
    static let shared = NativeAgents()

    /// The user's setting.
    @Published private(set) var enabled: Bool
    /// What went wrong turning the setting on or off, or reaching this Mac's
    /// runner, for Settings to show. A remote runner's is its pairing state.
    @Published private(set) var settingTrouble: String?
    /// A change to the setting is on its way to the runner: the switch waits
    /// for it, so two flips can't land out of order.
    @Published private(set) var changing = false

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

    /// The remote runners the fleet brought up, connected while the setting
    /// is on.
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
    /// How many of them a view is kept through before it's dropped for the
    /// terminal, and how long the first retry waits (doubling, to a minute).
    /// A test sets these near zero.
    var keepThroughFailures = 4
    var retryDelay: Duration = .seconds(2)

    /// How the setting reaches the runner. The real CLI in the app; a test
    /// passes its own.
    var setProjector: (Bool) async -> String? = { on in
        let ran = await CLI.run(["settings", "set-projector", on ? "on" : "off"])
        // The CLI's own words are for a terminal; this is the sentence for
        // a settings window. The usual cause is a runner from before ov-372.
        return ran.ok ? nil : "This Mac’s runner didn’t take the setting. Update Far Cooler and try again."
    }
    /// Where this Mac's runner listens.
    var socket: () -> String = { RunnerCore.localSocket() }
    /// How a remote runner is paired and reached.
    let pairing: RemotePairing

    static let settingKey = "nativeAgent.enabled"

    init(defaults: UserDefaults = .standard, pairing: RemotePairing? = nil) {
        self.defaults = defaults
        self.pairing = pairing ?? .shared
        enabled = defaults.bool(forKey: Self.settingKey)
    }

    /// Start reading this Mac's runner, when the setting is on. Called at
    /// launch, and when its daemon comes back.
    func start() {
        guard enabled else { return }
        reconnect()
    }

    /// A remote runner the fleet brought up or reconnected to: read it too,
    /// when the setting is on, unless it's already connected or on its way.
    /// A link that drops asks again by itself (`linkLost`).
    func start(target: String) {
        guard !target.isEmpty else { return start() }
        remotes.insert(target)
        guard enabled, connecting[target] == nil, links[target]?.rowsServed != true else { return }
        reconnect(target)
    }

    /// A runner removed from the fleet: its connection and panes go.
    func forget(_ target: String) {
        guard !target.isEmpty else { return }
        remotes.remove(target)
        connecting[target]?.cancel()
        connecting[target] = nil
        retrying[target]?.cancel()
        retrying[target] = nil
        drop(target)
        links[target] = nil
    }

    /// The setting's switch.
    func setEnabled(_ on: Bool) async {
        guard !changing else { return }
        changing = true
        defer { changing = false }
        settingTrouble = nil
        if let trouble = await setProjector(on) {
            settingTrouble = trouble
            return
        }
        enabled = on
        defaults.set(on, forKey: Self.settingKey)
        if on {
            for remote in remotes { reconnect(remote) }
            reconnect()
            await connecting[""]?.value
        } else {
            // A reconnect still on its way would otherwise set a core after.
            for task in connecting.values { task.cancel() }
            for task in retrying.values { task.cancel() }
            connecting = [:]
            retrying = [:]
            failures = [:]
            for pane in panes.values { pane.store.stop() }
            panes = [:]
            paneTargets = [:]
            links = [:]
        }
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
            if target.isEmpty { settingTrouble = nil }
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
            if target.isEmpty {
                settingTrouble = "Far Cooler can’t reach this Mac’s runner, so Claude panes show the terminal."
            }
            // The runner didn't answer this once, but it did before: keep
            // what that hello said for a few tries. Tearing the view down for
            // a blink would put the terminal up, with the keyboard, and bring
            // the view back a retry later; the panes' own banner says the
            // runner isn't answering (`AgentRowStore.isStale`). A kept view
            // is retried here, with backoff, rather than waiting for
            // something to call `start()`, and past `keepThroughFailures`
            // it's dropped: a connection nobody can reach shows the terminal.
            if link.rowsServed, link.core != nil, failed <= keepThroughFailures {
                if target.isEmpty {
                    settingTrouble = "Far Cooler can’t reach this Mac’s runner right now. The conversation view may be out of date until it answers."
                }
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
        guard enabled, links[target]?.core === core, connecting[target] == nil, retrying[target] == nil else { return }
        Task { [weak self] in
            guard await !core.isConnected, let self, self.links[target]?.core === core, self.connecting[target] == nil,
                self.retrying[target] == nil
            else { return }
            self.reconnect(target)
        }
    }

    /// Whether `terminal`, on the runner `target` names, gets the native
    /// view: the setting on, that runner serving rows over a connection this
    /// Mac holds, and claude running in a terminal-mode pane.
    func offers(_ terminal: Terminal, target: String) -> Bool {
        guard enabled, let link = links[target] else { return false }
        return link.rowsServed && link.core != nil && Self.isClaudeInATerminal(terminal)
    }

    /// Whether a hello offers the view: rows to read and compose to send.
    static func serves(_ offered: Set<String>) -> Bool {
        offered.contains("agent_rows") && offered.contains("agent_compose")
    }

    static func isClaudeInATerminal(_ terminal: Terminal) -> Bool {
        let mode = terminal.paneMode ?? "terminal"
        return mode == "terminal" && (terminal.program ?? terminal.preset).hasPrefix("claude")
    }

    /// The pane's model, made once and kept, following its runner's rows.
    func model(for terminal: String, target: String = "") -> NativePaneModel {
        if let model = panes[terminal] { return model }
        let store = AgentRowStore(key: target.isEmpty ? "local-\(terminal)" : "remote-\(terminal)")
        let core = links[target]?.core
        let model = NativePaneModel(terminal: terminal, store: store, sink: core)
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
        model.source = CoreRowSource(core: core, terminal: model.terminal) { [weak self] in
            Task { @MainActor in self?.linkLost(target, core: core) }
        }
        model.sink = core
        // Line breaks, images and commands where the runner takes them.
        model.rich = offered.contains(Capability.compose.rawValue)
        // Stop and Send Now where the runner presses them (ov-368).
        model.keys = offered.contains(Capability.terminalInterrupt.rawValue) ? core : nil
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

        let model = model(for: terminal.id, target: target)
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

    /// Settings' Pair Again: pair `target`'s runner on the next connect, now.
    func pairAgain(_ target: String) {
        pairing.allowPairing(target: target)
        if enabled { reconnect(target) }
    }

    /// A model a test made, under the registry's rules.
    func adopt(_ model: NativePaneModel, target: String = "") {
        panes[model.terminal] = model
        paneTargets[model.terminal] = target
    }

    /// For tests: the setting as if turned on, against `core`.
    func pretend(enabled: Bool, rowsServed: Bool, core: RunnerCore?, offered: Set<String> = [], target: String = "") {
        self.enabled = enabled
        links[target] = Link(core: core, offered: offered, rowsServed: rowsServed)
    }
}
