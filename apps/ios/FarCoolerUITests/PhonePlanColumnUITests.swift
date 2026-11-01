import UIKit
import XCTest

/// The iPhone's Plan, drawn as the Mac's plan column is (ov-874): the facts
/// line, Needs You, Initiatives, the pipeline (Stuck, Building, Reviewing,
/// Landing, each agent a card with its tickets under it) and the footer
/// counts. What it says is AgentKit's `PlanColumn`, pinned in
/// `PlanColumnTests`; what is here is what only a screen can show: the order
/// a person meets it in, monospaced keys, one agent for each ticket, and what a
/// tap does.
///
/// Over the canned runner, whose plan is a real board's
/// (`test/fixtures/plan-seeded.json`), with agents (`-phone-agents`).
final class PhonePlanColumnUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-seeded.json").path
    }

    /// `plan-phones-ov317-themed.json`: eight tickets in Reviewing and no agent on any.
    private static var bare: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-phones-ov317-themed.json").path
    }

    /// The first match: an identifier on a container reaches its text too.
    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// Billing's Plan, its screen while it has no orchestrator (ov-616).
    private func openPlan(_ extra: [String] = [], fixture: String? = nil, ready: String = "plan-facts") -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan", "-phone-agents", "-phone-plan-file", fixture ?? Self.fixture]
                + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30) || app.reveal(billing), "no Billing row")
        billing.tap()
        // Under the invitation to start an orchestrator, which at a large type size is a screen tall.
        XCTAssertTrue(app.reveal(element(app, ready)), "no Plan: \(app.debugDescription)")
        return app
    }

    private func scrollDown(_ app: XCUIApplication) {
        let window = app.windows.firstMatch
        window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.7))
            .press(forDuration: 0.05, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45)))
    }

    /// Scroll down until `id` can be touched.
    @discardableResult
    private func reach(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        let target = element(app, id)
        let window = app.windows.firstMatch.frame
        func shown() -> Bool { target.exists && target.frame.minY > window.height * 0.15 && target.frame.maxY < window.height * 0.92 }
        for _ in 0..<60 where !shown() { scrollDown(app) }
        XCTAssertTrue(shown(), "can't reach \(id): \(app.debugDescription)")
        return target
    }

    /// What a pass down the whole Plan met, and in what order.
    private struct Walk {
        /// Each marker the first time it was seen, in the order seen.
        var order: [String] = []
        /// Each ticket's key, with the agents it was seen under.
        var agentsOf: [String: Set<String>] = [:]
        var keyWidths: [String: CGFloat] = [:]
        /// How many tickets were seen, and how many of them with their card in view.
        var tickets = Set<String>()
    }

    private static let markers = [
        "plan-facts", "plan-needs-you", "plan-initiatives", "plan-pipeline", "plan-stage-stuck", "plan-stage-building",
        "plan-stage-reviewing", "plan-stage-landing", "plan-footer",
    ]

    /// Scroll the Plan top to bottom, noting the headings in the order they
    /// first come up, and which agent each ticket is under (the agent whose
    /// card is the nearest above it on the screen).
    private func walk(_ app: XCUIApplication) -> Walk {
        var walk = Walk()
        func look() {
            let found = Self.markers.compactMap { id -> (String, CGFloat)? in
                let e = element(app, id)
                return e.exists ? (id, e.frame.minY) : nil
            }
            for (id, _) in found.sorted(by: { $0.1 < $1.1 }) where !walk.order.contains(id) { walk.order.append(id) }
            let rows = app.descendants(matching: .any).matching(
                NSPredicate(format: "identifier BEGINSWITH 'plan-agent-' OR identifier BEGINSWITH 'plan-ticket-'"))
            let seen = rows.allElementsBoundByIndex.filter { $0.exists }.map { ($0.identifier, $0.frame) }
                .filter { !$0.0.hasPrefix("plan-agent-glyph-") && !$0.0.hasPrefix("plan-ticket-key-") }
                .sorted { $0.1.minY < $1.1.minY }
            // Only the agent on the same screen, above it: a card scrolled off is never guessed.
            var agent: String?
            for (id, _) in seen {
                if id.hasPrefix("plan-agent-") {
                    agent = id
                } else {
                    let key = String(id.dropFirst("plan-ticket-".count))
                    walk.tickets.insert(key)
                    if let agent { walk.agentsOf[key, default: []].insert(agent) }
                }
            }
            // Keys, with a widest text that has none the agents can differ in.
            for key in walk.agentsOf.keys where walk.keyWidths[key] == nil {
                let e = element(app, "plan-ticket-key-\(key)")
                if e.exists, e.frame.width > 0 { walk.keyWidths[key] = e.frame.width }
            }
        }
        for _ in 0..<120 {
            look()
            if walk.order.contains("plan-footer") { break }
            scrollDown(app)
        }
        look()
        return walk
    }

    // MARK: Order

    /// **The Plan reads in the order the owner thinks of it**: the facts, what needs
    /// them, the initiatives, then the pipeline from Stuck down to Landing, then the
    /// counts.
    func testThePlanReadsInOrder() {
        let app = openPlan()
        let walk = walk(app)
        let expected = Self.markers.filter { walk.order.contains($0) }
        XCTAssertEqual(walk.order, expected, "the Plan is out of order")
        for id in ["plan-facts", "plan-initiatives", "plan-pipeline", "plan-stage-building", "plan-stage-landing", "plan-footer"] {
            XCTAssertTrue(walk.order.contains(id), "no \(id) in the Plan: \(walk.order)")
        }
        // Stuck leads the stages it has.
        let stages = walk.order.filter { $0.hasPrefix("plan-stage-") }
        let wanted = ["stuck", "building", "reviewing", "landing"].map { "plan-stage-\($0)" }.filter { stages.contains($0) }
        XCTAssertEqual(stages, wanted, "the stages aren't Stuck, Building, Reviewing, Landing")
        // No Stop, Assign or cost anywhere in it.
        for word in ["Stop", "Assign"] { XCTAssertFalse(app.buttons[word].exists, "a \(word) button in the Plan") }
        XCTAssertFalse(element(app, "plan-cost-header").exists, "cost in the Plan")
    }

    /// **A ticket is on one agent**, and each key is set in monospace: its text is as wide as
    /// the monospaced font says and not as the proportional one does.
    func testEachTicketIsOnOneAgentAndItsKeyIsMonospaced() {
        let app = openPlan()
        let walk = walk(app)
        XCTAssertFalse(walk.agentsOf.isEmpty, "no tickets under any agent: \(walk.order)")
        for (key, agents) in walk.agentsOf { XCTAssertEqual(agents.count, 1, "\(key) is under \(agents.sorted())") }
        // Most tickets were seen under their card, so the check above looked at them.
        XCTAssertGreaterThanOrEqual(walk.agentsOf.count * 2, walk.tickets.count, "few tickets were seen under an agent")
        let style = UIFont.preferredFont(forTextStyle: .footnote)
        let mono = UIFont.monospacedSystemFont(ofSize: style.pointSize, weight: .regular)
        XCTAssertFalse(walk.keyWidths.isEmpty, "no key was measured")
        for (key, width) in walk.keyWidths {
            let monoWidth = (key as NSString).size(withAttributes: [.font: mono]).width
            let plainWidth = (key as NSString).size(withAttributes: [.font: style]).width
            XCTAssertEqual(width, monoWidth, accuracy: 3, "\(key) isn't in monospace (\(width) wide; mono \(monoWidth), plain \(plainWidth))")
            XCTAssertGreaterThan(abs(monoWidth - plainWidth), 1, "the check can't tell monospace from the system font for \(key)")
        }
    }

    // MARK: Taps

    /// **A ticket opens the task screen.**
    func testATicketOpensItsTask() {
        let app = openPlan()
        let ticket = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'plan-ticket-ov-'")).firstMatch
        XCTAssertTrue(app.reveal(ticket), "no ticket: \(app.debugDescription)")
        let id = ticket.identifier
        XCTAssertFalse(id.hasPrefix("plan-ticket-key-"), "the first ticket found was a key: \(id)")
        reach(app, id).tap()
        XCTAssertTrue(element(app, "task-screen").waitForExistence(timeout: 10), "a ticket didn't open its task: \(app.debugDescription)")
    }

    /// **Each footer count opens the list of its tickets, and a ticket in it opens its task**
    /// (ov-874): a ticket whose initiative is quiet or absent is reached here, and keeps its
    /// page and its Mark Ready.
    func testEachFooterCountOpensItsTickets() {
        var seen = 0
        for kind in ["waiting", "notStarted", "done"] {
            let app = openPlan()
            let count = element(app, "plan-footer-\(kind)")
            guard app.reveal(count) else { app.terminate(); continue }
            seen += 1
            reach(app, count.identifier).tap()
            XCTAssertTrue(element(app, "board-history").waitForExistence(timeout: 10), "\(kind) opened no list: \(app.debugDescription)")
            let row = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'history-row-'")).firstMatch
            XCTAssertTrue(row.waitForExistence(timeout: 10), "\(kind)'s list has no tickets: \(app.debugDescription)")
            row.tap()
            XCTAssertTrue(element(app, "task-screen").waitForExistence(timeout: 10), "\(kind): a ticket didn't open its task")
            app.terminate()
        }
        XCTAssertGreaterThan(seen, 0, "the fixture has no footer counts")
    }

    /// **A stage nobody is on shows its first three tickets and a Show All that opens the rest**
    /// (ov-874): the line is a button, and a row in the list opens its task.
    func testTheMoreLineOpensItsListAndARowOpensItsTask() {
        let app = openPlan(fixture: Self.bare, ready: "plan-pipeline")
        let all = element(app, "plan-more-all-reviewing")
        XCTAssertTrue(app.reveal(all), "no Show All under Reviewing: \(app.debugDescription)")
        let tickets = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier BEGINSWITH 'plan-ticket-ov-'"))
        XCTAssertEqual(tickets.count, 3, "the stage draws three of its eight tickets: \(app.debugDescription)")
        reach(app, all.identifier).tap()
        XCTAssertTrue(element(app, "board-history").waitForExistence(timeout: 10), "Show All opened no list: \(app.debugDescription)")
        let rows = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'history-row-'"))
        XCTAssertTrue(rows.firstMatch.waitForExistence(timeout: 10), "the list has no tickets")
        XCTAssertEqual(rows.count, 8, "all eight are in the list")
        rows.element(boundBy: 5).tap()
        XCTAssertTrue(element(app, "task-screen").waitForExistence(timeout: 10), "a row didn't open its task: \(app.debugDescription)")
    }

    /// **"N more waiting for review" under an agent's card is a button** (ov-874): the tickets
    /// nobody is on are reached from it.
    func testAnAgentsMoreLineOpensItsList() throws {
        // The seeded plan with one reviewing ticket taken off every agent: nobody is on it.
        var plan = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: Self.fixture))) as? [String: Any])
        var inner = try XCTUnwrap(plan["plan"] as? [String: Any])
        let lines = try XCTUnwrap(inner["lines"] as? [[String: Any]])
        let loose = try XCTUnwrap(lines.first { $0["stage"] as? String == "reviewing" }?["task"] as? String)
        let agents = try XCTUnwrap(inner["agents"] as? [[String: Any]])
        inner["agents"] = agents.map { agent -> [String: Any] in
            var agent = agent
            agent["tickets"] = (agent["tickets"] as? [[String: Any]] ?? []).filter { $0["task"] as? String != loose }
            return agent
        }
        plan["plan"] = inner
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("plan-loose-\(UUID().uuidString).json")
        try JSONSerialization.data(withJSONObject: plan).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        let app = openPlan(fixture: file.path)
        let more = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier BEGINSWITH 'plan-more-agent:'")).firstMatch
        XCTAssertTrue(app.reveal(more), "no more line under an agent: \(app.debugDescription)")
        reach(app, more.identifier).tap()
        XCTAssertTrue(element(app, "board-history").waitForExistence(timeout: 10), "the more line opened no list: \(app.debugDescription)")
        let row = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'history-row-'")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "the list has no tickets")
        row.tap()
        XCTAssertTrue(element(app, "task-screen").waitForExistence(timeout: 10), "a row didn't open its task: \(app.debugDescription)")
    }

    /// **A runner with no plan keeps its old screen** (ov-874), not the column.
    func testARunnerWithNoPlanKeepsItsOldScreen() {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox", "-phone-board-reads"])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30) || app.reveal(billing), "no Billing row")
        billing.tap()
        XCTAssertTrue(element(app, "workspace-body").waitForExistence(timeout: 30), "no workspace screen: \(app.debugDescription)")
        XCTAssertFalse(element(app, "plan-facts").exists || element(app, "plan-pipeline").exists, "the column drew on a runner with no plan")
    }

    /// **An agent with no pane opens its first ticket**; one with a pane opens its chat.
    func testAnAgentOpensItsTicketWhenItHasNoPane() {
        let app = openPlan()
        let agent = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier BEGINSWITH 'plan-agent-agent:'")).firstMatch
        XCTAssertTrue(app.reveal(agent), "no agent: \(app.debugDescription)")
        reach(app, agent.identifier).tap()
        XCTAssertTrue(element(app, "task-screen").waitForExistence(timeout: 10), "an agent opened nothing: \(app.debugDescription)")
    }

    /// **An agent with a pane opens its chat**: `plan-seeded-pane.json` is the seeded plan with its
    /// first builder on the harness's agent terminal, whose worktree opens on that pane.
    func testAnAgentWithAPaneOpensItsChat() {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        let pane = root.appendingPathComponent("test/fixtures/plan-seeded-pane.json").path
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan", "-phone-agents", "-phone-plan-file", pane])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        let agent = element(app, "plan-agent-agent:a7c31e0f2b9d4e18")
        XCTAssertTrue(app.reveal(agent), "no builder: \(app.debugDescription)")
        reach(app, agent.identifier).tap()
        XCTAssertTrue(app.buttons["worktree-back"].firstMatch.waitForExistence(timeout: 20), "the agent's pane didn't open: \(app.debugDescription)")
        XCTAssertFalse(element(app, "task-screen").exists, "it opened the ticket and not the chat")
    }

    /// **Needs You leaves its words in the orchestrator's composer and sends nothing**: a draft
    /// pasted without Enter, then the sheet is closed onto the conversation.
    func testANeedsYouRowDraftsIntoTheOrchestrator() {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-billing-led", "-phone-board-reads", "-phone-plan", "-phone-agents", "-phone-plan-file", Self.fixture])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 30), "no strip")
        strip.tap()
        let ask = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'plan-needs-theme-'")).firstMatch
        XCTAssertTrue(ask.waitForExistence(timeout: 30), "no Needs You row: \(app.debugDescription)")
        ask.tap()
        let lines: () -> [String] = {
            (self.element(app, "harness-sent").value as? String).map { $0.split(separator: "\n").map(String.init) } ?? []
        }
        let drafted = NSPredicate { _, _ in lines().contains { $0.hasPrefix("draft billing") } }
        expectation(for: drafted, evaluatedWith: nil)
        waitForExpectations(timeout: 10)
        XCTAssertFalse(lines().contains { $0.hasPrefix("send billing") || $0.contains("\r") }, "the draft was sent: \(lines())")
        XCTAssertTrue(app.navigationBars["Plan"].waitForNonExistence(timeout: 10), "the Plan stayed over the conversation")
    }

    // MARK: Type size

    /// **At the largest accessibility size nothing runs off the screen's edge**, the
    /// agent's name and what it does wrap, and a ticket's key keeps its own line's room.
    func testItHoldsAtTheLargestAccessibilityTypeSize() {
        let app = openPlan(["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
        let width = app.windows.firstMatch.frame.width
        let walk = walk(app)
        XCTAssertTrue(walk.order.contains("plan-footer"), "never reached the end: \(walk.order)")
        for key in walk.keyWidths.keys {
            let e = element(app, "plan-ticket-key-\(key)")
            if e.exists { XCTAssertLessThanOrEqual(e.frame.maxX, width, "\(key) runs off the screen") }
        }
    }

    // MARK: Captures

    /// Light and dark, kept as attachments and, with `TEST_RUNNER_FARCOOLER_CAPTURE_OUT`, as files.
    /// Off unless `TEST_RUNNER_FC_CAPTURES=1`.
    func testCaptures() throws {
        let environment = ProcessInfo.processInfo.environment
        try XCTSkipUnless(environment["FC_CAPTURES"] == "1", "capture-only; set TEST_RUNNER_FC_CAPTURES=1")
        let out = environment["FARCOOLER_CAPTURE_OUT"].map { URL(fileURLWithPath: $0) }
        for (name, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            let app = openPlan(["-phone-appearance", name])
            for (index, id) in ["plan-facts", "plan-stage-building", "plan-stage-landing", "plan-footer", "plan-bottom"].enumerated() {
                if id == "plan-bottom" {
                    // Past the footer: Pages and the loose worktrees.
                    for _ in 0..<12 { app.windows.firstMatch.swipeUp() }
                } else if id == "plan-footer" {
                    XCTAssertTrue(app.reveal(element(app, id)), "no footer")
                    for _ in 0..<3 { app.windows.firstMatch.swipeUp() }
                } else if index > 0 {
                    reach(app, id)
                }
                Thread.sleep(forTimeInterval: 1)
                let shot = app.screenshot()
                if let out {
                    try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
                    try shot.pngRepresentation.write(to: out.appendingPathComponent("plan-phone-\(index)-\(name).png"))
                }
                let attachment = XCTAttachment(screenshot: shot)
                attachment.name = "plan-phone-\(index)-\(name)"
                attachment.lifetime = .keepAlways
                add(attachment)
            }
            app.terminate()
            // A stage nobody is on: three tickets and Show All; and the largest type size.
            func shoot(_ app: XCUIApplication, _ file: String) throws {
                Thread.sleep(forTimeInterval: 1)
                let shot = app.screenshot()
                if let out {
                    try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
                    try shot.pngRepresentation.write(to: out.appendingPathComponent("\(file).png"))
                }
                let attachment = XCTAttachment(screenshot: shot)
                attachment.name = file
                attachment.lifetime = .keepAlways
                add(attachment)
            }
            let bare = openPlan(["-phone-appearance", name], fixture: Self.bare, ready: "plan-pipeline")
            reach(bare, "plan-more-all-reviewing")
            try shoot(bare, "plan-phone-more-\(name)")
            reach(bare, "plan-more-all-reviewing").tap()
            XCTAssertTrue(element(bare, "board-history").waitForExistence(timeout: 10))
            try shoot(bare, "plan-phone-stage-list-\(name)")
            bare.terminate()
            let big = openPlan(["-phone-appearance", name, "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"], fixture: Self.bare, ready: "plan-pipeline")
            reach(big, "plan-more-all-reviewing")
            try shoot(big, "plan-phone-more-xxxl-\(name)")
            big.terminate()
        }
    }
}
