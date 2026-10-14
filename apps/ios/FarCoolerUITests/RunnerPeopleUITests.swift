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

    private func keep(_ app: XCUIApplication, _ name: String) {
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
        keep(app, "runner-people")
    }

    func testARunnerNobodySharesSaysNothingAboutPeople() {
        let app = runnerSettings([])
        // The section the People one would sit above, drawn: the screen read.
        XCTAssertTrue(app.staticTexts["Branches"].firstMatch.waitForExistence(timeout: 30) || app.staticTexts["BRANCHES"].firstMatch.waitForExistence(timeout: 5), "the screen never drew: \(app.debugDescription)")
        XCTAssertFalse(element(app, "runner-people-account").exists, "a private runner names an account")
    }
}
