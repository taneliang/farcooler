import XCTest

/// The plan layer on a phone (ov-274), over the canned runner, whose plan is a
/// real board's: `test/fixtures/plan-seeded.json` is the CLI's own output for
/// the board `.claude/agent/reports/ov-273/seed.sh` seeds.
///
/// Only the wiring a unit test cannot reach (ov-538): a runner without
/// `board_plan` shows nothing new (the smoke test), a plan notice reads the
/// plan again, and a lane and a theme open their pages. What the Plan says
/// (the words of a lane, a train, a tile and the cost, the default segment, a
/// read that fails, which boards a notice reads) is AgentKit's, pinned in
/// `PlanPhoneBoardTests`.
final class PlanUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    /// `test/fixtures/plan-seeded.json`, from this file's own place.
    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-seeded.json").path
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func keep(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// Billing's board, on a runner that advertises `board_plan` unless
    /// `extra` says otherwise.
    private func openBoard(_ extra: [String] = ["-phone-plan"], fixture: String = PlanUITests.fixture) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan-file", fixture] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        app.buttons["segment-board"].tap()
        XCTAssertTrue(element(app, "board").waitForExistence(timeout: 10), "no board")
        return app
    }

    private func showPlan(_ app: XCUIApplication) {
        let control = app.segmentedControls["plan-switch"]
        XCTAssertTrue(control.waitForExistence(timeout: 10), "no Tasks | Plan control: \(app.debugDescription)")
        control.buttons["Plan"].tap()
    }

    func testARunnerWithoutBoardPlanShowsNothingNew() {
        let app = openBoard([])
        XCTAssertTrue(element(app, "board-section-needs_decision").waitForExistence(timeout: 10), "no tasks")
        XCTAssertFalse(app.segmentedControls["plan-switch"].exists, "a control on a runner with no plan")
        XCTAssertFalse(element(app, "plan-landing").exists)
    }

    /// A runner notice, as the harness takes one: a Darwin notification.
    private static func notify(_ which: String) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName("com.farcooler.harness.\(which)" as CFString), nil, nil, true)
    }

    private func sent(_ app: XCUIApplication) -> [String] {
        (element(app, "harness-sent").value as? String).map { $0.split(separator: "\n").map(String.init) } ?? []
    }

    /// A `plan` notice reaches the connection and the plan on screen is read again.
    func testAPlanNoticeReadsThePlanAgain() {
        let app = openBoard()
        showPlan(app)
        XCTAssertTrue(app.reveal(element(app, "plan-lane-plan-phones")))
        Self.notify("plan-news")
        XCTAssertTrue(app.reveal(element(app, "plan-lane-plan-phones-v2")), "the notice didn't re-read the plan")
    }

    /// A task notice moves counts the plan derives: it reads the plan again too.
    func testATaskNoticeReadsThePlanAgain() {
        let app = openBoard()
        showPlan(app)
        XCTAssertTrue(app.reveal(element(app, "plan-lane-plan-phones")))
        Self.notify("task-news")
        XCTAssertTrue(app.reveal(element(app, "plan-lane-plan-phones-v2")), "the task notice didn't re-read the plan")
    }

    /// A notice reads the record of the page on screen, not every page opened since.
    func testANoticeReadsOnlyThePageOnScreen() {
        let app = openBoard()
        showPlan(app)
        let lane = element(app, "plan-lane-plan-phones")
        XCTAssertTrue(app.reveal(lane))
        lane.tap()
        XCTAssertTrue(element(app, "plan-lane-page").waitForExistence(timeout: 10))
        element(app, "plan-lane-theme").tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10))
        let gets = sent(app).filter { $0 == "plan.get" }.count
        let records = sent(app).filter { $0.hasPrefix("plan.events") }.count
        XCTAssertEqual(records, 2, "one record per page opened: \(sent(app))")
        Self.notify("plan-news")
        let deadline = Date().addingTimeInterval(15)
        while sent(app).filter({ $0.hasPrefix("plan.events") }).count == records, Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.3))
        }
        RunLoop.current.run(until: Date().addingTimeInterval(2))
        let after = sent(app)
        XCTAssertGreaterThan(after.filter { $0 == "plan.get" }.count, gets, "the plan wasn't read again")
        XCTAssertEqual(after.filter { $0.hasPrefix("plan.events") }.count, records + 1, "more than the open page was read: \(after)")
        // Back twice returns to the plan, and the choice of Plan was not forgotten.
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        // The notice renamed the first Next Up lane, as a new plan does.
        XCTAssertTrue(element(app, "plan-lane-plan-phones-v2").waitForExistence(timeout: 10), "Back didn't return to the plan")
        for _ in 0..<30 where !app.segmentedControls["plan-switch"].exists { app.swipeDown(velocity: .slow) }
        XCTAssertTrue(app.segmentedControls["plan-switch"].buttons["Plan"].isSelected, "the choice was forgotten")
    }

    /// The sheets: the overview, its themes, a lane and a theme, light and dark.
    ///
    /// Opt-in, like the Android Roborazzi captures: it makes screenshots and
    /// asserts nothing a behavior test does not, and it flips the whole
    /// device's appearance, so it stays out of CI and of an ordinary run.
    /// `TEST_RUNNER_FC_CAPTURES=1 xcodebuild test …` turns it on.
    func testCaptures() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["FC_CAPTURES"] == "1",
            "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take the sheets")
        for (name, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            let app = openBoard()
            showPlan(app)
            XCTAssertTrue(element(app, "plan-landing").waitForExistence(timeout: 10))
            keep(app, "capture-overview-\(name)")
            let theme = element(app, "plan-theme-Visual language")
            for _ in 0..<8 where !(theme.exists && theme.isHittable) { app.swipeUp() }
            keep(app, "capture-themes-\(name)")
            theme.tap()
            XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10))
            keep(app, "capture-theme-page-\(name)")
            app.navigationBars.buttons["BackButton"].firstMatch.tap()
            let lane = element(app, "plan-lane-mac-vis")
            for _ in 0..<8 where !(lane.exists && lane.isHittable) { app.swipeDown() }
            lane.tap()
            XCTAssertTrue(element(app, "plan-lane-page").waitForExistence(timeout: 10))
            keep(app, "capture-lane-page-\(name)")
            app.terminateRetrying()
        }
    }
}
