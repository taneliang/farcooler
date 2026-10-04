import XCTest

/// A patch the daemon cut off says so, and a very long one is held back (ov-149).
///
/// Needs no runner: `-changes-layout-harness` mounts the review screen over a
/// canned change set, and `-diff-truncated`, `-diff-merge` and `-diff-long`
/// shape the open file's diff. The notice comes out of the same decode the
/// store runs on the daemon's bytes, so this is the wire fixture on screen.
final class ChangesPatchNoticeTests: XCTestCase {
    private func launch(_ flags: String...) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += ["-changes-layout-harness"] + flags
        app.launchDrawn()
        return app
    }

    private func element(_ app: XCUIApplication, _ identifier: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: identifier).firstMatch
    }

    func testATruncatedPatchSaysItWasCutShort() {
        let app = launch("-diff-truncated")
        let notice = element(app, "changes-file-notice")
        XCTAssertTrue(notice.waitForExistence(timeout: 30), "a cut-off patch said nothing")
        XCTAssertEqual(notice.label, "This patch was cut short. It\u{2019}s too big to send whole.")
    }

    func testAMergeSaysItIsShownAgainstItsFirstParent() {
        let app = launch("-diff-merge")
        let notice = element(app, "changes-file-notice")
        XCTAssertTrue(notice.waitForExistence(timeout: 30))
        XCTAssertEqual(notice.label, "This is a merge, shown against its first parent only.")
    }

    func testAWholePatchSaysNothing() {
        let app = launch()
        XCTAssertTrue(
            app.staticTexts["feat/handle-retries-on-429"].waitForExistence(timeout: 30),
            "the changes harness never mounted")
        XCTAssertFalse(element(app, "changes-file-notice").exists)
        XCTAssertFalse(element(app, "changes-show-more-lines").exists)
    }

    func testALongPatchIsHeldBackAndOfferedWhole() {
        let app = launch("-diff-long")
        let more = element(app, "changes-show-more-lines")
        // Lazy rows: the offer is at the END of 600 drawn lines, so scroll to it.
        for _ in 0..<60 where !more.exists { app.swipeUp(velocity: .fast) }
        XCTAssertTrue(more.exists, "a 700-line patch drew every line")
        XCTAssertEqual(more.label, "Show 100 More Lines")
        // The budget's call site, not just its label: 600 lines are drawn.
        let drawn = element(app, "changes-patch-drawn")
        XCTAssertEqual(drawn.value as? String, "600", "the held-back lines were drawn")
        more.tap()
        let all = NSPredicate { _, _ in (drawn.value as? String) == "700" }
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: all, object: nil)], timeout: 10),
            .completed, "Show More showed nothing")
        // No `waitForNonExistence` here: it snapshots the whole 700-line tree, which
        // timed out after 266 s on CI's slow simulator (ov-249). The probe reaching
        // 700 is the proof the rest was drawn, and it is one element's value.
    }
}
