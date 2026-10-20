import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A pane's subagents (ov-453, ov-641), drawn in a real offscreen window: no
/// tray over the box (the agents at work are the Agents card's, at the top of
/// the column), an agent opening to its own conversation in the pane's place
/// with a way back, and what a runner without `subagent_rows` offers. Who's
/// counted and in what words is AgentKit's (`RunningAgentsTests`); a
/// subagent's own rows are the runner's (`subagent_rows.rs`).
@MainActor
@Suite(.serialized)
struct NativeAgentSubagentTests {
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

    /// Four more lanes, so eight run.
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

    static let longPrompt = "Run shell commands from /tmp/fc-t/ov463/runs/old-S21-2/home. Read nothing outside /tmp/fc-t/ov463/runs/old-S21-2. Your final message is your reply to the owner. "
        + String(repeating: "Make the conversation view read text first: fold long prompts, put tool rows under the words, and open each call to its input and result. ", count: 4)

    /// What `agent-a1` wrote in its own transcript.
    static func agentRows(_ agent: String, long: Bool = false) -> [[String: Any]] {
        (long ? (0..<12).map { i in P.prose(20 + i, "prose:long\(i)", "Reply \(i), long enough to push the first row out of a short pane when the rows are stacked.") } : [])
        + [
            T.row(0, "turn:s1", ["Turn": [
                "prompt": long ? longPrompt : "Make the conversation view read text first: fold long prompts, put tool rows under the words, and open each call to its input and result.",
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
        var long = false

        func page(before: UInt64?, limit: Int) async throws -> Data {
            let (agent, many, long) = (agent, many, long)
            return await MainActor.run { T.page(agent.map { NativeAgentSubagentTests.agentRows($0, long: long) } ?? NativeAgentSubagentTests.rows(many: many)) }
        }

        func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
            try await Task.sleep(for: .milliseconds(max(waitMs, 50)))
            return try JSONSerialization.data(withJSONObject: ["epoch": 1, "rev": afterRev, "reset": false, "changes": [Any]()])
        }

        func subagent(_ agentId: String) -> (any AgentRowSource)? {
            opens && agent == nil ? Source(agent: agentId, many: many, long: long) : nil
        }
    }

    static func shown(width: CGFloat = 720, height: CGFloat = 900, opens: Bool = true, many: Bool = false, long: Bool = false) async throws -> (NativePaneModel, P.Frames, NSWindow) {
        let model = T.model(try T.terminal())
        model.source = Source(opens: opens, many: many, long: long)
        model.store.apply(try await model.store.ledger.page(T.page(rows(many: many))))
        let frames = P.Frames()
        // In the card it has in the app, on its plane: the card's corners clip
        // what the view draws near its edge (ov-467).
        let window = P.window(width: width, height: height, P.Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {}).paneCard().paneCanvas()))
        await T.settle(window, still: frames)
        return (model, frames, window)
    }

    @Test("No tray sits over the box: the agents at work are the Agents card's, at the top of the column (ov-641)")
    func noTrayOverTheBox() async throws {
        let (_, frames, window) = try await Self.shown()
        defer { window.close() }
        let seen = Set(frames.views.keys)
        #expect(!seen.contains { $0.hasPrefix("native-agent-tray") }, "a tray is back: \(seen.filter { $0.hasPrefix("native-agent-tray") })")
        let composer = try #require(frames.views["native-composer"])
        #expect(composer.maxY > 900 - 200, "the box is pinned at the bottom with nothing over it: \(composer)")
        // Four agents run, and the inline rows in the transcript stay as history.
        for id in ["sub:a1", "sub:a2", "sub:a3", "sub:a4"] {
            #expect(seen.contains("native-subagent-open-\(id)"), "\(id) is in the transcript, to open")
        }
    }

    @Test("An agent opens to its own conversation in the pane's place, and Back returns")
    func anAgentOpensAndBackReturns() async throws {
        let (model, frames, window) = try await Self.shown()
        defer { window.close() }
        // From the Agents panel's row, or the inline row in the transcript.
        #expect(P.click("native-subagent-open-sub:a1", frames: frames, in: window))
        await T.settle(window, 600)
        #expect(model.drill.opened?.agentId == "agent-a1")
        var seen: Set<String> { Set(frames.views.keys) }
        #expect(seen.contains("native-agent-back"), "a clear way back")
        #expect(seen.contains("native-agent-header"))
        #expect(seen.contains("native-row-prose:s1"), "the agent's own words, in the view's style")
        #expect(!seen.contains("native-row-prose:1"), "not the pane's")
        #expect(!seen.contains("native-composer"), "nothing to send to an agent")

        #expect(P.click("native-agent-back", frames: frames, in: window))
        await T.settle(window, 600)
        #expect(model.drill.opened == nil)
        #expect(seen.contains("native-row-prose:1") && seen.contains("native-composer"))

        // The Agents panel opens one too: through the model, as its row does.
        #expect(model.openAgent(row: "sub:a2", agentId: "agent-a2"))
        await T.settle(window, 600)
        #expect(model.drill.opened?.agentId == "agent-a2")
        // And another from there, without going back first.
        #expect(model.openAgent(row: "sub:a3", agentId: "agent-a3"))
        await T.settle(window, 400)
        #expect(model.drill.opened?.agentId == "agent-a3")
        model.closeAgent()
        await T.settle(window, 400)
        #expect(model.drill.opened == nil)
    }

    /// The transcript's own scroll view (the tallest), moved to its top or end.
    static func scrollTranscript(_ window: NSWindow, toTop: Bool) -> Bool {
        func all(_ view: NSView) -> [NSScrollView] {
            (view as? NSScrollView).map { [$0] } ?? [] + view.subviews.flatMap(all)
        }
        guard let root = window.contentView, let list = all(root).max(by: { $0.frame.height < $1.frame.height }), let document = list.documentView else { return false }
        let end = NSPoint(x: 0, y: toTop ? -list.contentInsets.top : !document.isFlipped ? 0 : max(0, document.frame.height - list.contentView.bounds.height))
        list.contentView.scroll(to: end)
        list.reflectScrolledClipView(list.contentView)
        print("DBG2", list.contentView.bounds)
        return true
    }

    /// Whether the transcript's scroll view (the tallest) is scrolled to its end.
    static func transcriptAtEnd(_ window: NSWindow) -> Bool {
        func all(_ view: NSView) -> [NSScrollView] {
            (view as? NSScrollView).map { [$0] } ?? [] + view.subviews.flatMap(all)
        }
        guard let root = window.contentView, let list = all(root).max(by: { $0.frame.height < $1.frame.height }), let document = list.documentView
        else { return false }
        let end = max(0, document.frame.height - list.contentView.bounds.height)
        return end > 0 && list.contentView.bounds.origin.y >= end - 1
    }

    /// Whether the transcript's scroll view (the tallest) is scrolled to its top.
    static func transcriptAtTop(_ window: NSWindow) -> Bool {
        func all(_ view: NSView) -> [NSScrollView] {
            (view as? NSScrollView).map { [$0] } ?? [] + view.subviews.flatMap(all)
        }
        guard let root = window.contentView, let list = all(root).max(by: { $0.frame.height < $1.frame.height }) else { return false }
        return list.contentView.bounds.origin.y <= 0
    }

    @Test("A subagent's header sits clear of its first row, at the top of a long transcript, narrow and wide (ov-467)")
    func theHeaderClearsTheFirstRow() async throws {
        for width in [CGFloat(420), 1_000] {
            let (model, frames, window) = try await Self.shown(width: width, height: 500, long: true)
            defer { window.close() }
            #expect(P.click("native-subagent-open-sub:a1", frames: frames, in: window))
            // The agent's own rows are read after the click, and the transcript
            // is drawn again as its store goes live and follows its end, which
            // takes it back down from the top. So it is taken to the top until
            // it stays there for a draw, and read in the same turn.
            //
            // ov-537: this was a 600 ms sleep, a scroll and a 1.5 s sleep, and
            // by then the follow had already taken the transcript back to its
            // end: the test read `turn:s1`, the row at the bottom here (the
            // long replies come before it), as "the first row", and could not
            // go red for a header on top of the real one.
            await Settle.until { frames.views["native-row-turn:s1"] != nil && Self.transcriptAtEnd(window) }
            await Settle.until {
                _ = Self.scrollTranscript(window, toTop: true)
                window.contentView?.layoutSubtreeIfNeeded()
                window.displayIfNeeded()
                return Self.transcriptAtTop(window) && frames.views.keys.contains { $0.hasPrefix("native-row-") }
            }
            let header = try #require(frames.views["native-agent-header"])
            let rows = frames.views.filter { $0.key.hasPrefix("native-row-") }
            let first = try #require(rows.min { $0.value.minY < $1.value.minY }, "no row drawn at the top").value
            // What the reader sees of the row is what the transcript's own
            // frame leaves of it; the rest is scrolled out of view.
            let transcript = frames.views["native-transcript"] ?? CGRect(x: 0, y: 0, width: width, height: 500)
            let seen = first.intersection(transcript)
            #expect(transcript.minY >= header.maxY - 0.5, "width \(width): the transcript \(transcript) starts below the header \(header)")
            #expect(seen.isNull || header.intersection(seen).height < 1, "width \(width): the header \(header) and the first row \(seen) overlap")
        }
    }

    @Test("The message box sits inside the pane with the same margin below as at its sides, with attachments (ov-467)")
    func theComposerKeepsItsBottomMargin() async throws {
        let png = try #require(ComposeImage.make(NativeComposerTests.png(), type: .png))
        for (width, attached) in [(CGFloat(420), false), (1_000, true), (420, true), (1_000, false)] {
            let model = T.model(try T.terminal())
            model.source = Source(opens: true)
            model.store.apply(try await model.store.ledger.page(T.page(Self.rows())))
            model.rich = true
            if attached { #expect(model.attach([png])) }
            let frames = P.Frames()
            let window = P.window(width: width, height: 600, P.Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {}).paneCard().paneCanvas()))
            defer { window.close() }
            await T.settle(window, still: frames)
            let composer = try #require(frames.views["native-composer-box"])
            let note = "width \(width), attached \(attached): composer \(composer)"
            #expect(attached == frames.views.keys.contains("native-image-chips"), "the attachment strip shows when one waits: \(note)")
            // The card the pane is drawn in, on its plane.
            let pane = try #require(frames.views["native-agent-view"], "\(note)")
            #expect(pane.contains(composer), "the box is inside the pane: pane \(pane): \(note)")
            let side = composer.minX - pane.minX
            #expect(abs(side - Spacing.inset) < 1 && abs(pane.maxX - composer.maxX - side) < 1, "equal side insets of \(Spacing.inset), not \(side): \(note)")
            let bottom = pane.maxY - composer.maxY
            #expect(abs(bottom - side) < 1, "a bottom inset equal to the sides' \(side), not \(bottom): \(note)")
        }
    }

    @Test("⌘[ goes back from an agent's conversation, as Back does")
    func commandBracketGoesBack() async throws {
        let (model, frames, window) = try await Self.shown()
        defer { window.close() }
        #expect(P.click("native-subagent-open-sub:a1", frames: frames, in: window))
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
        #expect(!frames.views.keys.contains("native-subagent-open-sub:a1"), "the inline row isn't a button")
        #expect(!model.openAgent(row: "sub:a1", agentId: "agent-a1"), "the panel's row can't open it either")
        await T.settle(window, 300)
        #expect(model.drill.opened == nil)
    }
}
