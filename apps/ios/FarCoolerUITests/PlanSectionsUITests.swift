import XCTest

/// The Plan as sections on an iPhone (ov-503, phase 7), over the canned runner
/// and `test/fixtures/plan-phones-ov503.json`. Only the wiring a unit test cannot
/// reach stays (ov-538): the band is one spoken element, and Lanes is a row that
/// pushes a list whose lane opens its page. What the sections say, in what order,
/// the ladder, the groups, the fold, the agents and the tiles are AgentKit's,
/// pinned in `PlanPhoneSectionsTests` on the same fixture.
final class PlanSectionsUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-phones-ov503.json").path
    }

    /// The first match: an identifier on a container reaches its text too.
    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// A short drag up, so a lazy list draws every row it passes.
    private func scrollDown(_ app: XCUIApplication) {
        let window = app.windows.firstMatch
        window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75))
            .press(forDuration: 0.05, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45)))
    }

    /// Billing's Plan, its screen while it has no orchestrator (ov-616).
    private func openPlan() -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan", "-phone-agents", "-phone-plan-file", Self.fixture])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        // Under the tree, which leads the Plan (ov-616).
        XCTAssertTrue(app.reveal(element(app, "plan-landing")), "no Landing: \(app.debugDescription)")
        return app
    }

    /// Scroll down until `id` can be touched.
    @discardableResult
    private func reach(_ app: XCUIApplication, _ id: String, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let target = element(app, id)
        // On screen, below the navigation bar and above the home indicator: a header isn't "hittable".
        let window = app.windows.firstMatch.frame
        func shown() -> Bool { target.exists && target.frame.minY > window.height * 0.2 && target.frame.maxY < window.height * 0.95 }
        for _ in 0..<40 where !shown() { scrollDown(app) }
        XCTAssertTrue(shown(), "can't reach \(id) (frame \(target.frame), exists \(target.exists)): \(app.debugDescription)", file: file, line: line)
        return target
    }

    // MARK: The band

    /// **The band is Now and Next in two lines, and what needs the owner is its one mark**, the
    /// words AgentKit's: the furthest-along train leads Now, and Next names the first ranked lane.
    func testTheBandSaysNowAndNextAndWhatNeedsYou() {
        let app = openPlan()
        let band = element(app, "plan-band")
        XCTAssertTrue(band.waitForExistence(timeout: 10), "no band: \(app.debugDescription)")
        // One element, read as the sentence: Now, Next, then what needs the owner.
        XCTAssertTrue(band.label.contains("Train 86 checking on CI"), band.label)
        XCTAssertTrue(band.label.contains(" building") && band.label.contains(" in review"), band.label)
        XCTAssertTrue(band.label.contains("Next: A real window capture on the front page, then Onboarding"), band.label)
        XCTAssertTrue(band.label.hasSuffix(" needs you."), "the band doesn't say what needs you: \(band.label)")
    }

    // MARK: Phase 11

    /// **A train waiting for main's checks to pass says so in place of its first step, and nothing says who holds it**
    /// (ov-546). `integ-89` is `waiting_for_main` in the fixture; the words are `TrainPhase`'s.
    func testATrainWaitingForMainSaysSo() {
        let app = openPlan()
        let phase = reach(app, "plan-train-integ-89-phase")
        XCTAssertEqual(phase.label, "Waiting for Main's Checks to Pass \u{00B7} step 1 of 3", "the phase isn't said: \(phase.label)")
        XCTAssertFalse(element(app, "plan-train-integ-89-held").exists, "a train never says who holds it")
        XCTAssertTrue(element(app, "plan-train-integ-89").label.contains("Waiting for Main's Checks to Pass"), element(app, "plan-train-integ-89").label)
    }

    // MARK: Gestures

    /// **A long press on a train explains it and never opens CI** (ov-518): the release of a long
    /// press once fired the tap that opens the run. The words are `TrainCard.explanation`'s.
    func testALongPressOnATrainExplainsAndNeverOpensCI() {
        let app = openPlan()
        reach(app, "plan-train-integ-86").press(forDuration: 0.8)
        XCTAssertTrue(
            app.staticTexts["A train is a batch of reviewed lanes that land on main together, after one round of checks."]
                .waitForExistence(timeout: 10),
            "a long press on a train didn't explain it: \(app.debugDescription)")
        XCTAssertEqual(app.state, .runningForeground, "a long press left the app")
        XCTAssertNotEqual(XCUIApplication(bundleIdentifier: "com.apple.mobilesafari").state, .runningForeground, "a long press opened CI")
    }

    // MARK: Themes and Lanes

    /// **Lanes groups them as the Plan does**: by train, then In Review, Building and the folded
    /// Waiting, each lane with its agents; a lane opens its page.
    func testLanesPushesItsList() {
        let app = openPlan()
        reach(app, "plan-browse-lanes").tap()
        XCTAssertTrue(element(app, "plan-browse-lanes-list").waitForExistence(timeout: 10), "Lanes didn't push")
        XCTAssertTrue(element(app, "plan-browse-Landing").waitForExistence(timeout: 5), "no Landing group")
        XCTAssertTrue(element(app, "plan-browse-row-Train 86").exists, "no train inside Landing")
        let lane = element(app, "plan-browse-row-team-dir")
        XCTAssertTrue(lane.exists, "no lane under its train")
        XCTAssertFalse(lane.label.contains("Ready"), "a lane aboard says nothing of its state: \(lane.label)")
        lane.tap()
        XCTAssertTrue(element(app, "plan-lane-page").waitForExistence(timeout: 10), "a lane didn't open its page")
    }

}
