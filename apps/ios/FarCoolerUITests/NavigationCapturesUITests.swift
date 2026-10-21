import XCTest

/// Captures of the iPhone's navigation (ov-616, ov-619, ov-617) for a review:
/// a conversation under its two cards, the Agents view, the Plan sheet, a
/// workspace that is its Plan, All Agents, and a worktree with no shell bar,
/// light and dark, at the standard size and an accessibility size. Run it on
/// the narrowest and the widest iPhone the Mac has.
///
/// Opt-in: every method skips unless `FARCOOLER_CAPTURE_OUT`
/// (`TEST_RUNNER_FARCOOLER_CAPTURE_OUT`) names a directory, which gets
/// `ios-nav-<scene>-<appearance>-<size>.png`. It asserts only that each scene
/// came up.
final class NavigationCapturesUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private var out: URL? {
        ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"].map { URL(fileURLWithPath: $0) }
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func shoot(_ name: String, expecting state: CaptureState) {
        guard let out else { return }
        guard requireState(state, for: "ios-nav-\(name)") else { return }
        try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let url = out.appendingPathComponent("ios-nav-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    private static let sizes: [(String, [String])] = [
        ("standard", []),
        ("large", ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityL"]),
    ]

    private func each(_ body: (XCUIApplication, String) throws -> Void, extra: [String] = []) throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_CAPTURE_OUT")
        for appearance in ["light", "dark"] {
            for (size, flags) in Self.sizes {
                let app = XCUIApplication.phoneHarness(
                    ["-phone-empty-inbox", "-phone-native", "-native-agents", "-phone-appearance", appearance] + flags + extra)
                try body(app, "\(appearance)-\(size)")
                app.terminateRetrying()
            }
        }
    }

    private func open(_ app: XCUIApplication, _ workspace: String) {
        let row = app.buttons["workspace-row-\(workspace)"]
        XCTAssertTrue(app.reveal(row) || row.waitForExistence(timeout: 30), "no \(workspace) row")
        row.tap()
    }

    /// Main's conversation under Now/Next and Agents, then the agents view.
    func testConversationAndAgents() throws {
        try each { app, tag in
            open(app, "Main")
            let agents = app.buttons["agents-card"]
            XCTAssertTrue(agents.waitForExistence(timeout: 20), "no Agents card")
            Thread.sleep(forTimeInterval: 1)
            shoot("conversation-\(tag)", expecting: .shows(agents, "Main's conversation under the Agents card"))
            agents.tap()
            XCTAssertTrue(element(app, "agents-list").waitForExistence(timeout: 10), "no agents view")
            Thread.sleep(forTimeInterval: 1)
            shoot("agents-\(tag)", expecting: .shows(element(app, "agents-list")))
        }
    }

    /// `test/fixtures/plan-phones-sheet.json`: a plan with every section the
    /// sheet draws.
    private static var sheetPlan: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-phones-sheet.json").path
    }

    /// The Plan sheet at its medium detent, then at full height down
    /// through its sections: the tree, the trains landing, the lanes, the
    /// agents and their spend, cost and rulings.
    func testPlanSheet() throws {
        try each({ app, tag in
            open(app, "Billing")
            let card = app.buttons["plan-strip"]
            XCTAssertTrue(card.waitForExistence(timeout: 20), "no Now/Next card")
            shoot("billing-led-\(tag)", expecting: .shows(card, "the Now/Next card"))
            card.tap()
            let bar = app.navigationBars["Plan"]
            XCTAssertTrue(bar.waitForExistence(timeout: 10))
            Thread.sleep(forTimeInterval: 1)
            shoot("plan-medium-\(tag)", expecting: .when("the Plan sheet at its medium detent") {
                bar.exists && bar.frame.minY >= app.windows.firstMatch.frame.height * 0.2
            })
            // Dragged by its bar to the top, again if a loaded simulator lost
            // the first drag: the large detent, the sheet's bar near the top.
            let large = NSPredicate { _, _ in bar.exists && bar.frame.minY < app.windows.firstMatch.frame.height * 0.2 }
            for _ in 0..<3 where !(bar.exists && bar.frame.minY < app.windows.firstMatch.frame.height * 0.2) {
                bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
                    .press(forDuration: 0.1, thenDragTo: app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02)))
                Thread.sleep(forTimeInterval: 1)
            }
            XCTAssertEqual(
                XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: large, object: nil)], timeout: 10), .completed,
                "the sheet didn't reach its large detent")
            Thread.sleep(forTimeInterval: 1)
            shoot("plan-large-\(tag)", expecting: .when("the Plan sheet at its large detent") {
                bar.exists && bar.frame.minY < app.windows.firstMatch.frame.height * 0.2
            })
            for (section, name) in [("plan-landing", "landing"), ("plan-agents", "agents"), ("plan-cost-header", "cost"), ("plan-rulings", "rulings")] {
                let target = element(app, section)
                XCTAssertTrue(app.reveal(target), "no \(section) in the sheet")
                // Drawn, then brought up under the sheet's bar. The drag starts
                // no lower than 80% down the window: one begun at the bottom
                // edge is the home gesture, and leaves the app.
                let height = app.windows.firstMatch.frame.height
                if target.frame.minY > height * 0.3 {
                    let startY = min(target.frame.midY, height * 0.8)
                    let rise = target.frame.minY - height * 0.18
                    let window = app.windows.firstMatch.coordinate(withNormalizedOffset: .zero)
                    window.withOffset(CGVector(dx: app.windows.firstMatch.frame.width / 2, dy: startY))
                        .press(forDuration: 0.05, thenDragTo: window.withOffset(CGVector(dx: app.windows.firstMatch.frame.width / 2, dy: max(startY - rise, height * 0.1))))
                }
                Thread.sleep(forTimeInterval: 1)
                shoot("plan-large-\(name)-\(tag)", expecting: .when("\(section) brought up under the sheet's bar") {
                    target.exists && target.frame.minY < app.windows.firstMatch.frame.height * 0.35
                })
            }
        }, extra: ["-phone-billing-led", "-phone-plan", "-phone-agents", "-phone-rulings", "-phone-plan-file", Self.sheetPlan])
    }

    /// A workspace with no orchestrator: its Plan, with Start Orchestrator.
    func testWorkspaceThatIsItsPlan() throws {
        try each { app, tag in
            open(app, "Billing")
            XCTAssertTrue(app.buttons["start-orchestrator"].waitForExistence(timeout: 20))
            Thread.sleep(forTimeInterval: 1)
            shoot("billing-plan-\(tag)", expecting: .shows(app.buttons["start-orchestrator"]))
        }
    }

    /// Needs You with All Agents, and All Agents.
    func testAllAgents() throws {
        try each { app, tag in
            let row = app.buttons["all-agents-row"]
            XCTAssertTrue(row.waitForExistence(timeout: 30), "no All Agents row")
            Thread.sleep(forTimeInterval: 1)
            shoot("needs-you-\(tag)", expecting: .shows(row, "the All Agents row"))
            row.tap()
            XCTAssertTrue(app.navigationBars["All Agents"].waitForExistence(timeout: 10))
            Thread.sleep(forTimeInterval: 1)
            shoot("all-agents-\(tag)", expecting: .shows(app.navigationBars["All Agents"]))
        }
    }

    /// All Agents across four repositories with long names (ov-676).
    func testAllAgentsManyRepositories() throws {
        try each(
            { app, tag in
                let row = app.buttons["all-agents-row"]
                XCTAssertTrue(row.waitForExistence(timeout: 30), "no All Agents row")
                row.tap()
                XCTAssertTrue(app.navigationBars["All Agents"].waitForExistence(timeout: 10))
                Thread.sleep(forTimeInterval: 1)
                shoot("all-agents-repos-\(tag)", expecting: .shows(app.navigationBars["All Agents"]))
            }, extra: ["-phone-many-repos"])
    }

    /// Needs You across repositories whose main checkouts are all "Main" (ov-676).
    func testNeedsYouManyRepositories() throws {
        try XCTSkipIf(out == nil, "capture-only; set TEST_RUNNER_FARCOOLER_CAPTURE_OUT")
        let app = XCUIApplication.phoneHarness(["-phone-native", "-phone-many-repos", "-phone-appearance", "light"])
        let row = app.buttons["all-agents-row"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no All Agents row")
        let relay = app.staticTexts.matching(
            NSPredicate(format: "label BEGINSWITH 'farcooler-relay-and-push-notification-service'")
        ).firstMatch
        XCTAssertTrue(relay.waitForExistence(timeout: 10), "no item names the relay repository: \(app.debugDescription)")
        XCTAssertFalse(
            app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Main · '")).firstMatch.exists, "an item still says Main")
        Thread.sleep(forTimeInterval: 1)
        shoot("needs-you-repos", expecting: .shows(relay, "an item's repository"))
        app.terminateRetrying()
    }

    /// A worktree, opened from All Agents: its pane, and Back, with no bar
    /// at the bottom.
    func testWorktree() throws {
        try each { app, tag in
            let row = app.buttons["all-agents-row"]
            XCTAssertTrue(row.waitForExistence(timeout: 30), "no All Agents row")
            row.tap()
            let agent = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'agent-row-0198f2c0-0000-7000-8000-00000000d002'")).firstMatch
            XCTAssertTrue(app.reveal(agent) || agent.waitForExistence(timeout: 10), "no Billing agent")
            agent.tap()
            XCTAssertTrue(app.buttons["worktree-back"].firstMatch.waitForExistence(timeout: 20))
            Thread.sleep(forTimeInterval: 1.5)
            shoot("worktree-\(tag)", expecting: .shows(app.buttons["worktree-back"].firstMatch, "the worktree's back button"))
        }
    }
}
