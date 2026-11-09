import XCTest

/// The owner's phone answering a browser's offer (ov-770), over
/// `-sealed-reply`: the reply goes to the relay once, sealed, and the browser's
/// key opens it to a reply carrying Cosmo's host key, with the code the phone
/// shows. An offer with no reply key, or one that isn't a key, posts nothing.
final class SealedReplyUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += ["-sealed-reply"] + arguments
        app.launchDrawn()
        return app
    }

    private func label(_ app: XCUIApplication, _ id: String) -> String {
        let element = app.descendants(matching: .any)[id]
        XCTAssertTrue(element.waitForExistence(timeout: 30), "no \(id): \(app.debugDescription)")
        return element.label
    }

    func testABrowsersOfferIsSealedPostedOnceAndOpensWithItsKey() {
        let app = launch([])
        XCTAssertEqual(label(app, "sealed-posts"), "1")
        XCTAssertEqual(label(app, "sealed-phase"), "showing reply")
        XCTAssertEqual(label(app, "sealed-qr"), "reply on screen")
        XCTAssertEqual(label(app, "sealed-opened"), "opened with Cosmo’s host key")
        let code = label(app, "sealed-phone-code")
        XCTAssertEqual(code.count, 6, code)
        XCTAssertEqual(label(app, "sealed-opened-code"), code, "the browser computes the phone's code")
        XCTAssertEqual(label(app, "sealed-reply-code"), code)
        XCTAssertEqual(label(app, "sealed-reply-status"), "Sent to Chrome on Mac. Check that it shows this code:")
    }

    func testARefusedPostShowsTheFailureAndNeverTheCode() {
        let app = launch(["-sealed-reply-refused"])
        XCTAssertEqual(label(app, "sealed-posts"), "1")
        XCTAssertEqual(label(app, "sealed-qr"), "reply on screen")
        XCTAssertEqual(label(app, "sealed-phone-code"), "none")
        XCTAssertTrue(label(app, "sealed-reply-status").hasPrefix("Couldn’t send this to Chrome on Mac"))
        XCTAssertFalse(app.descendants(matching: .any)["sealed-reply-code"].exists)
    }

    /// Confirm tapped twice while Face ID is up builds and posts one reply
    /// (ov-778 review L1).
    func testADoubleTapOnConfirmPostsOnce() {
        let app = launch(["-sealed-reply-double"])
        XCTAssertEqual(label(app, "sealed-posts"), "1")
        XCTAssertEqual(label(app, "sealed-reply-status"), "Sent to Chrome on Mac. Check that it shows this code:")
    }

    /// The same reply sent again within the window: the relay keeps the first
    /// (409), and the browser holds the first, so the phone shows the first's
    /// code rather than "couldn't send" (ov-778 review L1).
    func testASecondSendTheRelayAlreadyHoldsShowsTheFirstCode() {
        let app = launch(["-sealed-reply-again"])
        XCTAssertEqual(label(app, "sealed-posts"), "2")
        XCTAssertEqual(label(app, "sealed-reply-status"), "Sent to Chrome on Mac. Check that it shows this code:")
        let opened = label(app, "sealed-opened-code")
        XCTAssertEqual(opened.count, 6, opened)
        XCTAssertEqual(label(app, "sealed-reply-code"), opened, "the code shown is the one the browser holds")
    }

    func testAnOfferWithoutAReplyKeyIsTheQrOnlyCeremony() {
        let app = launch(["-sealed-reply-none"])
        XCTAssertEqual(label(app, "sealed-posts"), "0")
        XCTAssertEqual(label(app, "sealed-phase"), "showing reply")
        XCTAssertEqual(label(app, "sealed-qr"), "reply on screen")
        XCTAssertEqual(label(app, "sealed-phone-code"), "none")
        XCTAssertFalse(app.descendants(matching: .any)["sealed-reply-status"].exists)
    }

    func testAReplyKeyThatIsNotAKeyIsRefusedAndPostsNothing() {
        let app = launch(["-sealed-reply-malformed"])
        XCTAssertEqual(label(app, "sealed-posts"), "0")
        XCTAssertEqual(label(app, "sealed-phase"), "refused malformed")
        XCTAssertEqual(label(app, "sealed-qr"), "no reply")
    }
}
