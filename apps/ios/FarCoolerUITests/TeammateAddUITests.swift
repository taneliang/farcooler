import XCTest

/// The owner's phone adding a teammate's device (ov-854), over
/// `-teammate-add`: a code from another account is held while the directory
/// is asked, opens the teammate's confirmation only when the directory
/// vouches for it, asks what they may do and where, and writes only on Add
/// Teammate, at the access chosen and nowhere else.
final class TeammateAddUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ arguments: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += ["-teammate-add"] + arguments
        app.launchDrawn()
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func asked(_ app: XCUIApplication) -> String {
        (element(app, "teammate-asked").value as? String) ?? ""
    }

    private func keep(_ app: XCUIApplication, _ name: String, expecting state: CaptureState) {
        guard requireState(state, for: name) else { return }
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// The teammate's confirmation, up.
    private func confirmation(_ app: XCUIApplication) -> XCUIElement {
        let title = element(app, "teammate-title")
        XCTAssertTrue(
            title.waitForExistence(timeout: XCUIApplication.firstFrame),
            "the teammate's confirmation never opened: \(app.debugDescription)")
        return title
    }

    func testTheDirectorysEmailAndTeamAreNamedAndNothingIsTickedByDefault() {
        let app = launch()
        let title = confirmation(app)
        XCTAssertEqual(title.label, "Add Your Teammate’s Device?")
        let subtitle = element(app, "teammate-subtitle").label
        XCTAssertTrue(subtitle.contains("bob.smith@example.com"), subtitle)
        XCTAssertTrue(subtitle.contains("“Platform”"), subtitle)
        XCTAssertEqual((element(app, "teammate-name").value as? String), "Bob Smith")
        XCTAssertEqual((element(app, "teammate-handle").value as? String), "bob-smith")
        // Nothing is granted to someone else by default: Add Teammate waits for a runner.
        XCTAssertFalse(element(app, "teammate-add").isEnabled, "Add Teammate was on with no runner ticked")
        keep(app, "teammate-add-iphone", expecting: .reads(title, "Add Your Teammate’s Device?"))
    }

    func testARunnerThePhoneOnlyControlsCannotBeTicked() {
        let app = launch()
        _ = confirmation(app)
        let blocked = element(app, "teammate-runner-gpu-workstation")
        XCTAssertTrue(blocked.exists, app.debugDescription)
        XCTAssertFalse(blocked.isEnabled, "a runner this phone can't add a teammate on was tickable")
        XCTAssertTrue(
            app.staticTexts["Only an Owner or an Admin can add people to gpu-workstation."].exists,
            "the row didn't say why: \(app.debugDescription)")
        XCTAssertTrue(element(app, "teammate-runner-build-box").isEnabled)
    }

    func testAddTeammateWritesAsAMemberToTheTickedRunnerOnly() {
        let app = launch()
        _ = confirmation(app)
        element(app, "teammate-runner-build-box").tap()
        let add = element(app, "teammate-add")
        XCTAssertTrue(add.isEnabled, app.debugDescription)
        add.tap()
        XCTAssertTrue(
            app.staticTexts["Scan this code with Bob’s iPhone"].waitForExistence(timeout: 30),
            "no reply: \(app.debugDescription)")
        XCTAssertEqual(
            asked(app),
            "access=member handle=bob-smith name=Bob Smith account=user_bob runners=build-box")
    }

    func testViewerIsWhatIsSentWhenViewerIsChosen() {
        let app = launch()
        _ = confirmation(app)
        element(app, "teammate-access").buttons["Viewer"].tap()
        XCTAssertTrue(
            app.staticTexts["Can see the board and progress, but not terminals."].exists, app.debugDescription)
        element(app, "teammate-runner-build-box").tap()
        element(app, "teammate-add").tap()
        XCTAssertTrue(app.staticTexts["Scan this code with Bob’s iPhone"].waitForExistence(timeout: 30), app.debugDescription)
        XCTAssertTrue(asked(app).hasPrefix("access=viewer "), asked(app))
    }

    /// Admin is the Owner's to give: offered on a runner where this phone is the
    /// Owner, never where it is an Admin.
    func testAdminIsOfferedToTheOwnerAndToNoOneElse() {
        let owner = launch()
        _ = confirmation(owner)
        XCTAssertTrue(element(owner, "teammate-access").buttons["Admin"].exists, owner.debugDescription)
        element(owner, "teammate-access").buttons["Admin"].tap()
        element(owner, "teammate-runner-build-box").tap()
        element(owner, "teammate-add").tap()
        XCTAssertTrue(owner.staticTexts["Scan this code with Bob’s iPhone"].waitForExistence(timeout: 30), owner.debugDescription)
        XCTAssertTrue(asked(owner).hasPrefix("access=admin "), asked(owner))
        owner.terminate()

        let admin = launch(["-teammate-add-admin"])
        _ = confirmation(admin)
        XCTAssertTrue(element(admin, "teammate-access").buttons["Member"].exists, admin.debugDescription)
        XCTAssertFalse(element(admin, "teammate-access").buttons["Admin"].exists, "an Admin was offered Admin")
    }

    /// A person the runner already has under this account is picked to start,
    /// keeps the access they have, and New Person is the other choice.
    func testAPersonAlreadyHereIsPickedAndKeepsTheirAccess() {
        let app = launch(["-teammate-add-existing"])
        _ = confirmation(app)
        XCTAssertTrue(element(app, "teammate-person").exists, app.debugDescription)
        XCTAssertTrue(
            app.staticTexts["Bob is already on build-box as a Member, and keeps that access."].exists,
            app.debugDescription)
        XCTAssertFalse(element(app, "teammate-handle").exists, "a person who is here has no handle to type")
        element(app, "teammate-runner-build-box").tap()
        element(app, "teammate-add").tap()
        XCTAssertTrue(app.staticTexts["Scan this code with Bob’s iPhone"].waitForExistence(timeout: 30), app.debugDescription)
        XCTAssertEqual(asked(app), "access=member handle=bob name=Bob account=user_bob runners=build-box")
    }

    func testAHandleARunnerWouldRefuseKeepsAddTeammateOff() {
        let app = launch()
        _ = confirmation(app)
        element(app, "teammate-runner-build-box").tap()
        let handle = element(app, "teammate-handle")
        handle.tap()
        handle.press(forDuration: 1.0)
        if app.menuItems["Select All"].waitForExistence(timeout: 2) { app.menuItems["Select All"].tap() }
        handle.typeText("Bob Smith")
        XCTAssertTrue(
            element(app, "teammate-handle-note").waitForExistence(timeout: 5),
            "no word about the handle: \(app.debugDescription)")
        XCTAssertFalse(element(app, "teammate-add").isEnabled)
    }

    func testARunnerThatCouldNotBindIsNamedOnTheReply() {
        let app = launch(["-teammate-add-refused"])
        _ = confirmation(app)
        element(app, "teammate-runner-build-box").tap()
        element(app, "teammate-add").tap()
        XCTAssertTrue(app.staticTexts["Scan this code with Bob’s iPhone"].waitForExistence(timeout: 30), app.debugDescription)
        let said = NSPredicate(
            format: "label == %@",
            "build-box’s Far Cooler is too old to record whose device this is, so nothing was added there. "
                + "Update Far Cooler on it, then add your teammate again.")
        XCTAssertTrue(
            app.staticTexts.matching(said).firstMatch.exists,
            "the reply didn't say which runner didn't take it: \(app.debugDescription)")
    }

    func testAnAccountOnNoneOfYourTeamsIsRefusedWithTheWayIn() {
        let app = launch(["-teammate-add-stranger"])
        let title = app.staticTexts["This device’s account isn’t on any of your teams"]
        XCTAssertTrue(title.waitForExistence(timeout: XCUIApplication.firstFrame), app.debugDescription)
        XCTAssertTrue(
            app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "sign in to alice@example.com")).firstMatch.exists,
            app.debugDescription)
        XCTAssertFalse(element(app, "teammate-title").exists, "the runner list was reachable")
        XCTAssertEqual(asked(app), "")
    }

    func testAnUnreachableDirectoryRefusesAndWritesNothing() {
        let app = launch(["-teammate-add-offline"])
        XCTAssertTrue(
            app.staticTexts["Couldn’t check your teams"].waitForExistence(timeout: XCUIApplication.firstFrame),
            app.debugDescription)
        XCTAssertFalse(element(app, "teammate-title").exists)
        XCTAssertEqual(asked(app), "")
    }

    /// Long names wrap, and the buttons stay on screen to reach.
    func testLongNamesWrapAndAddTeammateIsReachable() {
        let app = launch(["-teammate-add-long"])
        let title = confirmation(app)
        let subtitle = element(app, "teammate-subtitle")
        XCTAssertTrue(subtitle.label.contains("bartholomew.montgomery-fitzgerald@"), subtitle.label)
        keep(app, "teammate-add-long", expecting: .shows(subtitle, "the confirmation with the long email"))
        XCTAssertEqual(title.label, "Add Your Teammate’s Device?")
        let add = element(app, "teammate-add")
        XCTAssertTrue(app.reveal(add), "Add Teammate is out of reach: \(app.debugDescription)")
    }
}
