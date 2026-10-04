import XCTest

/// A failed send, retried, is drawn once (ov-172).
///
/// The harness has no runner behind it, so every send fails, which is the
/// state under test. The message is echoed into the transcript before the call
/// and the Retry button sends it again; the second send used to draw a second
/// echo beside the first.
final class AgentRetrySendTests: XCTestCase {
    func testARetriedSendIsDrawnOnce() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain"]
        app.launchDrawn()

        let typed = "please run the tests again"
        let composer = app.textViews.firstMatch
        XCTAssertTrue(composer.waitForExistence(timeout: 30))
        composer.tap()
        app.typeText(typed)
        let send = app.buttons["agent-send"]
        XCTAssertTrue(send.waitForExistence(timeout: 10))
        send.tap()

        let retry = app.buttons["Retry"]
        XCTAssertTrue(retry.waitForExistence(timeout: 30), "the failed send never said so")
        let drawn = app.staticTexts.matching(NSPredicate(format: "label == %@", typed))
        XCTAssertEqual(drawn.count, 1, "the first send drew the message more than once")

        retry.tap()
        // The retry fails the same way, so the banner is the sign it finished.
        XCTAssertTrue(retry.waitForExistence(timeout: 30))
        Thread.sleep(forTimeInterval: 1)
        XCTAssertEqual(drawn.count, 1, "a retried send drew the message twice")
    }
}
