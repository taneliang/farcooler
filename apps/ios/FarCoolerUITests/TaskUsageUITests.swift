import XCTest

/// A task's Usage section (ov-195), over the canned runner: bil-7's spend
/// with its breakdown, an older runner, and a read that didn't come back.
/// Each attaches a screenshot, kept, for the light and dark sheets.
final class TaskUsageUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    /// Open bil-7 and scroll its Usage section into view.
    private func usage(_ arguments: [String], waitFor id: String) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(["-push-task", "bil-7"] + arguments)
        XCTAssertTrue(element(app, "task-heading").waitForExistence(timeout: 30), "bil-7 didn't open")
        let target = element(app, id)
        for _ in 0..<6 where !(target.exists && target.isHittable) {
            app.swipeUp()
        }
        XCTAssertTrue(target.waitForExistence(timeout: 10), "no \(id): \(app.debugDescription)")
        return app
    }

    private func keep(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    func testSpendWithItsBreakdown() {
        let app = usage([], waitFor: "task-usage")
        let totals = element(app, "task-usage")
        XCTAssertTrue(totals.label.contains("tokens"), totals.label)
        XCTAssertTrue(totals.label.contains("API-equivalent"), totals.label)
        XCTAssertTrue(totals.label.contains("partly not reported"), totals.label)
        element(app, "task-usage-breakdown").tap()
        let rows = app.descendants(matching: .any)
        let claude = rows.matching(NSPredicate(format: "label CONTAINS '$2.87 partly not reported'")).firstMatch
        XCTAssertTrue(claude.waitForExistence(timeout: 5), "claude's row hides its caveat: \(app.debugDescription)")
        // The breakdown opens below the fold since the task screen grew its
        // Ask the Orchestrator section (ov-241): on an iPhone 17 claude's row
        // ends at the bottom of the screen and codex's is a cell the list has
        // not made yet. So scroll to it, as a reader would.
        let codex = rows.matching(NSPredicate(format: "label CONTAINS 'Cost not reported'")).firstMatch
        for _ in 0..<4 where !(codex.exists && codex.isHittable) {
            app.swipeUp()
        }
        XCTAssertTrue(codex.waitForExistence(timeout: 5), "codex's row: \(app.debugDescription)")
        keep(app, "usage-breakdown")
    }

    func testAnOlderRunnerNeedsAnUpdate() {
        let app = usage(["-phone-usage-old"], waitFor: "task-usage-needs-update")
        XCTAssertEqual(element(app, "task-usage-needs-update").label, "This runner needs an update to show spend.")
        keep(app, "usage-needs-update")
    }

    func testAFailedReadOffersTryAgain() {
        let app = usage(["-phone-usage-fails"], waitFor: "task-usage-retry")
        XCTAssertTrue(app.staticTexts["Far Cooler couldn’t read this task’s usage."].exists)
        keep(app, "usage-failed")
    }
}
