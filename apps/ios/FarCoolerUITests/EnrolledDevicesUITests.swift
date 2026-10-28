import XCTest

/// Runner Settings lists the devices that may log in to the runner, each with
/// who added it, and Revoke offers to take what a device added too (ov-855).
/// Over the canned runner (`-phone-harness -phone-devices`): a Mac that added a
/// phone that added two browsers, and a tablet whose adder is gone.
///
/// Also a capture tool: off unless `TEST_RUNNER_FARCOOLER_CAPTURE_OUT` names a
/// folder, it shoots the list and the two choices in light and dark.
final class EnrolledDevicesUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func devices(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(["-phone-devices"] + arguments)
        let gear = app.buttons["Settings"].firstMatch
        XCTAssertTrue(gear.waitForExistence(timeout: 30), "no Settings button: \(app.debugDescription)")
        gear.tap()
        let link = app.buttons["Runner Settings"].firstMatch
        XCTAssertTrue(app.reveal(link), "no Runner Settings row: \(app.debugDescription)")
        link.tap()
        XCTAssertTrue(
            app.staticTexts["Added by Studio Mac"].firstMatch.waitForExistence(timeout: 30)
                || app.reveal(app.staticTexts["Added by Studio Mac"].firstMatch),
            "no device list: \(app.debugDescription)")
        return app
    }

    private func sent(_ app: XCUIApplication) -> [String] {
        (app.descendants(matching: .any)["harness-sent"].firstMatch.value as? String)
            .map { $0.split(separator: "\n").map(String.init) } ?? []
    }

    private func shoot(_ name: String, _ scheme: String, expecting state: CaptureState) {
        guard let path = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        guard requireState(state, for: "\(name)-\(scheme)") else { return }
        Thread.sleep(forTimeInterval: 1)
        let url = URL(fileURLWithPath: path).appendingPathComponent("iphone-\(name)-\(scheme).png")
        try? FileManager.default.createDirectory(at: URL(fileURLWithPath: path), withIntermediateDirectories: true)
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    func testEachDeviceNamesWhoAddedIt() {
        let app = devices([])
        for line in [
            "Added by Studio Mac", "Added by Alexandra Montgomery-Fitzgerald’s iPhone", "Added by a device that was removed",
        ] {
            XCTAssertTrue(app.reveal(app.staticTexts[line].firstMatch), "no “\(line)”: \(app.debugDescription)")
        }
    }

    func testRevokingADeviceThatAddedOthersOffersBothChoicesAndSendsWithAddedOnlyForTheSecond() {
        for scheme in ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] == nil ? ["light"] : ["light", "dark"] {
            let app = devices(["-phone-appearance", scheme])
            let revokes = app.buttons.matching(identifier: "Revoke…")
            XCTAssertTrue(app.reveal(revokes.element(boundBy: 1)), "no Revoke: \(app.debugDescription)")
            shoot("devices-list", scheme, expecting: .shows(app.staticTexts["Added by Studio Mac"].firstMatch, "the list"))
            revokes.element(boundBy: 1).tap()
            let both = app.buttons["Revoke This and the 2 It Added"].firstMatch
            XCTAssertTrue(both.waitForExistence(timeout: 10), "no second choice: \(app.debugDescription)")
            XCTAssertTrue(app.buttons["Revoke This"].firstMatch.exists)
            shoot("devices-revoke", scheme, expecting: .shows(both, "the two choices"))
            // Just the one first: the browsers it added stay.
            app.buttons["Revoke This"].firstMatch.tap()
            XCTAssertTrue(
                Self.eventually { self.sent(app).contains("client.revoke phone-a withAdded=false") }, "\(self.sent(app))")
            XCTAssertTrue(app.staticTexts["Added by a device that was removed"].firstMatch.waitForExistence(timeout: 10))
            app.terminate()
        }
        let app = devices([])
        let revokes = app.buttons.matching(identifier: "Revoke…")
        XCTAssertTrue(app.reveal(revokes.element(boundBy: 1)))
        revokes.element(boundBy: 1).tap()
        app.buttons["Revoke This and the 2 It Added"].firstMatch.tap()
        XCTAssertTrue(
            Self.eventually { self.sent(app).contains("client.revoke phone-a withAdded=true") }, "\(self.sent(app))")
        XCTAssertFalse(app.staticTexts["Alexandra’s Browser at the Platform Team Office"].firstMatch.waitForExistence(timeout: 3))
    }

    func testARefusedRevokeKeepsTheListAndSaysSo() {
        let app = devices(["-phone-devices-refuse"])
        let revokes = app.buttons.matching(identifier: "Revoke…")
        XCTAssertTrue(app.reveal(revokes.element(boundBy: 1)))
        revokes.element(boundBy: 1).tap()
        app.buttons["Revoke This"].firstMatch.tap()
        XCTAssertTrue(
            app.staticTexts["Couldn’t revoke that device. Check the runner is reachable, then try again."].firstMatch
                .waitForExistence(timeout: 10), "\(app.debugDescription)")
        XCTAssertTrue(app.staticTexts["Added by Studio Mac"].firstMatch.exists)
    }

    private static func eventually(within seconds: TimeInterval = 10, _ condition: () -> Bool) -> Bool {
        let end = Date().addingTimeInterval(seconds)
        while Date() < end {
            if condition() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return condition()
    }
}
