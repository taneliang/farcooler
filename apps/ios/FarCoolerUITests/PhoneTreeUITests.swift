import XCTest

/// The plan strip, the Plan sheet and the One tree on an iPhone (ov-300),
/// over the canned runner, whose plan is a real board's:
/// `test/fixtures/plan-seeded.json` is the CLI's own output. Billing is led
/// by an orchestrator, so its screen opens on the strip.
///
/// The strip says what needs you and what's moving; a tap peeks the Plan as a
/// sheet at the medium detent, and a lane chosen in it closes the sheet and is
/// pushed. The Themes segment is the tree's root, and each level is pushed:
/// Theme › Task › Lane, each level opening on its node's own page, and a
/// terminal opening its worktree on that pane.
final class PhoneTreeUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

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

    /// Billing, led and planned, on its orchestrator.
    private func openBilling(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-billing-led", "-phone-plan", "-phone-plan-file", Self.fixture] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        XCTAssertTrue(app.buttons["segment-tree"].waitForExistence(timeout: 10), "no Themes segment")
        return app
    }

    private func tap(_ app: XCUIApplication, _ id: String, _ why: String) {
        let button = app.buttons[id]
        XCTAssertTrue(button.waitForExistence(timeout: 10), "\(why): no \(id) in \(app.debugDescription)")
        button.tap()
    }

    // MARK: The strip and the sheet

    /// **The strip says what needs you, what's moving and what's next**, over
    /// the orchestrator: the plan's two themes with an ask, its first two
    /// lanes in Now and how many more, and its next lane.
    func testTheStripSaysWhatNeedsYouAndWhatsMoving() {
        let app = openBilling()
        XCTAssertTrue(element(app, "orchestrator-pane").waitForExistence(timeout: 10), "not on the orchestrator")
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 15), "no strip: \(app.debugDescription)")
        XCTAssertTrue(strip.label.hasPrefix("Orchestrator idle, 2 need you, mac-vis Building, phones-b In Review"), strip.label)
        XCTAssertTrue(strip.label.contains("+4"), strip.label)
        XCTAssertTrue(strip.label.hasSuffix("next: plan-phones"), strip.label)
        // Above the pane, not over it: the pane's top (its mount probe sits
        // at its top edge) starts under the strip.
        let mount = element(app, "orchestrator-mount")
        XCTAssertLessThanOrEqual(strip.frame.maxY, mount.frame.minY + 1, "the strip covers the pane")
        keep(app, "strip")

        // Only on the orchestrator.
        app.buttons["segment-board"].tap()
        XCTAssertFalse(strip.waitForExistence(timeout: 2), "the strip stayed over the board")
    }

    /// **A tap peeks the Plan at the medium detent, over the pane**: the
    /// orchestrator's state and the plan's sections, and Done puts it away.
    func testTheStripPeeksThePlanAsASheet() {
        let app = openBilling()
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 15), "no strip")
        strip.tap()
        let header = element(app, "plan-sheet-orchestrator")
        XCTAssertTrue(header.waitForExistence(timeout: 10), "no sheet: \(app.debugDescription)")
        XCTAssertTrue(header.label.contains("Orchestrator · Idle"), header.label)
        XCTAssertTrue(element(app, "plan-sheet-needs-you").exists, "the sheet doesn't say what needs you")
        XCTAssertTrue(element(app, "plan-next-up").waitForExistence(timeout: 10), "no Next Up in the sheet")
        // A peek: the medium detent leaves the top of the screen uncovered.
        let window = app.windows.firstMatch.frame
        XCTAssertGreaterThan(app.navigationBars["Plan"].frame.minY, window.height * 0.3, "the sheet opened full height")
        keep(app, "peek")
        app.buttons["plan-sheet-done"].tap()
        XCTAssertFalse(header.waitForExistence(timeout: 3) && header.isHittable, "Done left the sheet up")
        XCTAssertTrue(strip.isHittable, "the strip is gone after the peek")
    }

    /// **A lane chosen in the sheet is pushed on the phone's one stack**, the
    /// sheet gone, and Back comes back to the orchestrator.
    func testALaneChosenInTheSheetIsPushed() {
        let app = openBilling()
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 15), "no strip")
        strip.tap()
        let lane = element(app, "plan-lane-plan-phones")
        XCTAssertTrue(lane.waitForExistence(timeout: 10), "no Next Up lane: \(app.debugDescription)")
        lane.tap()
        XCTAssertTrue(app.navigationBars["plan-phones"].waitForExistence(timeout: 10), "the lane wasn't pushed")
        XCTAssertFalse(element(app, "plan-sheet-orchestrator").exists, "the sheet stayed up")
        app.navigationBars.buttons.firstMatch.tap()
        XCTAssertTrue(strip.waitForExistence(timeout: 10), "Back didn't come back to the orchestrator")
    }

    // MARK: The tree

    /// **The Themes segment is the tree's root**: the plan's themes in its
    /// order with their progress, a dot on each that asks, then No Theme, and
    /// below them the checkout. No pinned places: Needs You is the app's root
    /// and Plan is the sheet.
    func testTheThemesSegmentIsTheTreesRoot() {
        let app = openBilling()
        app.buttons["segment-tree"].tap()
        let visual = app.buttons["tree-row-Visual language"]
        XCTAssertTrue(visual.waitForExistence(timeout: 10), "no theme rows: \(app.debugDescription)")
        XCTAssertTrue(visual.label.contains("Needs You"), "no dot on a theme that asks: \(visual.label)")
        let navigation = app.buttons["tree-row-Mac navigation"]
        XCTAssertTrue(navigation.exists)
        XCTAssertFalse(navigation.label.contains("Needs You"), "a dot on a theme that asks nothing")
        XCTAssertLessThan(visual.frame.minY, navigation.frame.minY, "not in plan order")
        XCTAssertTrue(app.buttons["tree-row-No Theme"].exists, "no No Theme")
        // Any element: a place's row would draw with no button, since it goes
        // nowhere on the phone.
        XCTAssertFalse(element(app, "tree-row-Plan").exists, "a pinned Plan place on the phone")
        XCTAssertFalse(element(app, "tree-row-Needs You").exists, "a pinned Needs You place on the phone")
        keep(app, "tree-root")
    }

    /// **Theme › Task › Lane, each pushed, each opening on its own page**,
    /// and a subagent with no pane says so rather than leading nowhere.
    func testTheTreePushesThemeTaskAndLane() {
        let app = openBilling()
        app.buttons["segment-tree"].tap()
        tap(app, "tree-row-Visual language", "the root")
        XCTAssertTrue(app.navigationBars["Visual language"].waitForExistence(timeout: 10), "the theme wasn't pushed")
        XCTAssertEqual(app.buttons["tree-own"].label, "Theme Page")
        tap(app, "tree-row-ov-220", "the theme's level")
        XCTAssertTrue(app.buttons["tree-own"].waitForExistence(timeout: 10))
        XCTAssertEqual(app.buttons["tree-own"].label, "Task Details")
        tap(app, "tree-row-mac-vis", "the task's level")
        XCTAssertTrue(app.navigationBars["mac-vis"].waitForExistence(timeout: 10), "the lane wasn't pushed")
        XCTAssertEqual(app.buttons["tree-own"].label, "Lane Page")
        // Its builder runs inside the orchestrator: a row, and no button.
        let subagent = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "No terminal; runs inside the orchestrator")).firstMatch
        XCTAssertTrue(subagent.waitForExistence(timeout: 5), "no subagent row: \(app.debugDescription)")
        keep(app, "tree-lane")
        app.buttons["tree-own"].tap()
        XCTAssertTrue(element(app, "plan-lane-page").waitForExistence(timeout: 10), "Lane Page didn't open the lane's page")
        // Back walks up the way it came down.
        app.navigationBars.buttons.firstMatch.tap()
        XCTAssertEqual(app.buttons["tree-own"].label, "Lane Page")
        app.navigationBars.buttons.firstMatch.tap()
        XCTAssertEqual(app.buttons["tree-own"].label, "Task Details")
    }

    /// **A terminal in the tree opens its worktree on that pane**: No Theme ›
    /// bil-9 › its own worktree › its shell.
    func testATerminalOpensItsWorktree() {
        let app = openBilling()
        app.buttons["segment-tree"].tap()
        tap(app, "tree-row-No Theme", "the root")
        tap(app, "tree-row-bil-9", "No Theme")
        tap(app, "worktree-row-fc-3-webhooks", "bil-9's level")
        XCTAssertEqual(app.buttons["tree-own"].label, "Open Worktree")
        let shell = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'tree-row-' AND NOT (label CONTAINS 'Agent')")).firstMatch
        XCTAssertTrue(shell.waitForExistence(timeout: 10), "no shell row: \(app.debugDescription)")
        shell.tap()
        XCTAssertTrue(app.buttons["worktree-back"].firstMatch.waitForExistence(timeout: 20), "the worktree didn't open")
    }

    /// **The filter is the tree's status axis**: In Review leaves the cards
    /// in review, and it's kept.
    func testTheFilterNarrowsTheTree() {
        let app = openBilling()
        app.buttons["segment-tree"].tap()
        XCTAssertTrue(app.buttons["tree-row-No Theme"].waitForExistence(timeout: 10))
        tap(app, "tree-filter", "the root")
        let inReview = app.buttons["In Review"]
        XCTAssertTrue(inReview.waitForExistence(timeout: 5), "no In Review in the menu")
        inReview.tap()
        // Billing's own cards: bil-7 needs a decision and bil-9 is in progress,
        // so No Theme holds none in review and goes.
        XCTAssertFalse(app.buttons["tree-row-No Theme"].waitForExistence(timeout: 3), "No Theme stayed under In Review")
        XCTAssertTrue(app.buttons["tree-row-Visual language"].exists, "a theme with cards in review went")
    }

    // MARK: Captures

    /// The strip, the peek, the full sheet, the tree's root and two levels,
    /// light and dark. Opt-in, as `PlanUITests.testCaptures` is: it flips the
    /// device's appearance and asserts nothing the tests above don't.
    /// `TEST_RUNNER_FC_CAPTURES=1`; `TEST_RUNNER_FC_CAPTURE_TAG` names a run
    /// taken with Increase Contrast or Reduce Transparency set on the device.
    func testCaptures() throws {
        let environment = ProcessInfo.processInfo.environment
        try XCTSkipUnless(environment["FC_CAPTURES"] == "1", "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take the sheets")
        let tag = environment["FC_CAPTURE_TAG"].map { "-\($0)" } ?? ""
        for (name, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            let app = openBilling()
            let strip = app.buttons["plan-strip"]
            XCTAssertTrue(strip.waitForExistence(timeout: 15))
            keep(app, "capture-strip-\(name)\(tag)")
            strip.tap()
            XCTAssertTrue(element(app, "plan-next-up").waitForExistence(timeout: 10))
            keep(app, "capture-peek-\(name)\(tag)")
            app.navigationBars["Plan"].swipeUp()
            keep(app, "capture-sheet-\(name)\(tag)")
            app.buttons["plan-sheet-done"].tap()
            app.buttons["segment-tree"].tap()
            XCTAssertTrue(app.buttons["tree-row-Visual language"].waitForExistence(timeout: 10))
            keep(app, "capture-tree-root-\(name)\(tag)")
            app.buttons["tree-row-Visual language"].tap()
            XCTAssertTrue(app.buttons["tree-row-ov-220"].waitForExistence(timeout: 10))
            keep(app, "capture-tree-theme-\(name)\(tag)")
            app.buttons["tree-row-ov-220"].tap()
            app.buttons["tree-row-mac-vis"].tap()
            XCTAssertTrue(app.navigationBars["mac-vis"].waitForExistence(timeout: 10))
            keep(app, "capture-tree-lane-\(name)\(tag)")
            app.terminate()
        }
    }
}
