import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The agent tray (ov-453), drawn in a real offscreen window: while any
/// subagent runs, main and every running agent sit pinned above the composer,
/// it folds to its header, and an agent opens to its own conversation in the
/// pane's place with a way back. Who's listed and in what words is AgentKit's
/// (`AgentTrayTests`); a subagent's own rows are the runner's
/// (`subagent_rows.rs`).
@MainActor
@Suite(.serialized)
struct NativeAgentTrayTests {
    typealias T = NativeAgentTests
    typealias P = NativePolishTests

    static var now: Int { Int(Date().timeIntervalSince1970 * 1000) }

    static func sub(_ ord: Int, _ id: String, _ description: String, action: String, type: String = "general-purpose", status: String = "Running", ago: Int, tokens: Int) -> [String: Any] {
        var sub: [String: Any] = ["tool_use_id": id, "agent_id": "agent-\(id)", "agent_type": type, "description": description]
        sub["background"] = true
        sub["status"] = status
        sub["started_ms"] = now - ago * 1000
        if status != "Running" { sub["ended_ms"] = now - 5_000 }
        sub["tool_count"] = 12
        sub["current_action"] = action
        sub["last_ms"] = now - 2_000
        sub["tokens"] = tokens
        return T.row(ord, "sub:\(id)", ["Subagent": sub])
    }

    /// The owner's own session of Oct 10, mid-dispatch: four lanes running,
    /// one already back, and main checking the build.
    static func rows(many: Bool = false) -> [[String: Any]] {
        base() + (many ? more() : [])
    }

    /// Four more lanes, so eight run: more than the tray shows at once.
    static func more() -> [[String: Any]] {
        [
            sub(8, "a6", "ov-456 relay retries", action: "Bash cargo test -p farcooler-relay", ago: 70, tokens: 9_800),
            sub(9, "a7", "ov-458 board note wrapping", action: "Read TaskNoteFeed.swift", ago: 50, tokens: 7_300),
            sub(10, "a8", "ov-459 iPad sidebar width", action: "Grep sidebarWidth", type: "Explore", ago: 30, tokens: 4_100),
            sub(11, "a9", "ov-461 Android queue words", action: "Read AgentConversation.kt", ago: 12, tokens: 2_600),
        ]
    }

    static func base() -> [[String: Any]] {
        [
            T.row(0, "turn:p1", ["Turn": [
                "prompt": "Split the chat-view feedback into lanes and get them going.", "origin": "Typed", "started_ms": now - 200_000,
                "background_running": 4, "activity": "Busy", "tokens": 52_100,
            ]]),
            P.prose(1, "prose:1", "I've split your feedback into five pieces of work. Four are running now:"),
            sub(2, "a1", "ov-452 conversation view hierarchy", action: "Bash Read old normalize result and task list code", ago: 190, tokens: 87_200),
            sub(3, "a2", "ov-454 image paste and attachments", action: "Read ComposerTextView paste and drag handling", ago: 185, tokens: 80_100),
            sub(4, "a3", "ov-453 subagent tray", action: "Grep subagents folder beside the session", type: "Explore", ago: 120, tokens: 41_900),
            sub(5, "a4", "ov-455 queue phrasing", action: "Bash Tally turnOrigin values in transcripts", ago: 82, tokens: 12_400),
            sub(6, "a5", "ov-451 strip URL tokens", action: "", status: "Completed", ago: 400, tokens: 30_000),
            T.row(7, "tool:t1", ["Tool": ["name": "Bash", "summary": "Check the build on main", "status": "Running", "started_ms": now - 4_000, "diff": [Any]()]]),
        ]
    }

    /// What `agent-a1` wrote in its own transcript.
    static func agentRows(_ agent: String) -> [[String: Any]] {
        [
            T.row(0, "turn:s1", ["Turn": [
                "prompt": "Make the conversation view read text first: fold long prompts, put tool rows under the words, and open each call to its input and result.",
                "origin": "Other", "started_ms": now - 190_000, "background_running": 0, "tokens": 87_200,
            ]]),
            P.thinking(1, "think:s1", ms: now - 189_000),
            P.tool(2, "tool:s1", "Bash", "Find the conversation view's rows", input: "command: rg -n NativeRows apps\ndescription: Find the conversation view's rows", result: "apps/macos/Sources/FarCooler/NativeAgent/NativeRows.swift", ms: now - 188_000),
            P.tool(3, "tool:s2", "Read", "apps/macos/Sources/FarCooler/NativeAgent/NativeRows.swift", ms: now - 180_000),
            P.prose(4, "prose:s1", "The rows are drawn in `NativeRows.swift`. A tool row is set in bold monospace, heavier than the reply, so I'll move it to the callout size in secondary and let it open to its input and result."),
            T.row(5, "tool:s3", ["Tool": ["name": "Bash", "summary": "Read old normalize result and task list code", "status": "Running", "started_ms": now - 3_000, "diff": [Any]()]]),
        ]
    }

    /// The pane's rows, and each agent's own where the runner serves them.
    struct Source: AgentRowSource {
        var agent: String? = nil
        var opens = true
        var many = false

        func page(before: UInt64?, limit: Int) async throws -> Data {
            let (agent, many) = (agent, many)
            return await MainActor.run { T.page(agent.map { NativeAgentTrayTests.agentRows($0) } ?? NativeAgentTrayTests.rows(many: many)) }
        }

        func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
            try await Task.sleep(for: .milliseconds(max(waitMs, 50)))
            return try JSONSerialization.data(withJSONObject: ["epoch": 1, "rev": afterRev, "reset": false, "changes": [Any]()])
        }

        func subagent(_ agentId: String) -> (any AgentRowSource)? {
            opens && agent == nil ? Source(agent: agentId, many: many) : nil
        }
    }

    static func shown(width: CGFloat = 720, height: CGFloat = 900, opens: Bool = true, many: Bool = false) async throws -> (NativePaneModel, P.Frames, NSWindow) {
        let model = T.model(try T.terminal())
        model.source = Source(opens: opens, many: many)
        model.store.apply(try await model.store.ledger.page(T.page(rows(many: many))))
        let frames = P.Frames()
        let window = P.window(width: width, height: height, P.Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        await T.settle(window, 300)
        return (model, frames, window)
    }

    @Test("While agents run, main and each running agent are pinned above the composer; none running, no tray")
    func theTrayListsTheRunningAgents() async throws {
        let (model, frames, window) = try await Self.shown()
        defer { window.close() }
        let seen = Set(frames.views.keys)
        #expect(seen.contains("native-agent-tray"))
        for id in [AgentTray.mainID, "sub:a1", "sub:a2", "sub:a3", "sub:a4"] {
            #expect(seen.contains("native-agent-tray-\(id)"), "\(id) is listed")
        }
        #expect(!seen.contains("native-agent-tray-sub:a5"), "one that ended is history, not the tray's")
        let tray = try #require(frames.views["native-agent-tray"])
        let composer = try #require(frames.views["native-composer"])
        #expect(tray.maxY <= composer.minY, "above the composer")
        #expect(tray.maxY > 900 - 260, "pinned at the bottom, not scrolled with the rows: \(tray)")

        // Every agent ends: the tray goes.
        let ended = Self.rows().map { row -> [String: Any] in
            guard var kind = row["kind"] as? [String: Any], var sub = kind["Subagent"] as? [String: Any] else { return row }
            sub["status"] = "Completed"
            kind["Subagent"] = sub
            var row = row
            row["kind"] = kind
            return row
        }
        model.store.apply(try await model.store.ledger.page(T.page(ended)))
        await T.settle(window, 300)
        #expect(!frames.views.keys.contains("native-agent-tray"))
    }

    @Test("Its header folds it to one line and opens it again")
    func theTrayFolds() async throws {
        let (model, frames, window) = try await Self.shown()
        defer { window.close() }
        #expect(P.click("native-agent-tray-header", frames: frames, in: window))
        await T.settle(window, 400)
        #expect(model.drill.collapsed)
        #expect(frames.views.keys.contains("native-agent-tray-header"), "still pinned, as its header")
        #expect(!frames.views.keys.contains("native-agent-tray-sub:a1"))
        #expect(P.click("native-agent-tray-header", frames: frames, in: window))
        await T.settle(window, 400)
        #expect(frames.views.keys.contains("native-agent-tray-sub:a1"))
    }

    @Test("An agent opens to its own conversation in the pane's place, and Back returns")
    func anAgentOpensAndBackReturns() async throws {
        let (model, frames, window) = try await Self.shown()
        defer { window.close() }
        #expect(P.click("native-agent-tray-sub:a1", frames: frames, in: window))
        await T.settle(window, 600)
        #expect(model.drill.opened?.agentId == "agent-a1")
        var seen: Set<String> { Set(frames.views.keys) }
        #expect(seen.contains("native-agent-back"), "a clear way back")
        #expect(seen.contains("native-agent-header"))
        #expect(seen.contains("native-row-prose:s1"), "the agent's own words, in the view's style")
        #expect(!seen.contains("native-row-prose:1"), "not the pane's")
        #expect(!seen.contains("native-composer"), "nothing to send to an agent")
        #expect(seen.contains("native-agent-tray"), "the tray stays, to go to another")

        #expect(P.click("native-agent-back", frames: frames, in: window))
        await T.settle(window, 600)
        #expect(model.drill.opened == nil)
        #expect(seen.contains("native-row-prose:1") && seen.contains("native-composer"))

        // The inline row in the transcript opens it too.
        #expect(P.click("native-subagent-open-sub:a2", frames: frames, in: window))
        await T.settle(window, 600)
        #expect(model.drill.opened?.agentId == "agent-a2")
        // And main, in the tray, goes back.
        #expect(P.click("native-agent-tray-\(AgentTray.mainID)", frames: frames, in: window))
        await T.settle(window, 400)
        #expect(model.drill.opened == nil)
    }

    /// The tray's own scroll view, scrolled to its end, as a wheel would.
    static func scrollTrayToEnd(_ window: NSWindow) -> Bool {
        func all(_ view: NSView) -> [NSScrollView] {
            (view as? NSScrollView).map { [$0] } ?? [] + view.subviews.flatMap(all)
        }
        guard let root = window.contentView,
            // The list's: the tallest no taller than the tray's cap (the
            // composer's field is one too, a line high).
            let tray = all(root).filter({ $0.frame.height <= NativeAgentTray.tallest + 1 }).max(by: { $0.frame.height < $1.frame.height }),
            let document = tray.documentView
        else { return false }
        let end = NSPoint(x: 0, y: document.isFlipped ? max(0, document.frame.height - tray.contentView.bounds.height) : 0)
        tray.contentView.scroll(to: end)
        tray.reflectScrolledClipView(tray.contentView)
        return true
    }

    @Test("Eight agents scroll in the tray, none cut off, and the last one opens")
    func everyAgentIsReachable() async throws {
        let (model, frames, window) = try await Self.shown(many: true)
        defer { window.close() }
        let list = try #require(frames.views["native-agent-tray-list"])
        #expect(list.height <= NativeAgentTray.tallest + 1, "it scrolls rather than covering the pane: \(list)")
        for id in ["sub:a1", "sub:a2", "sub:a3", "sub:a4", "sub:a6", "sub:a7", "sub:a8", "sub:a9"] {
            #expect(frames.views.keys.contains("native-agent-tray-\(id)"), "\(id) is in the list")
        }
        let last = try #require(frames.views["native-agent-tray-sub:a9"])
        #expect(last.minY >= list.maxY - 1, "below the fold until scrolled: \(last) in \(list)")

        #expect(Self.scrollTrayToEnd(window))
        await T.settle(window, 400)
        let shown = try #require(frames.views["native-agent-tray-sub:a9"])
        #expect(shown.maxY <= list.maxY + 1 && shown.minY >= list.minY - 1, "scrolled into view: \(shown) in \(list)")
        #expect(P.click("native-agent-tray-sub:a9", frames: frames, in: window))
        await T.settle(window, 600)
        #expect(model.drill.opened?.agentId == "agent-a9", "the last agent opens")
    }

    @Test("⌘[ goes back from an agent's conversation, as Back does")
    func commandBracketGoesBack() async throws {
        let (model, frames, window) = try await Self.shown()
        defer { window.close() }
        #expect(P.click("native-agent-tray-sub:a1", frames: frames, in: window))
        await T.settle(window, 600)
        #expect(model.drill.opened != nil)
        let key = try #require(NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, characters: "[", charactersIgnoringModifiers: "[", isARepeat: false, keyCode: 33))
        #expect(window.performKeyEquivalent(with: key), "the window took ⌘[")
        await T.settle(window, 400)
        #expect(model.drill.opened == nil, "back to the conversation")
        #expect(frames.views.keys.contains("native-composer"))
    }

    @Test("A runner without subagent_rows lists the agents and opens none")
    func anOlderRunnerOpensNone() async throws {
        let (model, frames, window) = try await Self.shown(opens: false)
        defer { window.close() }
        #expect(frames.views.keys.contains("native-agent-tray-sub:a1"))
        #expect(!frames.views.keys.contains("native-subagent-open-sub:a1"), "the inline row isn't a button")
        P.click("native-agent-tray-sub:a1", frames: frames, in: window)
        await T.settle(window, 300)
        #expect(model.drill.opened == nil)
    }
}

/// Opt-in (FARCOOLER_CAPTURE_OUT): the tray with four agents running, at a
/// narrow and a wide width, light and dark, and one agent opened.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct NativeAgentTrayCaptures {
    @Test func agentTray() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (width, size) in [(CGFloat(420), "narrow"), (CGFloat(1_000), "wide")] {
            for (name, appearance) in RealWindowCaptures.variants.prefix(2) {
                for (state, act) in [("tray", ""), ("collapsed", "native-agent-tray-header"), ("opened", "native-agent-tray-sub:a1")] {
                    // Eight running at the narrow width: the list scrolls.
                    let (_, frames, window) = try await NativeAgentTrayTests.shown(width: width, height: 900, many: size == "narrow")
                    window.appearance = NSAppearance(named: appearance)
                    await NativeAgentTests.settle(window, 300)
                    if !act.isEmpty {
                        NativePolishTests.click(act, frames: frames, in: window)
                        await NativeAgentTests.settle(window, 700)
                    }
                    let image = try #require(RealWindowCaptures.windowImage(window), "screencapture -l isn't allowed here")
                    try #require(image.representation(using: .png, properties: [:]))
                        .write(to: URL(fileURLWithPath: out).appendingPathComponent("mac-\(size)-\(name)-\(state).png"))
                    window.close()
                }
            }
        }
    }
}
