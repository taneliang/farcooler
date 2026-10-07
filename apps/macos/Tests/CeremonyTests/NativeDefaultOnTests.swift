import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// What the conversation view needed before it could be on by default
/// (ov-394): follow only panes on screen, a blink of the runner that doesn't
/// tear the view down, a follow that ended unavailable coming back, and a
/// key for the switch. On the stand-ins of `NativeAgentTests`.
@MainActor
@Suite(.serialized)
struct NativeDefaultOnTests {
    /// Poll for `condition`, and return early. A bound well past any honest
    /// wait, since CI's runner is slow.
    static func until(_ seconds: Int = 30, _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(seconds)
        while !condition(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
    }

    /// Rows that can't be had until `served` says so: the runner with its
    /// projector off, then on again.
    final class Switchable: AgentRowSource, @unchecked Sendable {
        private let lock = NSLock()
        private var served = false
        var serves: Bool {
            get { lock.withLock { served } }
            set { lock.withLock { served = newValue } }
        }
        func page(before: UInt64?, limit: Int) async throws -> Data {
            if !serves { throw AgentRowsUnavailable() }
            return try JSONSerialization.data(withJSONObject: ["epoch": 1, "rev": 1, "moreBefore": false, "rows": [Any]()])
        }
        func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
            if !serves { throw AgentRowsUnavailable() }
            try await Task.sleep(for: .seconds(60))
            return Data()
        }
    }

    // MARK: - Only panes on screen follow

    @Test("A pane remembered as the conversation follows only while it's on screen")
    func onlyAPaneOnScreenFollows() throws {
        let model = NativeAgentTests.model(try NativeAgentTests.terminal())
        model.source = NativeAgentTests.FailingAfterAPage(page: NativeAgentTests.page([]))
        model.showsNative = true
        #expect(!model.store.isFollowing, "remembered as the conversation, and nobody can see it")
        model.onScreen = true
        #expect(model.store.isFollowing)
        model.onScreen = false
        #expect(!model.store.isFollowing)
    }

    @Test("A mounted pane behind a zoomed one, or in a hidden window, holds no follow")
    func aPaneOutOfSightHoldsNoFollow() async throws {
        for (outOfSight, windowVisible) in [(true, true), (false, false), (false, true)] {
            let terminal = try NativeAgentTests.terminal()
            let model = NativeAgentTests.model(terminal)
            model.source = NativeAgentTests.FailingAfterAPage(page: NativeAgentTests.page([]))
            model.showsNative = true
            let agents = NativeAgentTests.agents(model: model)
            let life = NativeAgentTests.SurfaceLife()
            let window = NativeAgentTests.window(
                NativeSwitch(terminal: terminal, target: "", isFocused: true, agents: agents) { focused in
                    NativeAgentTests.StandInSurface(life: life, focused: focused)
                }
                .environment(\.outOfSight, outOfSight)
                .environment(\.windowVisible, windowVisible))
            defer {
                window.close()
                model.store.stop()
                NativePaneModel.remember(false, for: terminal.id)
            }
            let shown = !outOfSight && windowVisible
            try await Self.until { model.store.isFollowing == shown }
            #expect(model.store.isFollowing == shown, "out of sight \(outOfSight), window visible \(windowVisible)")
            #expect(model.onScreen == shown)
        }
    }

    // MARK: - A blink of the runner

    /// The iOS review's finding 1, as the Mac has it: a reconnect that finds
    /// the runner not answering must not unmake the view. It's torn down
    /// (the switch gone, the terminal up and taking the keyboard) and comes
    /// back a retry later. The Mac's gate is the last hello's word, so a
    /// reconnect that succeeds never blinks; one that fails did.
    @Test("A reconnect the runner doesn't answer keeps the view it had")
    func aFailedReconnectKeepsTheView() async throws {
        let agents = NativeAgentTests.agents()
        let before = agents.core
        agents.socket = { "/tmp/fc-t/no-such-runner/farcoolerd.sock" }
        agents.reconnect()
        try await Self.until { agents.settingTrouble != nil }
        #expect(agents.settingTrouble != nil, "the reconnect never finished")
        #expect(agents.rowsServed, "the view was hidden for a blink")
        #expect(agents.core === before)
        let terminal = try NativeAgentTests.terminal()
        #expect(agents.offers(terminal, target: ""))
    }

    @Test("A runner that was never reached offers nothing")
    func aRunnerNeverReachedOffersNothing() async throws {
        let agents = NativeAgentTests.agents(rows: false)
        agents.socket = { "/tmp/fc-t/no-such-runner/farcoolerd.sock" }
        agents.reconnect()
        try await Self.until { agents.settingTrouble != nil }
        #expect(!agents.rowsServed)
        #expect(!agents.offers(try NativeAgentTests.terminal(), target: ""))
    }

    // MARK: - Unavailable, then served again

    /// The iOS review's finding 2: a follow that ended `.unavailable` is over
    /// (its loop returned), and the pane held it as running. When the pane
    /// is shown again with rows served, it follows again.
    @Test("A follow that ended unavailable starts again once the pane is shown")
    func anUnavailableFollowComesBack() async throws {
        let model = NativeAgentTests.model(try NativeAgentTests.terminal())
        let source = Switchable()
        model.onScreen = true
        model.source = source
        model.showsNative = true
        defer { model.store.stop() }
        try await Self.until { model.store.phase == .unavailable }
        #expect(model.store.phase == .unavailable)
        source.serves = true
        // What the view does as it comes back (`NativeSwitch`'s task).
        model.followIfShown()
        try await Self.until { model.store.phase == .live }
        #expect(model.store.phase == .live, "stuck at \(model.store.phase) until a relaunch")
    }

    /// And the setting's own off and on, which the iOS review named: the
    /// panes are dropped, so the next ones are made new, not the old ones
    /// in their dead state.
    @Test("Turning the setting off and on makes the panes anew")
    func offThenOnMakesPanesAnew() async throws {
        let agents = NativeAgentTests.agents()
        agents.socket = { "/tmp/fc-t/no-such-runner/farcoolerd.sock" }
        agents.setProjector = { _ in nil }
        let terminal = try NativeAgentTests.terminal()
        let old = agents.model(for: terminal.id)
        defer { NativePaneModel.remember(false, for: terminal.id) }
        await agents.setEnabled(false)
        await agents.setEnabled(true)
        let new = agents.model(for: terminal.id)
        #expect(new !== old)
        #expect(new.store.phase != .unavailable)
    }

    // MARK: - The key

    @Test("The switch's command flips an offered pane, and says no to another")
    func theCommandFlipsOnlyAnOfferedPane() throws {
        let terminal = try NativeAgentTests.terminal()
        let agents = NativeAgentTests.agents()
        defer { NativePaneModel.remember(false, for: terminal.id) }
        #expect(agents.toggleView(of: terminal, target: ""))
        #expect(agents.model(for: terminal.id).showsNative)
        #expect(agents.toggleView(of: terminal, target: ""))
        #expect(!agents.model(for: terminal.id).showsNative)
        let shell = try NativeAgentTests.terminal(id: "0199aaaa-0000-7000-8000-000000000002", program: "shell")
        #expect(!agents.toggleView(of: shell, target: ""))
        #expect(agents.model(ifMade: shell.id) == nil, "a shell pane got a model")
        #expect(!NativeAgentTests.agents(offering: false).toggleView(of: terminal, target: ""))
    }
}
