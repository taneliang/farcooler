import XCTest

/// A worktree says when large files weren't downloaded, and can try again
/// (ov-199).
///
/// `-changes-layout-harness` mounts the real review screen; `-lfs-pointers`
/// gives its worktree two pointer files, and Try Again takes a moment and
/// leaves them, as it does when the runner still lacks the objects.
final class ChangesLfsNoticeTests: XCTestCase {
    private func launch(_ flags: String...) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += ["-changes-layout-harness"] + flags
        app.launchDrawn()
        return app
    }

    private func element(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    func testAWorktreeWithPointerFilesSaysSoAndCanTryAgain() {
        let app = launch("-lfs-pointers")
        XCTAssertTrue(element(app, "changes-lfs-notice").waitForExistence(timeout: 30), "no notice")
        XCTAssertTrue(app.staticTexts["Some large files weren\u{2019}t downloaded."].exists)
        let retry = app.buttons["changes-lfs-retry"]
        XCTAssertEqual(retry.label, "Try Again")
        retry.tap()
        // The call is in flight: the button says so and can't be pressed twice.
        let working = NSPredicate(format: "label == %@ AND isEnabled == false", "Trying Again\u{2026}")
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: working, object: retry)], timeout: 5),
            .completed, "Try Again did not start a retry")
        let done = NSPredicate(format: "label == %@ AND isEnabled == true", "Try Again")
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: done, object: retry)], timeout: 15),
            .completed, "the retry never finished")
    }

    func testAReadGrantSeesTheSentenceWithoutTheButton() {
        let app = launch("-lfs-pointers", "-lfs-read-scope")
        XCTAssertTrue(element(app, "changes-lfs-notice").waitForExistence(timeout: 30))
        XCTAssertFalse(app.buttons["changes-lfs-retry"].exists)
    }

    func testAWorktreeWithNoPointerFilesSaysNothing() {
        let app = launch()
        XCTAssertTrue(app.staticTexts["feat/handle-retries-on-429"].waitForExistence(timeout: 30))
        XCTAssertFalse(element(app, "changes-lfs-notice").exists)
    }
}
