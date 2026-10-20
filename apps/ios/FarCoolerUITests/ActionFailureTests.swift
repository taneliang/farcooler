import XCTest

/// A call the runner refuses is said, not swallowed (ov-179).
///
/// Each of these was a `try?`: the control moved or the sheet closed and
/// nothing came back, so a refusal looked exactly like success. They stand on
/// the harnesses' canned runners, which refuse on request, so none of this
/// needs a runner.
final class ActionFailureTests: XCTestCase {
    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    /// A hide the runner refuses says so in an alert.
    func testARefusedUnhideSaysSo() {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-webhooks-hidden", "-phone-hide-fails"])
        let workspace = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(workspace.waitForExistence(timeout: 30))
        workspace.tap()
        // bil-9's own worktree, under its card in Billing's Plan (ov-300, ov-616).
        for id in ["tree-row-No Theme", "tree-row-bil-9"] {
            XCTAssertTrue(app.buttons[id].waitForExistence(timeout: 10), "no \(id)")
            app.buttons[id].tap()
        }
        let row = app.buttons["worktree-row-fc-3-webhooks"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        let from = row.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5))
        from.press(
            forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -120, dy: 0)),
            withVelocity: .slow, thenHoldForDuration: 0.3)
        let unhide = app.buttons["unhide-fc-3-webhooks"]
        XCTAssertTrue(unhide.waitForExistence(timeout: 5))
        unhide.tap()
        XCTAssertTrue(
            app.alerts["Couldn’t unhide the worktree"].waitForExistence(timeout: 10),
            "a refused unhide said nothing")
    }

    /// A task whose record can't be read says the record is missing.
    func testATaskWhoseRecordCannotBeReadSaysSo() {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox", "-phone-task-fails"])
        let workspace = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(workspace.waitForExistence(timeout: 30))
        workspace.tap()
        for id in ["tree-row-No Theme", "tree-row-bil-9", "tree-own"] {
            XCTAssertTrue(app.buttons[id].waitForExistence(timeout: 10), "no \(id)")
            app.buttons[id].tap()
        }
        XCTAssertTrue(
            app.staticTexts["Couldn’t read this task’s record. Pull down to try again."]
                .waitForExistence(timeout: 10),
            "a record that didn't load said nothing")
    }

    /// A create the runner can't make keeps the sheet open and says why,
    /// where it used to close on a worktree that was never made.
    func testARefusedCreateKeepsTheSheetOpenAndSaysSo() {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox"])
        let workspace = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(workspace.waitForExistence(timeout: 30))
        workspace.tap()
        let fromBranch = app.buttons["From a Branch…"]
        XCTAssertTrue(element(app, "plan-filters").waitForExistence(timeout: 10))
        XCTAssertTrue(app.reveal(fromBranch), "no From a Branch…")
        fromBranch.tap()
        // The repository picker is a menu; the harness has one repository.
        let picker = app.buttons.matching(
            NSPredicate(format: "label BEGINSWITH %@", "Repository")).firstMatch
        XCTAssertTrue(picker.waitForExistence(timeout: 10), app.debugDescription)
        picker.tap()
        app.buttons["overnight"].tap()
        let name = app.textFields["Name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap()
        name.typeText("retry-queue")
        app.buttons["Create"].tap()
        XCTAssertTrue(
            app.staticTexts.matching(
                NSPredicate(format: "label BEGINSWITH %@", "Couldn’t create the worktree"))
                .firstMatch.waitForExistence(timeout: 30),
            "a refused create said nothing")
        XCTAssertTrue(app.navigationBars["New Worktree"].exists, "the sheet closed on a refusal")
    }
}
