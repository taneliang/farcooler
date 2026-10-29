import XCTest

/// The People screen's Access and the step-up behind its buttons (ov-856,
/// slice 4), over the canned runner that keeps the people's access
/// (`-phone-access`). No face is asked: the harness's signer answers at once
/// and `harness-sent` shows what the prompt said.
final class PeopleAccessUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func people(_ arguments: [String] = []) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(["-phone-access"] + arguments)
        let gear = app.buttons["Settings"].firstMatch
        XCTAssertTrue(gear.waitForExistence(timeout: 30), "no Settings button: \(app.debugDescription)")
        gear.tap()
        let link = app.buttons["Runner Settings"].firstMatch
        XCTAssertTrue(app.reveal(link), "no Runner Settings row: \(app.debugDescription)")
        link.tap()
        return app
    }

    private func sent(_ app: XCUIApplication) -> String {
        element(app, "harness-sent").value as? String ?? ""
    }

    private func waitForSent(_ app: XCUIApplication, _ text: String, file: StaticString = #filePath, line: UInt = #line) {
        let probe = element(app, "harness-sent")
        let has = NSPredicate { _, _ in (probe.value as? String ?? "").contains(text) }
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation(for: has, evaluatedWith: nil)], timeout: 15), .completed,
            "never sent \"\(text)\": \(sent(app))", file: file, line: line)
    }

    private func keep(_ app: XCUIApplication, _ name: String, expecting state: CaptureState) {
        guard requireState(state, for: name) else { return }
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    func testEachPersonSaysTheirAccessAndTheirDevicesHaveTheirWarnings() {
        let app = people()
        let cosmo = element(app, "people-person-cosmo")
        XCTAssertTrue(app.reveal(cosmo), "no Cosmo: \(app.debugDescription)")
        XCTAssertEqual(element(app, "people-access-cosmo").label, "Member")
        XCTAssertTrue(app.reveal(element(app, "people-access-dan")))
        XCTAssertEqual(element(app, "people-access-dan").label, "Admin")
        XCTAssertTrue(app.staticTexts["Added by Cosmos-iPhone"].exists, "no Added by: \(app.debugDescription)")
        XCTAssertTrue(app.staticTexts["Viewer on This Device"].exists, "no Viewer badge")
        let shell = element(app, "device-shell-mac-d")
        XCTAssertTrue(app.reveal(shell), "no shell warning")
        XCTAssertEqual(shell.label, "This device has a shell on studio, so it can do everything.")
        XCTAssertTrue(app.reveal(app.staticTexts["Check These Devices"]), "no Check These Devices")
        XCTAssertTrue(app.reveal(app.staticTexts["Waiting for Approval"]), "no Waiting for Approval")
        keep(app, "people-access-iphone-light", expecting: .shows(element(app, "people-waiting")))
    }

    func testRemovingADeviceAsksOnceAndNamesWhatItTakes() {
        let app = people(["-phone-access-trusted"])
        let menu = element(app, "device-menu-phone-c")
        XCTAssertTrue(app.reveal(menu), "no device menu: \(app.debugDescription)")
        menu.tap()
        app.buttons["Remove Device…"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Cosmos-iPhone loses its access to studio right away. It added one other device."].waitForExistence(timeout: 10), app.debugDescription)
        app.buttons["Remove Device and 1 It Added"].firstMatch.tap()
        waitForSent(app, "prompt Remove Cosmos-iPhone and the device it added.")
        waitForSent(app, "revoke phone-c withAdded=true")
        XCTAssertFalse(element(app, "device-pad-c").waitForExistence(timeout: 3), "the device it added was left")
    }

    func testClosingThePromptRemovesNothing() {
        let app = people(["-phone-access-trusted", "-phone-access-cancel"])
        let menu = element(app, "device-menu-pad-c")
        XCTAssertTrue(app.reveal(menu))
        menu.tap()
        app.buttons["Remove Device…"].firstMatch.tap()
        app.buttons["Remove Device"].firstMatch.tap()
        XCTAssertFalse(sent(app).contains("revoke"), "a closed prompt still removed it: \(sent(app))")
        XCTAssertTrue(element(app, "device-pad-c").exists)
        XCTAssertFalse(element(app, "people-access-note").exists, "closing the prompt is silence")
    }

    func testAllowingAdminActionsShowsTheDigitsToCheck() {
        let app = people()
        let allow = element(app, "admin-actions-allow")
        XCTAssertTrue(app.reveal(allow), "no Allow Admin Actions: \(app.debugDescription)")
        allow.tap()
        let code = element(app, "admin-actions-code")
        XCTAssertTrue(app.reveal(code), app.debugDescription)
        XCTAssertEqual(code.label, "273 905")
        waitForSent(app, "propose key")
        keep(app, "people-allow-admin-iphone-light", expecting: .reads(code, "273 905"))
    }

    func testOnlyTheSamePersonsOtherDeviceCanBeAllowedFromThisPhone() {
        let app = people(["-phone-access-trusted"])
        XCTAssertTrue(app.reveal(element(app, "admin-actions-on")), "this phone's key should be on")
        let allow = element(app, "waiting-allow-ipad-o")
        XCTAssertTrue(app.reveal(allow), "no Allow for the owner's iPad: \(app.debugDescription)")
        XCTAssertTrue(app.staticTexts["Only allow this if you’re holding iPad right now. Check that it shows 418 207."].exists)
        XCTAssertFalse(element(app, "waiting-allow-phone-d").exists, "Dan's key is Dan's to approve, not this phone's")
        allow.tap()
        waitForSent(app, "prompt Allow iPad to approve changes to who has access.")
        waitForSent(app, "approve ipad-o")
    }

    /// **The app lock stands above a link that names a terminal** (ov-856,
    /// §3a). `-appLockHold` keeps the lock up without a face to give it, and
    /// `-deep-link` is a link to an agent's pane. The terminal's cover would
    /// have been drawn above a plain overlay; the lock shows, and no terminal
    /// does, until the person unlocks.
    func testALinkToATerminalWaitsBehindTheLock() {
        let app = XCUIApplication.phoneHarness(["-appLock", "YES", "-appLockHold", "-deep-link", "0198f2c0-0000-7000-8000-00000000d002"])
        XCTAssertTrue(element(app, "app-lock").waitForExistence(timeout: 30), "no lock: \(app.debugDescription)")
        XCTAssertFalse(
            app.buttons["worktree-back"].firstMatch.waitForExistence(timeout: 8),
            "a terminal opened under the lock: \(app.debugDescription)")
        XCTAssertTrue(element(app, "app-lock").exists, "the lock went away")
    }
}
