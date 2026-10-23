import XCTest

/// The agents at work (ov-619, ov-617), over the canned runner: Main's
/// orchestrator is claude with its conversation served (`-phone-native`) and
/// four subagents running (`-native-agents`), and Billing, led under
/// `-phone-billing-led`, has a lane's agent blocked in fc-3-webhooks.
///
/// No tray under the conversation. A compact Agents card beside Now/Next
/// counts them and opens the agents view, where a subagent opens its own
/// conversation in the orchestrator's place and a pane opens its worktree.
/// The workspace list's All Agents is the same view, across workspaces.
final class AgentsUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private static let mainOrchestrator = "0198f2c0-0000-7000-8000-00000000d001"
    private static let billingAgent = "0198f2c0-0000-7000-8000-00000000d002"

    private func launch(_ extra: [String] = ["-native-agents"]) -> XCUIApplication {
        .phoneHarness(["-phone-empty-inbox", "-phone-native"] + extra)
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func open(_ app: XCUIApplication, _ name: String) {
        let row = app.buttons["workspace-row-\(name)"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no \(name) row: \(app.debugDescription)")
        row.tap()
        XCTAssertTrue(app.navigationBars[name].waitForExistence(timeout: 10), "\(name) didn't open")
    }

    /// A subagent of Main's orchestrator, by its row in the conversation.
    private func subagent(_ app: XCUIApplication, _ row: String) -> XCUIElement {
        element(app, "agent-row-\(Self.mainOrchestrator)-\(row)")
    }

    /// **No tray under the conversation; the Agents card counts the
    /// subagents beside Now/Next**, the same height, and opens the view
    /// listing each with what it's doing.
    func testTheAgentsCardCountsAndOpensTheAgentsView() {
        let app = launch()
        open(app, "Main")
        XCTAssertTrue(element(app, "native-transcript").waitForExistence(timeout: 60), "the conversation never showed")
        let card = app.buttons["agents-card"], plan = app.buttons["plan-strip"]
        XCTAssertTrue(card.waitForExistence(timeout: 20), "no Agents card")
        XCTAssertEqual(card.value as? String, "4", "the card doesn't count the four running")
        XCTAssertEqual(card.label, "4 agents")
        XCTAssertEqual(card.frame.height, plan.frame.height, accuracy: 1, "the cards aren't one height")
        XCTAssertEqual(card.frame.midY, plan.frame.midY, accuracy: 1, "the cards aren't side by side")
        XCTAssertGreaterThan(card.frame.minX, plan.frame.maxX, "Agents isn't beside Now/Next")
        XCTAssertLessThan(card.frame.width, plan.frame.width / 2, "Agents isn't the compact one")
        XCTAssertFalse(element(app, "native-agent-tray").exists, "the tray is still under the conversation")

        card.tap()
        XCTAssertTrue(element(app, "agents-sheet").waitForExistence(timeout: 10), "no agents view")
        let a1 = subagent(app, "sub:a1")
        XCTAssertTrue(a1.waitForExistence(timeout: 10), "a1 isn't listed: \(app.debugDescription)")
        XCTAssertTrue(a1.label.contains("General purpose") && a1.label.contains("ov-452 conversation view hierarchy"), a1.label)
        for row in ["sub:a2", "sub:a3", "sub:a4"] {
            XCTAssertTrue(subagent(app, row).exists, "\(row) isn't listed")
        }
        XCTAssertFalse(subagent(app, "sub:a5").exists, "one that ended is listed")
        app.buttons["agents-done"].tap()
        XCTAssertTrue(element(app, "agents-sheet").waitForNonExistence(timeout: 10))
    }

    /// **Agents a turn left running are counted after it ends** (ov-619
    /// review): the runner names none of the four any more and still counts
    /// them, so the card says four, and the open conversation fills in each.
    func testAgentsATurnLeftRunningAreCountedAfterItEnds() {
        let app = launch(["-native-agents", "-native-agents-after-turn"])
        open(app, "Main")
        XCTAssertTrue(element(app, "native-transcript").waitForExistence(timeout: 60), "the conversation never showed")
        let card = app.buttons["agents-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 20), "the card collapsed with four running")
        XCTAssertEqual(card.label, "4 agents")
        card.tap()
        XCTAssertTrue(element(app, "agents-sheet").waitForExistence(timeout: 10), "no agents view")
        for row in ["sub:a1", "sub:a2", "sub:a3", "sub:a4"] {
            XCTAssertTrue(subagent(app, row).waitForExistence(timeout: 10), "\(row) isn't filled in from the conversation")
        }
        XCTAssertTrue(subagent(app, "sub:a1").label.contains("General purpose"), subagent(app, "sub:a1").label)
    }

    /// **A subagent's row opens its own conversation** in the orchestrator's
    /// place, under a header with a way back, and Back is the orchestrator's
    /// conversation again. Nothing is sent to an agent, so no composer.
    func testASubagentsRowOpensItsConversation() {
        let app = launch()
        open(app, "Main")
        let card = app.buttons["agents-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 60), "no Agents card: \(app.debugDescription)")
        card.tap()
        let a1 = subagent(app, "sub:a1")
        XCTAssertTrue(a1.waitForExistence(timeout: 10), "a1 isn't listed: \(app.debugDescription)")
        a1.tap()
        XCTAssertTrue(element(app, "native-agent-back").waitForExistence(timeout: 20), "a clear way back")
        XCTAssertTrue(element(app, "native-row-prose:agent-a1-1").waitForExistence(timeout: 20), "the agent's own words")
        XCTAssertFalse(element(app, "native-composer").exists, "nothing to send to an agent")
        // The rows start below the header and are cut there, so none draws
        // through its title (ov-467).
        let header = element(app, "native-agent-header").frame
        let rows = element(app, "native-agent-transcript").frame
        XCTAssertGreaterThanOrEqual(rows.minY, header.maxY - 1, "the rows \(rows) start below the header \(header)")

        element(app, "native-agent-back").tap()
        XCTAssertTrue(element(app, "native-composer").waitForExistence(timeout: 20))
        XCTAssertFalse(element(app, "native-agent-back").exists)
    }

    /// **A pane's row opens its worktree on that pane**, and Back is the
    /// workspace, one step from the list.
    func testAPanesRowOpensItsWorktree() {
        let app = launch(["-phone-billing-led"])
        open(app, "Billing")
        let card = app.buttons["agents-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 20), "no Agents card")
        XCTAssertEqual(card.value as? String, "1")
        card.tap()
        let agent = element(app, "agent-row-\(Self.billingAgent)")
        XCTAssertTrue(agent.waitForExistence(timeout: 10), "the lane's agent isn't listed: \(app.debugDescription)")
        agent.tap()
        let back = app.buttons["worktree-back"].firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 20), "the worktree didn't open")
        back.tap()
        XCTAssertTrue(app.navigationBars["Billing"].waitForExistence(timeout: 10), "Back isn't the workspace")
        app.tapBack()
        XCTAssertTrue(app.navigationBars["Needs You"].waitForExistence(timeout: 10), "the list isn't two Backs away")
    }

    /// **With none running the card goes**, and Now/Next takes the row.
    func testTheCardCollapsesWithNoneRunning() {
        let app = launch([])
        open(app, "Main")
        let plan = app.buttons["plan-strip"]
        XCTAssertTrue(plan.waitForExistence(timeout: 20), "no Now/Next card")
        Thread.sleep(forTimeInterval: 2)
        XCTAssertFalse(app.buttons["agents-card"].exists, "an Agents card with none running")
        let window = app.windows.firstMatch.frame
        XCTAssertGreaterThan(plan.frame.width, window.width - 48, "Now/Next doesn't span the row")
    }

    /// **All Agents lists everything, across workspaces** (ov-617): each
    /// workspace's orchestrator leading its own, Main's subagents, and
    /// Billing's lane agent; a row opens its agent, and Back comes back.
    func testAllAgentsListsEveryAgentAndOpensOne() {
        let app = launch(["-native-agents", "-phone-billing-led"])
        let row = app.buttons["all-agents-row"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no All Agents row")
        XCTAssertEqual(row.value as? String, "5 agents", "the row doesn't count every agent")
        row.tap()
        XCTAssertTrue(app.navigationBars["All Agents"].waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "agent-row-\(Self.mainOrchestrator)").exists, "Main's orchestrator isn't listed")
        XCTAssertTrue(element(app, "agent-row-0198f2c0-0000-7000-8000-00000000d004").exists, "Billing's orchestrator isn't listed")
        XCTAssertTrue(
            app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "agent-row-\(Self.mainOrchestrator)-")).count >= 4,
            "Main's subagents aren't listed")
        let agent = element(app, "agent-row-\(Self.billingAgent)")
        XCTAssertTrue(agent.exists, "Billing's agent isn't listed")
        XCTAssertTrue(
            app.staticTexts["overnight"].exists && app.staticTexts["overnight · Billing"].exists,
            "no section per workspace, each naming its repository")
        agent.tap()
        let back = app.buttons["worktree-back"].firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 20), "the worktree didn't open")
        back.tap()
        XCTAssertTrue(app.navigationBars["All Agents"].waitForExistence(timeout: 10), "Back isn't All Agents")
    }

    /// **Each repository's main checkout is headed by its repository** (ov-676):
    /// four repositories, four headers, and none of them "Main".
    func testAllAgentsNamesEachRepository() {
        let app = launch(["-native-agents", "-phone-many-repos"])
        let row = app.buttons["all-agents-row"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no All Agents row")
        row.tap()
        XCTAssertTrue(app.navigationBars["All Agents"].waitForExistence(timeout: 10))
        for name in [
            "overnight", "farcooler-relay-and-push-notification-service", "customer-billing-and-invoicing-platform",
            "next-js-storefront",
        ] {
            let header = app.staticTexts[name]
            XCTAssertTrue(app.reveal(header) || header.waitForExistence(timeout: 10), "no header for \(name)")
        }
        XCTAssertFalse(app.staticTexts["Main"].exists, "a header still says Main")
        let idle = element(app, "agent-row-0198f2c0-0000-7000-8000-000000004000")
        XCTAssertTrue(idle.waitForExistence(timeout: 10), "no idle orchestrator row")
        XCTAssertTrue(
            idle.label.contains("Minimize CI costs across every runner and every branch"),
            "the idle orchestrator's preview is cut: \(idle.label)")
    }

    /// A notification tapped while the app is open, as the harness takes one.
    private static func notify(_ which: String) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName("com.farcooler.harness.\(which)" as CFString), nil, nil, true)
    }

    /// **A runner that isn't answering says so in All Agents** (ov-619
    /// review): its agents' marks can't say what they're doing, and their
    /// rows say the runner isn't answering.
    func testAllAgentsSaysWhenARunnerIsntAnswering() {
        let app = launch(["-native-agents", "-phone-billing-led"])
        let row = app.buttons["all-agents-row"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no All Agents row")
        row.tap()
        let agent = element(app, "agent-row-\(Self.billingAgent)")
        XCTAssertTrue(agent.waitForExistence(timeout: 10), "Billing's agent isn't listed")
        XCTAssertEqual(agent.value as? String ?? "", "", "an answering runner's agent says it isn't")
        Self.notify("link-lost")
        let lost = NSPredicate(format: "value == %@", "Runner not answering")
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: lost, object: agent)], timeout: 10), .completed,
            "the row doesn't say its runner isn't answering: \(agent.value ?? "")")
    }

    /// **A notification while the agents view is up lands** (ov-619 review):
    /// the view goes, and the decision's task opens in the Plan sheet.
    func testANotificationWhileTheAgentsViewIsUpLands() {
        let app = launch(["-phone-billing-led"])
        open(app, "Billing")
        let card = app.buttons["agents-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 20), "no Agents card")
        card.tap()
        XCTAssertTrue(element(app, "agents-sheet").waitForExistence(timeout: 10), "no agents view")
        Self.notify("decision")
        XCTAssertTrue(element(app, "task-heading").waitForExistence(timeout: 15), "the decision opened nothing: \(app.debugDescription)")
        XCTAssertTrue(element(app, "plan-sheet").exists, "the task isn't in the Plan sheet")
        XCTAssertFalse(element(app, "agents-sheet").exists, "the agents view stayed up")
    }
}
