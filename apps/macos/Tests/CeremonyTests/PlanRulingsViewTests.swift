import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Decided For You on the Mac (ov-304): the canvas's section, read from the
/// CLI's real `plan --json` shape (`test/fixtures/plan.json`) through the
/// store's own read. Only the open rulings show; the kept and reversed ones
/// fold into Past Decisions (ov-333). Keep, Reverse and Discuss act on an open
/// one; a runner without `board_rulings` shows nothing.
@MainActor
@Suite(.serialized)
struct PlanRulingsViewTests {
    struct Hosted: View {
        @ObservedObject var plan: PlanStore
        let seen: NavigatorFilterTests.Seen
        let copy: @MainActor (String) -> Void
        var actions = PlanRulingActions.none
        var defaults = PlanViewTests.defaults()

        var body: some View {
            PlanRulingsSection(plan: plan, defaults: defaults, copy: copy)
            .environment(\.planRulingActions, actions)
            .frame(width: 520, height: 600, alignment: .topLeading)
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                GeometryReader { proxy in
                    let _ = seen.views = Dictionary(
                        probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                    Color.clear
                }
            }
        }
    }

    final class Drawn {
        let seen = NavigatorFilterTests.Seen()
        var copied: [String] = []
        var host: NSHostingView<Hosted>!
        var window: NavigatorFilterTests.KeyWindow!

        @MainActor func settle() async {
            for _ in 0..<15 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
    }

    /// The section as the canvas draws it, from a store that read the
    /// fixture through its stubbed CLI, on a runner that keeps rulings or not.
    static func draw(
        rulings: Bool, actions: PlanRulingActions = .none, pastOpen: Bool = false, extraOpen: Bool = false
    ) async throws -> Drawn {
        let defaults = PlanViewTests.defaults()
        let calls = PlanViewTests.Calls()
        if extraOpen {
            // A second open ruling, so the section offers Keep All.
            var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
            var list = try #require(object["rulings"] as? [[String: Any]])
            var more = list[0]
            more["id"] = "00000000-0000-0000-0000-000000005003"
            more["number"] = 3
            more["short"] = "R-3"
            more["decision"] = "The gutter is 12 points."
            list.insert(more, at: 0)
            object["rulings"] = list
            calls.plan = try JSONSerialization.data(withJSONObject: object)
        }
        let store = try await PlanViewTests.store(plan: true, defaults: defaults, calls: calls)
        if pastOpen {
            defaults.set(true, forKey: "board.plan.section.rulings.\(store.plan.host).\(store.plan.workspace.id).past")
        }
        if !rulings {
            store.client.daemonBuild = DaemonBuild(
                version: "test", matches: true, platform: "macos",
                capabilities: Set(Capability.allCases.map(\.rawValue).filter { $0 != "board_rulings" }))
        }
        await store.plan.reload()
        let drawn = Drawn()
        drawn.host = NSHostingView(
            rootView: Hosted(plan: store.plan, seen: drawn.seen, copy: { drawn.copied.append($0) }, actions: actions, defaults: defaults))
        drawn.window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 520, height: 600), styleMask: [.borderless],
            backing: .buffered, defer: false)
        drawn.window.isReleasedWhenClosed = false
        drawn.window.contentView = drawn.host
        drawn.window.makeKeyAndOrderFront(nil)
        await drawn.settle()
        return drawn
    }

    @Test("Only the open ruling shows, with its id and copy control; the settled one is folded away")
    func openOnly() async throws {
        let drawn = try await Self.draw(rulings: true)
        defer { drawn.window.close() }
        let views = drawn.seen.views
        #expect(views["plan-rulings"] != nil)
        #expect(views["plan-ruling-R-2"] != nil, "\(views.keys.sorted())")
        #expect(views["plan-ruling-R-2-copy"] != nil, "an open ruling offers Copy Reference")
        #expect(views["plan-ruling-R-1"] == nil, "a kept ruling isn't in Decided For You")
        #expect(views["plan-rulings-past"] != nil, "it's in Past Decisions")
    }

    @Test("Past Decisions is closed until opened, and opens onto the kept and reversed ones with their state")
    func theFoldHoldsTheRest() async throws {
        let closed = try await Self.draw(rulings: true)
        defer { closed.window.close() }
        #expect(closed.seen.views["plan-ruling-R-1"] == nil, "closed by default")
        let open = try await Self.draw(rulings: true, pastOpen: true)
        defer { open.window.close() }
        let views = open.seen.views
        let current = try #require(views["plan-ruling-R-2"])
        let past = try #require(views["plan-ruling-R-1"], "\(views.keys.sorted())")
        #expect(current.minY < past.minY, "open above past")
        #expect(past.height < current.height, "a past decision is one quiet line")
    }

    @Test("The owner's actions show on an open ruling when the runner takes them, and Keep calls Keep with that ruling")
    func keepActsOnItsRuling() async throws {
        var kept: [String] = []
        var asked: [String] = []
        var actions = PlanRulingActions(canKeep: true, canAsk: true, alwaysShown: true)
        actions.keep = { kept.append($0.short) }
        actions.reverse = { asked.append("reverse " + $0.short) }
        actions.discuss = { asked.append("discuss " + $0.short) }
        let drawn = try await Self.draw(rulings: true, actions: actions)
        defer { drawn.window.close() }
        for button in ["keep", "reverse", "discuss"] {
            #expect(drawn.seen.views["plan-ruling-R-2-\(button)"] != nil, "\(button): \(drawn.seen.views.keys.sorted())")
        }
        try Self.click(drawn, "plan-ruling-R-2-keep")
        await drawn.settle()
        #expect(kept == ["R-2"])
        #expect(asked.isEmpty, "Keep never reaches the orchestrator")
        try Self.click(drawn, "plan-ruling-R-2-discuss")
        #expect(asked == ["discuss R-2"])
        // Reverse asks first: a click opens the confirmation (a sheet, which
        // takes the clicks after it) and sends nothing.
        try Self.click(drawn, "plan-ruling-R-2-reverse")
        await drawn.settle()
        #expect(asked == ["discuss R-2"], "Reverse waits for its confirmation: \(asked)")
    }

    @Test("Without an orchestrator, Reverse and Discuss are off, never hidden; without the capability there are no actions")
    func offNotHidden() async throws {
        var asked = 0
        var actions = PlanRulingActions(canKeep: true, canAsk: false, alwaysShown: true)
        actions.reverse = { _ in asked += 1 }
        actions.discuss = { _ in asked += 1 }
        let drawn = try await Self.draw(rulings: true, actions: actions)
        defer { drawn.window.close() }
        #expect(drawn.seen.views["plan-ruling-R-2-discuss"] != nil)
        try Self.click(drawn, "plan-ruling-R-2-discuss")
        #expect(asked == 0, "a disabled Discuss sends nothing")
        // The control: the same click, at the same place, on an enabled one lands.
        var control = PlanRulingActions(canKeep: true, canAsk: true, alwaysShown: true)
        control.discuss = { _ in asked += 1 }
        let enabled = try await Self.draw(rulings: true, actions: control)
        defer { enabled.window.close() }
        try Self.click(enabled, "plan-ruling-R-2-discuss")
        #expect(asked == 1, "the click reaches an enabled Discuss, so the silence above is the disabling")
        let old = try await Self.draw(rulings: true, actions: PlanRulingActions(canKeep: false, canAsk: true, alwaysShown: true))
        defer { old.window.close() }
        #expect(old.seen.views["plan-ruling-R-2-keep"] == nil)
        #expect(old.seen.views["plan-ruling-R-2-copy"] != nil, "Copy Reference stays")
    }

    @Test("Keep All is on the section only when there's more than one to keep, and keeps them all")
    func keepAllNeedsMoreThanOne() async throws {
        var keptAll = 0
        var actions = PlanRulingActions(canKeep: true, canAsk: true)
        actions.keepAll = { keptAll += 1 }
        let one = try await Self.draw(rulings: true, actions: actions)
        defer { one.window.close() }
        #expect(one.seen.views["plan-rulings-keep-all"] == nil, "one open ruling has its own Keep")
        let two = try await Self.draw(rulings: true, actions: actions, extraOpen: true)
        defer { two.window.close() }
        try Self.click(two, "plan-rulings-keep-all")
        #expect(keptAll == 1)
    }

    static func click(_ drawn: Drawn, _ id: String) throws {
        let frame = try #require(drawn.seen.views[id], "\(id): \(drawn.seen.views.keys.sorted())")
        let at = NSPoint(x: frame.midX, y: 600 - frame.midY)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            drawn.window.sendEvent(
                NSEvent.mouseEvent(
                    with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: drawn.window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
        }
    }

    @Test("A runner without board_rulings shows no Decided For You, whatever the plan carries")
    func oldRunnerShowsNothing() async throws {
        let drawn = try await Self.draw(rulings: false)
        defer { drawn.window.close() }
        #expect(!drawn.seen.views.keys.contains { $0.hasPrefix("plan-ruling") }, "\(drawn.seen.views.keys.sorted())")
    }

    @Test("A board with only rulings keeps the board as it was: Unread, the rows, no plan layout (review 1005a F1)")
    func rulingsAloneDontPlanTheBoard() async throws {
        let defaults = PlanViewTests.defaults()
        let calls = PlanViewTests.Calls()
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        for key in ["themes", "lanes", "order", "cards"] { object[key] = [] }
        calls.plan = try JSONSerialization.data(withJSONObject: object)
        let store = try await PlanViewTests.store(plan: true, defaults: defaults, calls: calls)
        let drawn = await PlanViewTests.draw(store, defaults: defaults)
        defer { drawn.window.close() }
        for _ in 0..<20 where !store.plan.hasRead { await drawn.settle() }
        await drawn.settle()
        #expect(!store.plan.plan.rulings.isEmpty, "the rulings were read")
        #expect(!store.plan.planned, "rulings alone don't switch the layout")
        #expect(drawn.ids.contains("section-header-summary"), "Unread stays: \(drawn.ids)")
        #expect(drawn.ids.contains("board-row-ov-1"))
    }

    @Test("Copy Reference copies what the owner tells the orchestrator, and edits nothing")
    func copyReference() async throws {
        let drawn = try await Self.draw(rulings: true)
        defer { drawn.window.close() }
        let frame = try #require(drawn.seen.views["plan-ruling-R-2-copy"])
        let at = NSPoint(x: frame.midX, y: 600 - frame.midY)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            drawn.window.sendEvent(
                NSEvent.mouseEvent(
                    with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: drawn.window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
        }
        await drawn.settle()
        #expect(drawn.copied == ["ruling R-2: The inbox is amber."])
    }

    // MARK: Reverse asks first (ruling R-18)

    /// The dialog's own Reverse button, on the sheet a click on Reverse opened.
    private static func confirmButton(in window: NSWindow) -> NSButton? {
        func find(_ view: NSView) -> NSButton? {
            if let button = view as? NSButton, button.title == "Reverse" { return button }
            for child in view.subviews { if let found = find(child) { return found } }
            return nil
        }
        return window.attachedSheet?.contentView.flatMap(find)
    }

    @Test("Reverse sends the request only from the dialog's own Reverse button")
    func theDialogConfirmsAndSends() async throws {
        var sent: [String] = []
        var actions = PlanRulingActions(canKeep: true, canAsk: true, alwaysShown: true)
        actions.reverse = { sent.append($0.short) }
        let drawn = try await Self.draw(rulings: true, actions: actions)
        defer { drawn.window.close() }
        try Self.click(drawn, "plan-ruling-R-2-reverse")
        for _ in 0..<30 where drawn.window.attachedSheet == nil { await drawn.settle() }
        let confirm = try #require(Self.confirmButton(in: drawn.window), "no confirmation dialog opened")
        #expect(sent.isEmpty, "opening the dialog sends nothing")
        confirm.performClick(nil)
        for _ in 0..<30 where sent.isEmpty { await drawn.settle() }
        #expect(sent == ["R-2"], "confirming sends the reversal once")
    }

    /// Hosted on its own, the context menu's Reverse hands over to the row's
    /// confirmation and never sends itself: wiring it to `actions.reverse`
    /// goes red here.
    @Test("The context menu's Reverse goes to the confirmation, never straight to the orchestrator")
    func theMenuNeverSendsDirectly() async throws {
        var sent: [String] = []
        var asked = 0
        var actions = PlanRulingActions(canKeep: true, canAsk: true)
        actions.reverse = { sent.append($0.short) }
        let ruling = PlanRuling(id: "r2", number: 2, decision: "The inbox is amber.", why: "w", reversal: "r")
        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(
            rootView: RulingMenu(ruling: ruling, copy: { _ in }, onReverse: { asked += 1 })
                .environment(\.planRulingActions, actions)
                .frame(width: 200, height: 200)
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = seen.views = Dictionary(
                            probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                })
        let window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 200, height: 200), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }
        for _ in 0..<15 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        let frame = try #require(seen.views["ruling-menu-reverse"], "\(seen.views.keys.sorted())")
        let at = NSPoint(x: frame.midX, y: 200 - frame.midY)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            window.sendEvent(
                NSEvent.mouseEvent(
                    with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
        }
        try? await Task.sleep(for: .milliseconds(200))
        #expect(asked == 1, "the menu item asks for the confirmation")
        #expect(sent.isEmpty, "and sends nothing itself")
    }
}
