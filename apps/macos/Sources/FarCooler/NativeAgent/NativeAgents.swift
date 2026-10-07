import AgentKit
import Foundation
import SwiftUI

/// The native view for terminal-mode claude panes, app-wide (ov-372): the
/// setting that gates it, the one connection to this Mac's runner it reads
/// through, and each pane's model.
///
/// **Off by default**, so it can ship before it matches the terminal. Turning
/// it on turns the runner's projector on (`farcooler settings set-projector
/// on`, which writes `[agents] projector` to the runner's config.toml and
/// takes effect at once), then reconnects, since a hello says what a runner
/// offers only when it's made.
///
/// **This Mac's runner only, for now.** A remote runner is reached by the
/// CLI over ssh, a process at a time; reading its rows without a process per
/// pane needs the client core's own ssh session, which wants a key the Mac
/// doesn't hold yet. Its panes show the terminal and no switch.
///
/// Wherever the view can't be offered (the setting off, the runner without
/// `agent_rows`, a pane that isn't claude in a terminal), the switch is
/// hidden and the terminal shows. Never a blank pane.
@MainActor
final class NativeAgents: ObservableObject {
    static let shared = NativeAgents()

    /// The user's setting.
    @Published private(set) var enabled: Bool
    /// Whether this Mac's runner serves rows, as its last hello said.
    @Published private(set) var rowsServed = false
    /// What went wrong turning the setting on or off, for Settings to show.
    @Published private(set) var settingTrouble: String?
    /// A change to the setting is on its way to the runner: the switch waits
    /// for it, so two flips can't land out of order.
    @Published private(set) var changing = false

    private(set) var core: RunnerCore?
    private var panes: [String: NativePaneModel] = [:]
    private let defaults: UserDefaults
    private var connecting: Task<Void, Never>?

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

    static let settingKey = "nativeAgent.enabled"

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        enabled = defaults.bool(forKey: Self.settingKey)
    }

    /// Start reading the runner, when the setting is on. Called at launch.
    func start() {
        guard enabled else { return }
        reconnect()
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
            reconnect()
            await connecting?.value
        } else {
            // A reconnect still on its way would otherwise set `core` after.
            connecting?.cancel()
            connecting = nil
            for pane in panes.values { pane.store.stop() }
            panes = [:]
            core = nil
            rowsServed = false
        }
    }

    /// A fresh connection, so the hello says what the runner offers now.
    func reconnect() {
        connecting?.cancel()
        let core = RunnerCore()
        let socket = socket()
        connecting = Task { [weak self] in
            let offered: Set<String>
            var trouble: String?
            do {
                offered = try await core.connect(socket: socket)
            } catch {
                // Said in Settings, beside the switch; the panes show their
                // terminals meanwhile.
                offered = []
                trouble = "Far Cooler can’t reach this Mac’s runner, so Claude panes show the terminal."
            }
            guard let self, !Task.isCancelled else { return }
            self.settingTrouble = trouble
            self.core = core
            // Rows to read and a way to send: a runner with rows from
            // before `terminal.compose` gets the terminal, not a view whose
            // every send would fail.
            self.rowsServed = Self.serves(offered)
            // Panes opened on the old connection follow on the new one.
            for pane in self.panes.values where self.rowsServed {
                self.follow(pane, on: core)
            }
        }
    }

    /// Whether `terminal`, on the runner `target` names, gets the native
    /// view: the setting on, this Mac's runner serving rows, and claude
    /// running in a terminal-mode pane.
    func offers(_ terminal: Terminal, target: String) -> Bool {
        enabled && rowsServed && core != nil && target.isEmpty && Self.isClaudeInATerminal(terminal)
    }

    /// Whether a hello offers the view: rows to read and compose to send.
    static func serves(_ offered: Set<String>) -> Bool {
        offered.contains("agent_rows") && offered.contains("agent_compose")
    }

    static func isClaudeInATerminal(_ terminal: Terminal) -> Bool {
        let mode = terminal.paneMode ?? "terminal"
        return mode == "terminal" && (terminal.program ?? terminal.preset).hasPrefix("claude")
    }

    /// The pane's model, made once and kept, following its rows.
    func model(for terminal: String) -> NativePaneModel {
        if let model = panes[terminal] { return model }
        let store = AgentRowStore(key: "local-\(terminal)")
        let model = NativePaneModel(terminal: terminal, store: store, sink: core)
        if let core { follow(model, on: core) }
        panes[terminal] = model
        return model
    }

    /// Give `model` this connection. It follows only while its view shows
    /// (`NativePaneModel.showsNative`): a pane on its terminal holds no
    /// follow on the runner.
    private func follow(_ model: NativePaneModel, on core: RunnerCore) {
        model.source = CoreRowSource(core: core, terminal: model.terminal)
        model.sink = core
        model.followIfShown()
    }

    /// A model a test made, under the registry's rules.
    func adopt(_ model: NativePaneModel) {
        panes[model.terminal] = model
    }

    /// For tests: the setting as if turned on, against `core`.
    func pretend(enabled: Bool, rowsServed: Bool, core: RunnerCore?) {
        self.enabled = enabled
        self.rowsServed = rowsServed
        self.core = core
    }
}
