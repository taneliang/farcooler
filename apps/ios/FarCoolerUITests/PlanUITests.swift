import XCTest

/// The plan layer on a phone (ov-274), over the canned runner, whose plan is a
/// real board's: `test/fixtures/plan-seeded.json` is the CLI's own output for
/// the board `.claude/agent/reports/ov-273/seed.sh` seeds.
///
/// Opt-in, so a runner without `board_plan` shows nothing new; Tasks the
/// default, with the control switching only the task list; each theme legible
/// on its own and each Next Up lane naming its theme; an outcome wrapping to
/// three lines; and a read that is refused or never answered says so, with Try
/// Again, instead of spinning. Each attaches a screenshot, kept, for the light
/// and dark sheets.
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

    /// `test/fixtures/plan-cost-seeded.json` (ov-307): `farcooler plan --json` from a
    /// scratch daemon whose theme and lane have budgets, whose agents have spent
    /// across days, and whose finished cards were worked by two harnesses.
    private static var costFixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-cost-seeded.json").path
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
        XCTAssertFalse(element(app, "plan-next-up").exists)
    }

    /// Plan was chosen, and then the runner stopped keeping a plan: the board
    /// is its tasks, and nothing of the plan is left.
    func testARunnerThatLostBoardPlanDrawsTasksWhateverWasChosen() {
        let app = openBoard(["-phone-plan-was-chosen"])
        XCTAssertTrue(element(app, "board-section-needs_decision").waitForExistence(timeout: 10), "no tasks")
        XCTAssertFalse(app.segmentedControls["plan-switch"].exists)
        XCTAssertFalse(element(app, "plan-next-up").exists, "the plan is up on a runner that keeps none")
    }

    func testTasksIsTheDefaultAndPlanSwitchesOnlyTheTaskList() {
        let app = openBoard()
        let control = app.segmentedControls["plan-switch"]
        XCTAssertTrue(control.waitForExistence(timeout: 10), "no control on a runner with a plan")
        XCTAssertTrue(element(app, "board-section-needs_decision").waitForExistence(timeout: 10), "no tasks by default")
        XCTAssertTrue(element(app, "board-unread").exists, "no Unread strip in Tasks")
        XCTAssertFalse(element(app, "plan-next-up").exists, "the plan is up by default")

        control.buttons["Plan"].tap()
        XCTAssertTrue(element(app, "plan-next-up").waitForExistence(timeout: 10), "no Next Up: \(app.debugDescription)")
        // The list is lazy: look down the whole of the plan for a task section.
        for _ in 0..<12 { app.swipeUp() }
        XCTAssertEqual(
            app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'board-section-'")).count, 0,
            "the task list stayed under the plan")
        for _ in 0..<12 { app.swipeDown() }
        XCTAssertFalse(element(app, "board-unread").exists, "Unread is above the plan, which is the first thing to see")
        XCTAssertTrue(control.exists, "the control went")
        keep(app, "plan-overview")

        control.buttons["Tasks"].tap()
        XCTAssertTrue(element(app, "board-section-needs_decision").waitForExistence(timeout: 10), "tasks didn't return")
        XCTAssertTrue(element(app, "board-unread").waitForExistence(timeout: 10), "Unread didn't come back with the tasks")
        XCTAssertFalse(element(app, "plan-next-up").exists)
    }

    /// Cost (ov-307), from the CLI's own bytes: the theme past its budget says so in
    /// the overview, the Cost section holds the week and the comparison with n and
    /// the pair held back, and the theme's page draws its spend, budget and trend.
    func testCostShowsOnThePlanAndTheThemePage() {
        let app = openBoard(fixture: Self.costFixture)
        showPlan(app)
        XCTAssertTrue(element(app, "plan-themes").waitForExistence(timeout: 10), "no Themes: \(app.debugDescription)")
        let over = app.descendants(matching: .any).matching(identifier: "plan-budget-over")
        XCTAssertTrue(over.firstMatch.waitForExistence(timeout: 10), "no theme or lane over budget is flagged")
        for _ in 0..<6 where !element(app, "plan-cost-header").isHittable { app.swipeUp() }
        XCTAssertTrue(element(app, "plan-cost-header").exists, "no Cost section")
        // By what is said, since a block that reads as one element has one label.
        func said(_ words: String) -> Bool {
            app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", words)).firstMatch.exists
        }
        XCTAssertTrue(said("Over budget"), "nothing says a theme or lane is over budget")
        XCTAssertTrue(said("tokens in the last 7 days"), "no week: \(app.debugDescription)")
        XCTAssertTrue(said("held back until three cards have finished"), "the pair with too few cards isn't said to be held back")
        XCTAssertTrue(said("4 finished cards"), "no comparison row with its n")
        keep(app, "plan-cost")
        for _ in 0..<6 { app.swipeDown() }
        let theme = element(app, "plan-theme-Invoices")
        XCTAssertTrue(theme.waitForExistence(timeout: 10))
        theme.tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10))
        for _ in 0..<4 where !element(app, "plan-theme-spend").isHittable { app.swipeUp() }
        XCTAssertTrue(element(app, "plan-theme-spend").exists, "no Spend on the theme page")
        XCTAssertTrue(element(app, "plan-trend").exists, "no trend")
        XCTAssertTrue(said("Over budget. 4.9 million of 2 million tokens used."), "the theme page doesn't say it's over budget, in words")
        keep(app, "plan-theme-spend")
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
        XCTAssertTrue(element(app, "plan-lane-plan-phones").waitForExistence(timeout: 10))
        Self.notify("plan-news")
        XCTAssertTrue(element(app, "plan-lane-plan-phones-v2").waitForExistence(timeout: 15), "the notice didn't re-read the plan")
    }

    /// A task notice moves counts the plan derives: it reads the plan again too.
    func testATaskNoticeReadsThePlanAgain() {
        let app = openBoard()
        showPlan(app)
        XCTAssertTrue(element(app, "plan-lane-plan-phones").waitForExistence(timeout: 10))
        Self.notify("task-news")
        XCTAssertTrue(element(app, "plan-lane-plan-phones-v2").waitForExistence(timeout: 15), "the task notice didn't re-read the plan")
    }

    /// A notice reads the record of the page on screen, not every page opened since.
    func testANoticeReadsOnlyThePageOnScreen() {
        let app = openBoard()
        showPlan(app)
        let lane = element(app, "plan-lane-plan-phones")
        XCTAssertTrue(lane.waitForExistence(timeout: 10))
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
    }

    func testEachLaneNamesItsThemeAndEachThemeReadsOnItsOwn() {
        let app = openBoard()
        showPlan(app)
        XCTAssertTrue(element(app, "plan-next-up").waitForExistence(timeout: 10))
        let lane = element(app, "plan-lane-plan-phones")
        XCTAssertTrue(lane.waitForExistence(timeout: 5), "no plan-phones in Next Up")
        // What the row says is what it draws, so a theme that isn't drawn isn't here.
        XCTAssertTrue(lane.label.contains("Plan layer"), "the lane doesn't name its theme: \(lane.label)")
        XCTAssertTrue(lane.label.contains("Unblocked once the store"), lane.label)
        let now = element(app, "plan-now")
        for _ in 0..<8 where !now.exists { app.swipeUp() }
        XCTAssertTrue(now.exists, "no Now section")
        let working = element(app, "plan-lane-mac-vis")
        for _ in 0..<4 where !working.exists { app.swipeUp() }
        XCTAssertTrue(working.exists && working.label.contains("Visual language"), "a lane in Now doesn't name its theme")

        let theme = element(app, "plan-theme-Visual language")
        for _ in 0..<8 where !(theme.exists && theme.isHittable) { app.swipeUp() }
        XCTAssertTrue(theme.exists, "no theme card: \(app.debugDescription)")
        for part in [
            "Every Mac surface reads as one app", "4 of 18 done", "Next: mac-vis finishes",
            "Needs you: Should the sidebar tint",
        ] {
            XCTAssertTrue(theme.label.contains(part), "the theme doesn't say \(part): \(theme.label)")
        }
        keep(app, "plan-themes")
    }

    func testAnOutcomeWrapsToThreeLines() {
        let app = openBoard(["-phone-plan", "-phone-plan-outcomes"])
        showPlan(app)
        let rows = ["Visual language", "Mac navigation", "Reliability"].map { element(app, "plan-theme-\($0)") }
        XCTAssertTrue(element(app, "plan-next-up").waitForExistence(timeout: 10))
        for _ in 0..<8 where !rows.allSatisfy({ $0.exists && $0.isHittable }) { app.swipeUp() }
        XCTAssertTrue(rows.allSatisfy(\.exists), "the three themes aren't on screen together: \(app.debugDescription)")
        let heights = rows.map(\.frame.height)
        let line = heights[1] - heights[0]
        XCTAssertGreaterThan(line, 8, "one line and two lines are the same height: \(heights)")
        // Far too many words show three lines: two more than the short one.
        XCTAssertEqual(
            heights[2] - heights[0], 2 * line, accuracy: line * 0.6, "the outcome isn't three lines: \(heights)")
        keep(app, "plan-outcome-lines")
    }

    func testALaneAndAThemeOpenTheirPages() {
        let app = openBoard()
        showPlan(app)
        let lane = element(app, "plan-lane-plan-phones")
        XCTAssertTrue(lane.waitForExistence(timeout: 10))
        lane.tap()
        XCTAssertTrue(element(app, "plan-lane-page").waitForExistence(timeout: 10), "no lane page: \(app.debugDescription)")
        XCTAssertTrue(element(app, "plan-lane-theme").exists, "the lane page doesn't name its theme")
        keep(app, "plan-lane-page")
        element(app, "plan-lane-theme").tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10), "no theme page")
        XCTAssertTrue(app.staticTexts["Plan layer"].exists)
        keep(app, "plan-theme-page")
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        XCTAssertTrue(element(app, "plan-next-up").waitForExistence(timeout: 10), "Back didn't return to the plan")
        XCTAssertTrue(app.segmentedControls["plan-switch"].buttons["Plan"].isSelected, "the choice was forgotten")
    }

    func testAThemePageShowsWhatChanged() {
        let app = openBoard()
        showPlan(app)
        let theme = element(app, "plan-theme-Visual language")
        for _ in 0..<8 where !(theme.exists && theme.isHittable) { app.swipeUp() }
        theme.tap()
        XCTAssertTrue(element(app, "plan-theme-page").waitForExistence(timeout: 10))
        let change = element(app, "plan-what-changed")
        XCTAssertTrue(change.waitForExistence(timeout: 10), "no What Changed: the record wasn't read")
        change.tap()
        XCTAssertTrue(element(app, "plan-previous-story").waitForExistence(timeout: 5), "the old story isn't shown")
        keep(app, "plan-what-changed")
    }

    func testARefusedReadSaysSoAndOffersTryAgain() {
        let app = openBoard(["-phone-plan-fails"])
        showPlan(app)
        XCTAssertTrue(element(app, "plan-unavailable").waitForExistence(timeout: 10), "no unavailable state")
        XCTAssertEqual(element(app, "plan-unavailable").label, "Far Cooler couldn’t read this board’s plan.")
        XCTAssertTrue(element(app, "plan-retry").exists)
        XCTAssertFalse(element(app, "plan-loading").exists)
        keep(app, "plan-unavailable")
    }

    func testAnUnansweredReadEndsInTheUnavailableStateNotASpinner() {
        let app = openBoard(["-phone-plan-hangs", "-phone-plan-timeout", "2"])
        showPlan(app)
        // The runner never answers; the phone gives up after two seconds. (No
        // look at the spinner on the way: XCUITest waits for the app to idle
        // before it looks, and a spinner holds it from idling until it's gone.)
        XCTAssertTrue(
            element(app, "plan-unavailable").waitForExistence(timeout: 15), "the spinner never ended: \(app.debugDescription)")
        XCTAssertEqual(app.activityIndicators.matching(identifier: "plan-loading").count, 0, "still spinning")
        XCTAssertFalse(element(app, "plan-loading").exists, "still spinning")
        XCTAssertTrue(element(app, "plan-retry").exists)
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
            XCTAssertTrue(element(app, "plan-next-up").waitForExistence(timeout: 10))
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
            app.terminate()
        }
    }
}
