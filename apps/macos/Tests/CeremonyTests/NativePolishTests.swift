import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The conversation view reads text first (ov-452), drawn in a real
/// offscreen window from rows as the runner serves them: a scheduled task
/// says it is one, a long message folds, a run of tool calls is one line that
/// opens to each call and each call to its input and result, and a task list
/// is a checklist. The words and the grouping are AgentKit's and tested there
/// (`ConversationItemsTests`); the rows are the projector's
/// (`polish_tests.rs`).
@MainActor
@Suite(.serialized)
struct NativePolishTests {
    typealias T = NativeAgentTests

    static func turn(_ ord: Int, _ id: String, _ prompt: String, origin: String, ms: Int) -> [String: Any] {
        T.row(ord, id, ["Turn": [
            "prompt": prompt, "origin": origin, "started_ms": ms, "ended_ms": ms + 15_000, "duration_ms": 15_000,
            "outcome": "Finished", "background_running": 0, "activity": NSNull(),
        ]])
    }

    static func tool(
        _ ord: Int, _ id: String, _ name: String, _ summary: String, input: String? = nil, result: String? = nil, ms: Int
    ) -> [String: Any] {
        var tool: [String: Any] = [
            "name": name, "summary": summary, "status": "Done", "started_ms": ms, "ended_ms": ms + 400, "diff": [Any](),
        ]
        tool["input"] = input
        tool["result"] = result
        return T.row(ord, id, ["Tool": tool])
    }

    static func thinking(_ ord: Int, _ id: String, ms: Int) -> [String: Any] {
        T.row(ord, id, ["Thinking": ["started_ms": ms, "ended_ms": ms + 2_000]])
    }

    static func prose(_ ord: Int, _ id: String, _ text: String) -> [String: Any] {
        T.row(ord, id, ["Prose": ["text": text, "conclusion": true, "at_ms": 1]])
    }

    /// The owner's own session of Oct 10, as its rows came: a long message,
    /// a run of commands, a message taken mid-turn, a subagent, the answer;
    /// then a scheduled check-in, its two calls, a task list and the answer.
    static let checkIn = """
        1. **Re-evaluate the plan.** Step back from the queue. Is the current execution plan still balancing the owner's priorities: engineering quality, product quality, cost/token efficiency and velocity? Look at what is running, what is queued next, and what landed or stalled since the last check-in.
        2. **Re-check the themes and lanes themselves** (`farcooler-canary plan --repo overnight`):
          - Is each theme at the right granularity (not a grab-bag, not a single card) and still about one outcome?
          - Do cards sit in the right theme? Are new kinds of work missing a theme?
        3. **Take initiative within each theme.** The themes describe what the owner is trying to achieve.
        4. **Learn.** What went well or badly since the last check-in?
        5. **Run the loop:** board triage, verify and land finished lanes, dispatch ready work within the cap.

        Keep board notes short and readable. Never end without a check-in armed.
        """

    static let ask = """
        Can you polish the chat view a bit? Few things to note: there's a wall of detail text that should probably be collapsed.
        Things like bash calls, subagents, and stuff seem to be emphasized more than the actual text output, which is not the right hierarchy from a UI perspective.
        If subagents are running, we should ensure that there's a visible list of them.
        Also ensure that task lists are rendered correctly.
        Also some function calls say things like "CronCreate" but there's no detail.
        I think the collapsible UI that the old chat had was much better in this regard.
        I'm also confused about what "Sent from the queue" is.
        """

    static func rows() -> [[String: Any]] {
        [
            turn(0, "turn:p1", ask, origin: "Typed", ms: 1_000),
            thinking(1, "think:1", ms: 2_000),
            tool(2, "tool:b1", "Bash", "Map the conversation view code", input: "command: rg -n NativeRows apps\ndescription: Map the conversation view code", result: "apps/macos/Sources/FarCooler/NativeAgent/NativeRows.swift\napps/ios/FarCooler/NativeRows.swift", ms: 3_000),
            thinking(3, "think:2", ms: 4_000),
            tool(4, "tool:b2", "Bash", "Find the row source and paste handling", ms: 5_000),
            thinking(5, "think:3", ms: 6_000),
            tool(6, "tool:b3", "Bash", "Find transcript parsing and related cards", ms: 7_000),
            T.row(7, "queued:1", ["Queued": ["text": "If messages are attached, would be nice to be able to view them in the chat as well [Image #62] [Image #63]", "state": "Sent", "at_ms": 7_500]]),
            T.row(8, "sub:a1", ["Subagent": [
                "tool_use_id": "a1", "agent_type": "general-purpose", "description": "ov-452 conversation view hierarchy", "background": true,
                "status": "Running", "started_ms": Int(Date().timeIntervalSince1970 * 1000) - 160_000, "tool_count": 27, "current_action": "Bash Read old normalize result and task list code", "last_ms": 9_000,
            ]]),
            prose(9, "prose:1", "I've split your chat-view feedback into four pieces of work. Two are running now:\n\n- **ov-452**, the conversation view's hierarchy: long prompts fold, tool rows sit under the text and open to their input and result.\n- **ov-454**, pasting images and showing a message's attachments."),
            turn(10, "turn:p2", checkIn, origin: "Scheduled", ms: 20_000),
            thinking(11, "think:4", ms: 21_000),
            tool(12, "tool:c1", "Bash", "Get current time", input: "command: date\ndescription: Get current time", result: "Sat Oct 10 09:34:02 PDT 2026", ms: 22_000),
            tool(13, "tool:c2", "CronCreate", "", input: "cron: 31 11 10 10 *\nrecurring: false\nprompt: Coordinator heartbeat for `overnight`. Use the orchestrating-async-work skill.", result: "Scheduled 573b639b (31 11 10 10 *)", ms: 23_000),
            T.row(14, "tasks:turn:p2", ["Tasks": ["items": [
                ["subject": "Re-evaluate the plan", "status": "Completed"],
                ["subject": "Re-check the themes and lanes", "status": "InProgress"],
                ["subject": "Learn from the last check-in", "status": "Pending"],
            ]]]),
            prose(15, "prose:2", "Check-in at 09:34: nothing has changed. Main is green, the app is on build 2974, and no lanes are running. I've set the next check-in two hours out, at 11:31."),
        ]
    }

    static func window(width: CGFloat, height: CGFloat, _ view: some View) -> NSWindow {
        let window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: view.frame(width: width, height: height))
        window.makeKeyAndOrderFront(nil)
        return window
    }

    /// Where each probed view drew, by id, in the content's own coordinates.
    final class Frames {
        var views: [String: CGRect] = [:]
    }

    struct Framed<Content: View>: View {
        let frames: Frames
        let content: Content
        var body: some View {
            content
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = frames.views = Dictionary(probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                }
        }
    }

    /// Clicks the middle of the view probed as `id`, as a pointer would.
    @discardableResult
    static func click(_ id: String, frames: Frames, in window: NSWindow) -> Bool {
        guard let frame = frames.views[id], let height = window.contentView?.bounds.height else { return false }
        let at = NSPoint(x: frame.minX + 30, y: height - frame.midY)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            window.sendEvent(
                NSEvent.mouseEvent(
                    with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
        }
        return true
    }

    @Test("The text leads: a scheduled task says so, a run of calls is one line, a task list is a checklist")
    func theTextLeads() async throws {
        let model = T.model(try T.terminal())
        let frames = Frames()
        let window = Self.window(width: 720, height: 2_400, Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        defer { window.close() }
        model.store.apply(try await model.store.ledger.page(T.page(Self.rows())))
        await T.settle(window, 300)
        var seen: Set<String> { Set(frames.views.keys) }

        #expect(seen.contains("native-scheduled-turn"), "a scheduled task says it is one")
        #expect(!seen.contains("native-notice-turn"), "and is no notice wall")
        #expect(seen.contains("native-tool-group"), "three commands in a row are one line")
        #expect(!seen.contains("native-tool-tool:b1"), "its calls are inside it until it opens")
        #expect(seen.contains("native-tasks"))
        #expect(seen.contains("native-show-more"), "the long message folds")
        #expect(seen.contains("native-queued"))

        #expect(Self.click("section-header-toolgroup.tools:tool:b1", frames: frames, in: window), "the group opens")
        await T.settle(window, 600)
        #expect(seen.contains("native-tool-tool:b1"))
        #expect(Self.click("section-header-tool.tool:b1", frames: frames, in: window), "and a call in it opens in turn")
        await T.settle(window, 600)
        #expect(seen.contains("native-tool-detail"), "to its input and result")
    }
}

extension NativePolishTests {
    /// A 410-character message on one line: six lines or fewer in a wide
    /// pane, more in a 420 pt one, where it has to fold. A guess from its
    /// length (90 characters a line) never folded it (ov-452 review).
    @Test("A long one-line message folds in a narrow pane and not in a wide one")
    func aLongLineFoldsWhereItWraps() async throws {
        let line = String(repeating: "Fold this where it wraps past six lines. ", count: 10)
        for (width, folds) in [(CGFloat(420), true), (CGFloat(1_400), false)] {
            let model = T.model(try T.terminal())
            let frames = Frames()
            let window = Self.window(width: width, height: 900, Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
            model.store.apply(try await model.store.ledger.page(T.page([Self.turn(0, "turn:p1", line, origin: "Typed", ms: 1_000)])))
            await T.settle(window, 300)
            #expect(frames.views.keys.contains("native-show-more") == folds, "at \(Int(width)) pt")
            window.close()
        }
    }
}

/// Opt-in (FARCOOLER_CAPTURE_OUT): the same rows at a narrow and a wide
/// width, light and dark, folded and opened, for the polish review.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct NativePolishCaptures {
    @Test func conversationTextFirst() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (width, size) in [(CGFloat(420), "narrow"), (CGFloat(1_000), "wide")] {
            for (name, appearance) in RealWindowCaptures.variants.prefix(2) {
                for opened in [false, true] {
                    let model = NativeAgentTests.model(try NativeAgentTests.terminal())
                    model.store.apply(try await model.store.ledger.page(NativeAgentTests.page(NativePolishTests.rows())))
                    let frames = NativePolishTests.Frames()
                    let window = NativePolishTests.window(
                        width: width, height: 1_500,
                        NativePolishTests.Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
                    window.appearance = NSAppearance(named: appearance)
                    await NativeAgentTests.settle(window, 300)
                    if opened {
                        for id in ["toolgroup.tools:tool:b1", "toolgroup.tools:tool:c1", "tool.tool:b1", "tool.tool:c2"] {
                            NativePolishTests.click("section-header-\(id)", frames: frames, in: window)
                            await NativeAgentTests.settle(window, 600)
                        }
                    }
                    let image = try #require(RealWindowCaptures.windowImage(window), "screencapture -l isn't allowed here")
                    try #require(image.representation(using: .png, properties: [:]))
                        .write(to: URL(fileURLWithPath: out).appendingPathComponent("mac-\(size)-\(name)\(opened ? "-opened" : "").png"))
                    window.close()
                }
            }
        }
    }
}
