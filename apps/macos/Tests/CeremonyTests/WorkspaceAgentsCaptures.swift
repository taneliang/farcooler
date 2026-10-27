import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the cards at the top of a workspace's
/// conversation (ov-641), the production `WorkspaceView` with the Plan beside
/// the chat at a wide window and folded away at a narrow one, over the busy
/// plan, with the orchestrator's conversation and four agents at work. In the
/// two appearances, light and dark (the high-contrast ones drew the same: the app's colors don't read the setting), in a real titled
/// window off every screen, taken with `screencapture -l`. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct WorkspaceAgentsCaptures {
    typealias T = NativeAgentTests

    /// What is at work, in the colors it would be: the orchestrator with one subagent, and three lanes, one whose turn died,
    /// one waiting on the owner and one working. The plan the window carries is the busy one, which lists its own agents (the
    /// runner's count of them, from the turns it read) beside this card's (the fleet's panes and their subagents).
    static func fleet(running: Bool) throws -> Fleet {
        let turn = Date().timeIntervalSince1970 * 1000 - 185_000
        return try WorkspaceAgentsTests.fleet([
            WorkspaceAgentsTests.worktree("checkout", [
                WorkspaceAgentsTests.terminal(
                    "orch", role: "orchestrator", subagents: running ? ["Survey the repo for the card"] : [],
                    running: running ? 1 : 0, ids: running ? ["toolu_s"] : [])
            ]),
            WorkspaceAgentsTests.worktree("lane-fix", running ? [
                WorkspaceAgentsTests.terminal("fix", title: "Fix ov-640", activity: "done", line: "cargo test failed: 3 errors", turn: nil, failed: true)
            ] : []),
            WorkspaceAgentsTests.worktree("lane-review", running ? [
                WorkspaceAgentsTests.terminal("review", title: "Review ov-616", activity: "blocked", line: "Allow Bash cargo test?", turn: nil)
            ] : []),
            WorkspaceAgentsTests.worktree("lane-build", running ? [
                WorkspaceAgentsTests.terminal("build", title: "Build ov-619", line: "Edit RunningAgents.swift", turn: turn)
            ] : []),
        ])
    }

    static func shoot(
        _ name: String, width: CGFloat, height: CGFloat = 820, agents running: Bool = true, long: Bool = false,
        expecting: String
    ) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let board = try await PlanSectionViewTests.store(long ? .ladder : running ? .busy : .quiet)
        for (variant, appearance) in RealWindowCaptures.variants.prefix(2) {
            let model = T.model(try T.terminal())
            model.source = NativeAgentSubagentTests.Source(opens: true)
            model.store.apply(try await model.store.ledger.page(T.page(NativeAgentSubagentTests.rows())))
            let fleet = try Self.fleet(running: running)
            let rows = AgentTray.entries(model.store)
            let agents = MacAgents.running(workspace: WorkspaceAgentsTests.main, host: "", fleet: fleet) { _ in rows }
            // What the card is shot for: the plan read, and the agents at work when there are any.
            let kind: PlanScenario.Kind = long ? .ladder : running ? .busy : .quiet
            let reached = { @MainActor in PlanScenario.shown(kind, in: board.plan.plan) && (!running || !rows.isEmpty) }
            let view = WorkspaceView(
                opened: nil as String?, hasConversation: true, cell: WorkspaceColumns.defaultCell, focused: false,
                navigatorWidth: .constant(320),
                conversation: { NativeAgentView(model: model, isFocused: true, showTerminal: {}).paneCard().paneCanvas() },
                navigator: { Color.clear }, breadcrumb: { _ in Color.clear }, detail: { _, _ in Color.clear }, split: true,
                home: { AnyView(PlanHome(board: board, needsYou: PlanNeedsYou(), onOpen: { _ in })) },
                cards: { canvas in
                    canvas ? AnyView(EmptyView()) : AnyView(
                        WorkspaceStripRow {
                            PlanStripRow(plan: board.plan, needsYou: 2, statuses: board.board.statuses) {}
                        })
                })
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: view.frame(width: width, height: height).background(WorkspaceStyle.canvas))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            try await Capture.wait(expecting: expecting, until: reached)
            await T.settle(window, 900)
            try await Capture.keep(to: URL(fileURLWithPath: out).appendingPathComponent("mac-\(name)-\(Int(width))-\(variant).png"), expecting: expecting, until: reached) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
            window.close()
        }
    }

    @Test(arguments: [760.0, 1500.0])
    func cards(_ width: CGFloat) async throws {
        try await Self.shoot("cards", width: width, expecting: "four agents at work")
    }

    @Test(arguments: [760.0, 1500.0])
    func noneRunning(_ width: CGFloat) async throws {
        try await Self.shoot("cards-none", width: width, agents: false, expecting: "no agents at work")
    }

    @Test(arguments: [500.0])
    func narrowest(_ width: CGFloat) async throws {
        try await Self.shoot("cards-long", width: width, long: true, expecting: "the long ladder plan, agents at work")
    }

    /// The panel the card opens, over the card's own material.
    @Test func panel() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let model = T.model(try T.terminal())
        model.source = NativeAgentSubagentTests.Source(opens: true)
        model.store.apply(try await model.store.ledger.page(T.page(NativeAgentSubagentTests.rows())))
        let rows = AgentTray.entries(model.store)
        let fleet = try Self.fleet(running: true)
        let agents = MacAgents.running(workspace: WorkspaceAgentsTests.main, host: "", fleet: fleet) { $0 == "orch" ? rows : nil }
        let expecting = "the orchestrator's subagents listed"
        let reached = { @MainActor in !rows.isEmpty }
        for (variant, appearance) in RealWindowCaptures.variants.prefix(2) {
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: 420, height: 520), styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: AgentsPanel(agents: agents) { _ in }.frame(width: 420, height: 520, alignment: .top).background(.regularMaterial))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            try await Capture.wait(expecting: expecting, until: reached)
            await T.settle(window, 600)
            try await Capture.keep(to: URL(fileURLWithPath: out).appendingPathComponent("mac-agents-panel-\(variant).png"), expecting: expecting, until: reached) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
            window.close()
        }
    }
}
