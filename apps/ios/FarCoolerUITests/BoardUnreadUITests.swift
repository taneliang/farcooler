import XCTest

/// The board's Unread section on a phone (ov-113), over the canned runner that
/// keeps read state (`-phone-board-reads`): Billing's floor is a day back, so
/// bil-5 (done a day ago) and bil-7 (moved ten minutes ago) are unread.
///
/// Opening a ticket reads it, and Mark All as Read asks before it reads
/// anything. The runner is told each through `workspace.mark_read`, and the
/// harness's `sent` says what it was told: the line goes either way, since the
/// phone marks it read at once, so only `sent` shows the sync. A phone with a
/// Read grant tells it nothing, and says its Mark All as Read is this device only.
final class BoardUnreadUITests: XCTestCase {
    private static let done = "board-unread-0198f2c0-0000-7000-8000-00000000e005/done"
    private static let moved = "board-unread-0198f2c0-0000-7000-8000-00000000e007/needs_decision"
    private static let doneTask = "0198f2c0-0000-7000-8000-00000000e005"

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    /// What the screens have told the canned runner, one write a line.
    private func sent(_ app: XCUIApplication) -> [String] {
        (element(app, "harness-sent").value as? String).map { $0.split(separator: "\n").map(String.init) } ?? []
    }

    private func waitForSent(_ app: XCUIApplication, _ count: Int, file: StaticString = #filePath, line: UInt = #line) {
        let deadline = Date().addingTimeInterval(10)
        while sent(app).count < count, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        XCTAssertGreaterThanOrEqual(sent(app).count, count, "the runner was not told: \(sent(app))", file: file, line: line)
    }

    private func openBoard(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox", "-phone-board-reads"] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        app.buttons["segment-board"].tap()
        XCTAssertTrue(element(app, "board-unread").waitForExistence(timeout: 10), "no Unread section")
        return app
    }

    func testUnreadListsWhatIsNewAndOpeningATicketReadsIt() throws {
        let app = openBoard()
        XCTAssertTrue(element(app, Self.done).waitForExistence(timeout: 10), "bil-5's finish is not listed")
        XCTAssertTrue(element(app, Self.moved).exists, "bil-7's move is not listed")
        XCTAssertLessThan(
            element(app, "board-unread").frame.minY, element(app, "board-section-needs_decision").frame.minY,
            "Unread is not first")

        element(app, Self.done).tap()
        XCTAssertTrue(element(app, "task-heading").waitForExistence(timeout: 10), "the task did not open")
        app.navigationBars.buttons["BackButton"].firstMatch.tap()

        XCTAssertTrue(element(app, Self.moved).waitForExistence(timeout: 10), "the other line went too")
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: element(app, Self.done))
        waitForExpectations(timeout: 10)
        waitForSent(app, 1)
        XCTAssertEqual(sent(app), ["workspace.mark_read floor=0 tasks=\(Self.doneTask)"], "the runner was told the ticket was opened")
    }

    func testMarkAllAsReadAsksFirst() throws {
        let app = openBoard()
        XCTAssertTrue(element(app, Self.done).waitForExistence(timeout: 10))
        app.buttons["board-mark-all-read"].tap()
        let alert = app.alerts["Mark All as Read?"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5), "it did not ask")
        XCTAssertTrue(
            alert.staticTexts["2 tasks will be marked as read on all your devices."].exists,
            "it did not say it reaches every device")
        alert.buttons["Cancel"].tap()
        XCTAssertTrue(element(app, Self.done).exists, "Cancel read something")
        XCTAssertTrue(element(app, Self.moved).exists, "Cancel read something")

        app.buttons["board-mark-all-read"].tap()
        XCTAssertTrue(app.alerts.buttons["Mark as Read"].waitForExistence(timeout: 5))
        app.alerts.buttons["Mark as Read"].tap()
        XCTAssertTrue(
            element(app, "board-unread-empty").waitForExistence(timeout: 10), "Unread did not empty")
        XCTAssertEqual(element(app, "board-unread-empty").label, "You’re all caught up.")
        XCTAssertFalse(element(app, Self.done).exists)
        waitForSent(app, 1)
        let told = try XCTUnwrap(sent(app).last)
        XCTAssertTrue(told.hasPrefix("workspace.mark_read floor="), told)
        XCTAssertFalse(told.hasPrefix("workspace.mark_read floor=0 "), "Mark All as Read told the runner no floor: \(told)")
        XCTAssertTrue(told.hasSuffix("tasks="), "a floor, and no marks with it: \(told)")
    }

    func testAReadGrantKeepsItsMarksAndSaysSo() throws {
        let app = openBoard(["-phone-read-scope"])
        XCTAssertTrue(element(app, Self.done).waitForExistence(timeout: 10))
        element(app, Self.done).tap()
        XCTAssertTrue(element(app, "task-heading").waitForExistence(timeout: 10))
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: element(app, Self.done))
        waitForExpectations(timeout: 10)
        app.buttons["board-mark-all-read"].tap()
        let alert = app.alerts["Mark All as Read?"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5))
        XCTAssertTrue(
            alert.staticTexts["1 task will be marked as read on this device only."].exists,
            "it did not say it is this device only")
        alert.buttons["Mark as Read"].tap()
        XCTAssertTrue(element(app, "board-unread-empty").waitForExistence(timeout: 10))
        RunLoop.current.run(until: Date().addingTimeInterval(2))
        XCTAssertEqual(sent(app), [], "a Read grant told the runner something")
    }
}
