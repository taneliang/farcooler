import XCTest

/// The owner's phone adding a browser to Cosmo, a runner it reaches only
/// through Tailcat (ov-828), over `-grant-browser`: the runner is listed by
/// its route, not "e-liang@" with nothing after it, and when Cosmo doesn't
/// take the key the reply carries why, for the browser to say.
final class GrantBrowserUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += ["-grant-browser"] + arguments
        app.launchDrawn()
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    func testATailcatRunnerIsListedByItsRoute() {
        let app = launch([])
        let detail = element(app, "grant-row-detail")
        XCTAssertTrue(detail.waitForExistence(timeout: XCUIApplication.firstFrame), app.debugDescription)
        XCTAssertEqual(detail.label, "e-liang, through Tailcat")
        XCTAssertFalse(detail.label.hasSuffix("@"), "a Tailcat runner shows an empty address")
    }

    func testARefusedGrantCarriesItsReasonToTheBrowser() {
        let app = launch([])
        let reply = element(app, "grant-reply")
        XCTAssertTrue(reply.waitForExistence(timeout: 30), "no reply: \(app.debugDescription)")
        XCTAssertTrue(reply.label.contains(#""pending":true"#), reply.label)
        XCTAssertTrue(reply.label.contains(#""pending_why":"not_allowed""#), reply.label)
        let note = element(app, "grant-note")
        XCTAssertTrue(note.exists, app.debugDescription)
        XCTAssertEqual(
            note.label,
            "Cosmo didn’t let this iPhone add devices. Update Far Cooler on Cosmo, or add the device from your Mac.")
    }

    func testARunnerThePhoneNeverReachedSaysSo() {
        let app = launch(["-grant-browser-unreached"])
        let reply = element(app, "grant-reply")
        XCTAssertTrue(reply.waitForExistence(timeout: 30), "no reply: \(app.debugDescription)")
        XCTAssertTrue(reply.label.contains(#""pending_why":"not_connected""#), reply.label)
    }
}
