import XCTest

/// A shared runner's Member Logins switches (ov-652): the owner's phone turns
/// them, a member's reads them. Over the canned runner (`-phone-own-logins`).
final class MemberLoginsUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func runnerSettings(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(arguments)
        let gear = app.buttons["Settings"].firstMatch
        XCTAssertTrue(gear.waitForExistence(timeout: 30), "no Settings button: \(app.debugDescription)")
        gear.tap()
        let link = app.buttons["Runner Settings"].firstMatch
        XCTAssertTrue(app.reveal(link), "no Runner Settings row: \(app.debugDescription)")
        link.tap()
        return app
    }

    private func keep(_ app: XCUIApplication, _ name: String, expecting state: CaptureState) {
        guard requireState(state, for: name) else { return }
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    func testTheOwnersPhoneTurnsASwitch() {
        let app = runnerSettings(["-phone-shared", "-phone-own-logins", "-phone-rep-owner", "-phone-rep-long", "-phone-host-long"])
        let claude = element(app, "member-own-login-claude")
        XCTAssertTrue(app.reveal(claude), "no Claude switch: \(app.debugDescription)")
        let footer = element(app, "member-own-logins-footer")
        XCTAssertTrue(app.reveal(footer), "no footer: \(app.debugDescription)")
        XCTAssertTrue(footer.label.contains("uses your account, its quota and its history"), footer.label)
        let cursor = element(app, "member-own-login-cursor")
        XCTAssertTrue(app.reveal(cursor))
        XCTAssertTrue(cursor.isEnabled, "the owner may turn it")
        keep(app, "member-logins-owner", expecting: .reads(footer, "uses your account"))
        let toggle = cursor.switches.firstMatch.exists ? cursor.switches.firstMatch : cursor
        toggle.switches.firstMatch.exists ? toggle.switches.firstMatch.tap() : toggle.tap()
        let sent = element(app, "harness-sent")
        let turned = NSPredicate { _, _ in (sent.value as? String ?? "").contains("own-login cursor true") }
        XCTAssertEqual(XCTWaiter.wait(for: [expectation(for: turned, evaluatedWith: nil)], timeout: 10), .completed, "no switch sent: \(String(describing: sent.value))")
    }

    func testAMembersPhoneReadsThem() {
        let app = runnerSettings(["-phone-shared", "-phone-own-logins"])
        let codex = element(app, "member-own-login-codex")
        XCTAssertTrue(app.reveal(codex), "no Codex switch: \(app.debugDescription)")
        XCTAssertFalse(codex.isEnabled, "a member reads them only")
        let footer = element(app, "member-own-logins-footer")
        XCTAssertTrue(app.reveal(footer))
        XCTAssertTrue(footer.label.contains("the runner owner’s account") && footer.label.contains("Only this runner’s admins can change these."), footer.label)
        keep(app, "member-logins-member", expecting: .reads(footer, "Only this runner’s admins"))
    }

    /// A turn the runner refuses goes back to what the runner has, and the
    /// screen says it didn't take (ov-652).
    func testARefusedTurnGoesBack() {
        let app = runnerSettings(["-phone-shared", "-phone-own-logins", "-phone-own-logins-refused", "-phone-rep-owner"])
        let cursor = element(app, "member-own-login-cursor")
        XCTAssertTrue(app.reveal(cursor), "no Cursor switch: \(app.debugDescription)")
        let toggle = cursor.switches.firstMatch.exists ? cursor.switches.firstMatch : cursor
        XCTAssertEqual(toggle.value as? String, "0")
        toggle.tap()
        let said = app.staticTexts["That runner didn’t accept the change."].firstMatch
        XCTAssertTrue(app.reveal(said), "no failure said: \(app.debugDescription)")
        XCTAssertTrue(app.reveal(cursor))
        XCTAssertEqual(toggle.value as? String, "0", "the switch went back to what the runner has")
    }
}
