import XCTest

/// A shared runner's settings say whose Claude account the work runs on, and
/// list its people with their reps (ov-508); a runner nobody shares says
/// neither. Over the canned runner (`-phone-shared`).
final class RunnerPeopleUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    /// Settings, then Runner Settings, as a reader gets there.
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

    func testASharedRunnerSaysWhoseAccountTheWorkRunsOn() {
        let app = runnerSettings(["-phone-shared"])
        let line = element(app, "runner-people-account")
        XCTAssertTrue(line.waitForExistence(timeout: 30), "no account line: \(app.debugDescription)")
        XCTAssertEqual(line.label, "Work here runs on alice@example.com’s Claude account.")
        let alexandra = element(app, "runner-person-alexandra")
        XCTAssertTrue(app.reveal(alexandra), "no row for Alexandra: \(app.debugDescription)")
        XCTAssertTrue(alexandra.label.contains("Reps in Main and Billing"), alexandra.label)
        let bob = element(app, "runner-person-bob")
        XCTAssertTrue(app.reveal(bob), "no row for Bob: \(app.debugDescription)")
        XCTAssertTrue(bob.label.contains("No rep") && bob.label.contains("You"), bob.label)
        // Added by QR, so his row says the email he was added under (ov-525).
        XCTAssertTrue(bob.label.contains("bob@example.com"), bob.label)
        keep(app, "runner-people", expecting: .reads(bob, "bob@example.com"))
    }

    /// A long email wraps on its own line of the row rather than pushing the
    /// rest off (ov-525).
    func testATeammatesLongEmailIsShownWhole() {
        let app = runnerSettings(["-phone-shared", "-phone-rep-long"])
        let bob = element(app, "runner-person-bob")
        XCTAssertTrue(app.reveal(bob), "no row for Bob: \(app.debugDescription)")
        XCTAssertTrue(
            bob.label.contains("bob.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"), bob.label)
        keep(app, "runner-people-long", expecting: .reads(bob, "bob.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"))
    }

    private func sent(_ app: XCUIApplication) -> [String] {
        (element(app, "harness-sent").value as? String).map { $0.split(separator: "\n").map(String.init) } ?? []
    }

    /// The owner's phone offers a role on each member, and choosing one asks
    /// the runner by handle (ov-784).
    func testTheOwnerSetsAMembersRole() {
        let app = runnerSettings(["-phone-roles", "-phone-rep-owner"])
        let picker = element(app, "runner-person-role-bob")
        XCTAssertTrue(app.reveal(picker), "no role picker for Bob: \(app.debugDescription)")
        XCTAssertTrue(element(app, "runner-person-role-alexandra").exists, "no role picker for Alexandra")
        keep(app, "runner-people-roles", expecting: .shows(picker))
        picker.tap()
        let maintainer = app.buttons["Maintainer"].firstMatch
        XCTAssertTrue(maintainer.waitForExistence(timeout: 10), "no Maintainer in the menu: \(app.debugDescription)")
        maintainer.tap()
        let deadline = Date().addingTimeInterval(10)
        while !sent(app).contains("role bob maintainer"), Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        XCTAssertTrue(sent(app).contains("role bob maintainer"), "\(sent(app))")
    }

    /// The owner's People with each member's picker in their own row, long
    /// names, in Light and Dark (ov-784 review 4).
    func testRolePickerCaptures() {
        for appearance in ["light", "dark"] {
            let app = runnerSettings(["-phone-roles", "-phone-rep-owner", "-phone-rep-long", "-phone-appearance", appearance])
            let picker = element(app, "runner-person-role-bob")
            XCTAssertTrue(app.reveal(picker), "no role picker for Bob: \(app.debugDescription)")
            let alexandra = element(app, "runner-person-role-alexandra")
            XCTAssertTrue(alexandra.exists, "no role picker for Alexandra")
            // The picker sits in its member's row: level with the name, not below it.
            let row = element(app, "runner-person-alexandra")
            XCTAssertTrue(alexandra.frame.minY < row.frame.maxY && alexandra.frame.maxY > row.frame.minY, "Alexandra's picker isn't in her row: \(alexandra.frame) vs \(row.frame)")
            keep(app, "runner-people-roles-\(appearance)", expecting: .shows(picker))
            app.terminate()
        }
    }

    /// A member's phone sees who is a Maintainer and no picker.
    func testAMemberSeesRolesAndNoPicker() {
        let app = runnerSettings(["-phone-roles"])
        let alexandra = element(app, "runner-person-alexandra")
        XCTAssertTrue(app.reveal(alexandra), "no row for Alexandra: \(app.debugDescription)")
        XCTAssertTrue(alexandra.label.contains("Maintainer"), alexandra.label)
        XCTAssertFalse(element(app, "runner-person-role-bob").exists, "a member is offered a picker")
    }

    /// A removed member's phone says so on its runner's row (ov-784).
    func testARemovedPhoneIsToldSo() {
        let app = XCUIApplication.phoneHarness(["-phone-released"])
        let removed = element(app, "runner-removed")
        XCTAssertTrue(removed.waitForExistence(timeout: 30), "no removed row: \(app.debugDescription)")
        XCTAssertEqual(removed.label, "You Were Removed from This Runner")
        keep(app, "runner-removed", expecting: .shows(removed))
    }

    func testARunnerNobodySharesSaysNothingAboutPeople() {
        let app = runnerSettings([])
        // The section the People one would sit above, drawn: the screen read.
        XCTAssertTrue(app.staticTexts["Branches"].firstMatch.waitForExistence(timeout: 30) || app.staticTexts["BRANCHES"].firstMatch.waitForExistence(timeout: 5), "the screen never drew: \(app.debugDescription)")
        XCTAssertFalse(element(app, "runner-people-account").exists, "a private runner names an account")
    }
}
