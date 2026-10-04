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
        app.buttons["segment-worktrees"].tap()
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
        app.buttons["segment-board"].tap()
        let card = element(app, "board-card-bil-9")
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        card.tap()
        XCTAssertTrue(
            app.staticTexts["Couldn’t read this task’s record. Pull down to try again."]
                .waitForExistence(timeout: 10),
            "a record that didn't load said nothing")
    }

    /// A mode change the runner can't take is said beside the composer, with a
    /// Retry. The agent harness has no runner behind it, so every call fails.
    func testARefusedModeChangeSaysSo() {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain"]
        app.launchDrawn()
        let chip = app.buttons["Manual"]
        XCTAssertTrue(chip.waitForExistence(timeout: 30), "no mode chip")
        chip.tap()
        let auto = app.buttons["Auto"]
        XCTAssertTrue(auto.waitForExistence(timeout: 5))
        auto.tap()
        XCTAssertTrue(
            app.staticTexts["Couldn’t change that setting."].waitForExistence(timeout: 30),
            "a refused setting said nothing")
        XCTAssertTrue(app.buttons["Retry"].exists)
        // And the picker is back on what the runner still has.
        XCTAssertTrue(app.buttons["Manual"].exists, "the picker kept the refused value")
        XCTAssertFalse(app.buttons["Auto"].exists, "the picker kept the refused value")
    }

    /// A refused control doesn't take the unsent message's Retry away.
    func testARefusedControlKeepsAnUnsentMessagesRetry() {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain"]
        app.launchDrawn()
        let composer = app.textViews.firstMatch
        XCTAssertTrue(composer.waitForExistence(timeout: 30))
        composer.tap()
        app.typeText("keep this")
        app.buttons["agent-send"].tap()
        XCTAssertTrue(app.buttons["Retry"].waitForExistence(timeout: 30), "the send never failed")
        app.buttons["Manual"].tap()
        app.buttons["Auto"].tap()
        XCTAssertTrue(
            app.staticTexts["Couldn’t change that setting."].waitForExistence(timeout: 30))
        XCTAssertEqual(app.buttons.matching(identifier: "Retry").count, 2, "a banner was dropped")
    }

    /// A create the runner can't make keeps the sheet open and says why,
    /// where it used to close on a worktree that was never made.
    func testARefusedCreateKeepsTheSheetOpenAndSaysSo() {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox"])
        let workspace = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(workspace.waitForExistence(timeout: 30))
        workspace.tap()
        app.buttons["segment-worktrees"].tap()
        let fromBranch = app.buttons["From a Branch…"]
        XCTAssertTrue(fromBranch.waitForExistence(timeout: 10))
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
