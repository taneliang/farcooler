import XCTest

/// Decided For You on the iPhone (ov-304), over the canned runner, whose plan
/// is a real board's: `test/fixtures/plan-rulings-seeded.json` is `farcooler
/// plan --json` from a scratch daemon given four rulings with `plan ruling add`
/// and `set`, so the bytes the phone decodes are the CLI's.
///
/// Only the open ones, newest first; the settled one folds into Past
/// Decisions (ov-333). Keep and Keep All mark rulings kept through the runner;
/// nothing on a runner without `board_rulings`.
final class PlanRulingsUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-rulings-seeded.json").path
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

    /// Billing's board with Plan chosen, on a runner that keeps a plan and,
    /// unless `rulings` is false, rulings.
    private func openPlan(rulings: Bool = true) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan", "-phone-plan-file", Self.fixture]
                + (rulings ? ["-phone-rulings"] : []))
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        app.buttons["segment-board"].tap()
        let control = app.segmentedControls["plan-switch"]
        XCTAssertTrue(control.waitForExistence(timeout: 10), "no Tasks | Plan control")
        control.buttons["Plan"].tap()
        XCTAssertTrue(element(app, "plan-now").waitForExistence(timeout: 10), "the plan didn't draw")
        return app
    }

    private func reveal(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        let target = element(app, id)
        for _ in 0..<8 where !(target.exists && target.isHittable) { app.swipeUp() }
        return target
    }

    func testOpenRulingsComeFirstNewestFirstAndTheRestFold() {
        let app = openPlan()
        XCTAssertTrue(reveal(app, "plan-rulings").exists, "no Decided For You")
        let newest = reveal(app, "plan-ruling-R-4")
        XCTAssertTrue(newest.exists)
        let next = element(app, "plan-ruling-R-3")
        XCTAssertTrue(next.exists)
        XCTAssertLessThan(newest.frame.minY, next.frame.minY, "newest first")
        XCTAssertTrue(newest.label.contains("Why:"), "the reason is read: \(newest.label)")
        XCTAssertFalse(element(app, "plan-ruling-R-1").exists, "a kept ruling is folded into Past Decisions")
        let fold = reveal(app, "plan-rulings-past-header")
        XCTAssertTrue(fold.exists, "no Past Decisions")
        XCTAssertEqual(fold.value as? String, "Collapsed")
        fold.tap()
        let settled = reveal(app, "plan-ruling-R-1")
        XCTAssertTrue(settled.exists)
        XCTAssertTrue(settled.label.hasSuffix("Kept"), settled.label)
    }

    /// A swipe offers Keep; keeping moves the ruling from Decided For You into
    /// Past Decisions, and the runner was told.
    func testKeepMovesARulingIntoPastDecisions() {
        let app = openPlan()
        let row = reveal(app, "plan-ruling-R-2")
        row.swipeLeft()
        let keep = app.buttons["Keep"].firstMatch
        XCTAssertTrue(keep.waitForExistence(timeout: 5), "no Keep on the swipe")
        keep.tap()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: element(app, "plan-ruling-R-2"))
        waitForExpectations(timeout: 10)
        let fold = reveal(app, "plan-rulings-past-header")
        XCTAssertTrue(fold.exists)
        fold.tap()
        XCTAssertTrue(reveal(app, "plan-ruling-R-2").label.hasSuffix("Kept"))
        XCTAssertTrue(element(app, "plan-ruling-R-4").exists, "the others are still open")
    }

    /// Keep All is in the section's header while there's more than one open,
    /// and keeps every one: Decided For You is gone, and no count is left
    /// behind.
    func testKeepAllKeepsEveryOpenRuling() {
        let app = openPlan()
        let all = reveal(app, "plan-rulings-keep-all")
        XCTAssertTrue(all.exists, "no Keep All")
        all.tap()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: element(app, "plan-rulings"))
        waitForExpectations(timeout: 10)
        let fold = reveal(app, "plan-rulings-past-header")
        XCTAssertTrue(fold.exists)
        fold.tap()
        for short in ["R-4", "R-3", "R-2", "R-1"] {
            XCTAssertTrue(reveal(app, "plan-ruling-\(short)").label.hasSuffix("Kept"), short)
        }
    }

    func testCopyReferenceIsStillOnAnOpenRuling() {
        let app = openPlan()
        let copy = reveal(app, "plan-ruling-R-3-copy")
        XCTAssertTrue(copy.exists, "no Copy Reference")
        XCTAssertEqual(copy.label, "Copy Reference")
        copy.tap()
        // No sheet, no editor: the plan is still on screen.
        XCTAssertTrue(element(app, "plan-ruling-R-3").exists)
        XCTAssertFalse(app.textFields.firstMatch.exists || app.textViews.firstMatch.exists, "nothing to edit")
    }

    func testARunnerWithoutBoardRulingsShowsNothing() {
        let app = openPlan(rulings: false)
        for _ in 0..<6 { app.swipeUp() }
        XCTAssertFalse(element(app, "plan-rulings").exists)
        XCTAssertFalse(element(app, "plan-ruling-R-4").exists)
        XCTAssertFalse(element(app, "plan-rulings-past-header").exists)
    }

    /// Light and dark sheets. Opt-in, as PlanUITests' are:
    /// `TEST_RUNNER_FC_CAPTURES=1`.
    func testCaptures() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["FC_CAPTURES"] == "1",
            "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take the sheets")
        for (name, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            let app = openPlan()
            _ = reveal(app, "plan-ruling-R-2")
            keep(app, "capture-rulings-\(name)")
            app.terminate()
        }
    }
}
