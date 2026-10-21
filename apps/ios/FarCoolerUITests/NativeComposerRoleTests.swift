import XCTest

/// The composer is read as a group of controls, never as an alert (ov-680).
/// XCUI reads an element with the alert role as an interruption it must
/// handle before every tap, and VoiceOver announces one as an alert.
final class NativeComposerRoleTests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness"]
        app.launchDrawn()
        XCTAssertTrue(
            app.descendants(matching: .any)["native-transcript"].waitForExistence(timeout: 60),
            "the conversation never showed")
        return app
    }

    func testTheComposerIsNotAnAlert() {
        let app = launch()
        let stack = app.descendants(matching: .any)["native-composer-stack"]
        XCTAssertTrue(stack.waitForExistence(timeout: 30), "no composer")
        XCTAssertEqual(app.alerts.count, 0, "an Alert is in the hierarchy: \(app.alerts.firstMatch.debugDescription)")
        XCTAssertNotEqual(stack.elementType, .alert, "the composer's stack is read as an Alert")
        XCTAssertEqual(
            app.descendants(matching: .alert).count, 0, "an Alert is in the hierarchy: \(app.debugDescription)")
    }

    /// The composer still reads as a field, a photo button and Send, by their
    /// labels, and the audit finds nothing wrong with it.
    func testTheComposerReadsAsFieldAttachAndSend() throws {
        let app = launch()
        XCTAssertTrue(app.descendants(matching: .any)["native-composer"].waitForExistence(timeout: 30))
        XCTAssertEqual(app.descendants(matching: .any)["native-composer"].label, "Message Claude")
        XCTAssertEqual(app.buttons["native-attach"].label, "Attach Photos")
        // Send appears with something to send; at rest the box has no action.
        app.descendants(matching: .any)["native-composer"].tap()
        app.typeText("Hello")
        XCTAssertTrue(app.buttons["native-send"].waitForExistence(timeout: 30), "no Send once there is text")
        XCTAssertEqual(app.buttons["native-send"].label, "Send")
        // Only what is inside the composer is this test's to judge: the
        // transcript above it has audit findings of its own.
        let composer = app.descendants(matching: .any)["native-composer-stack"].frame
        try app.performAccessibilityAudit(for: [.sufficientElementDescription, .hitRegion, .trait]) { issue in
            guard let frame = issue.element?.frame else { return true }
            // The surface and the probe are DEBUG elements the tests read, not
            // controls a person meets.
            if ["native-composer-surface", "native-composer-probe"].contains(issue.element?.identifier) { return true }
            return !composer.insetBy(dx: -1, dy: -1).contains(frame)
        }
    }
}
