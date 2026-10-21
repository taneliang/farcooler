import XCTest

/// A teammate's phone asks before it joins runners another account granted
/// it (ov-525): it names whose they are, joins only on Join, and adds nothing
/// on Cancel. Over the canned reply (`-teammate-join`).
final class TeammateJoinUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ arguments: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += ["-teammate-join"] + arguments
        app.launchDrawn()
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func keep(_ app: XCUIApplication, _ name: String, expecting state: CaptureState) {
        guard requireState(state, for: name) else { return }
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    func testOneRunnerIsNamedAndJoinedOnlyOnJoin() {
        let app = launch([])
        let title = element(app, "join-teammate-title")
        XCTAssertTrue(
            title.waitForExistence(timeout: XCUIApplication.firstFrame),
            "the phone didn't ask: \(app.debugDescription)")
        XCTAssertEqual(title.label, "Join Your Teammate’s Runner “build-box”?")
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "alice@example.com")).firstMatch.exists, "whose runners weren't said: \(app.debugDescription)")
        keep(app, "teammate-join-one", expecting: .reads(title, "Join Your Teammate’s Runner “build-box”?"))
        element(app, "join-teammate-join").tap()
        let joined = element(app, "teammate-joined")
        XCTAssertTrue(joined.waitForExistence(timeout: 30), "Join joined nothing: \(app.debugDescription)")
        XCTAssertEqual(joined.label, "Joined build-box")
    }

    func testSeveralRunnersAreListedAndCancelJoinsNone() {
        let app = launch(["-teammate-join-many"])
        let title = element(app, "join-teammate-title")
        XCTAssertTrue(title.waitForExistence(timeout: XCUIApplication.firstFrame), app.debugDescription)
        XCTAssertEqual(title.label, "Join Your Teammate’s Runners?")
        XCTAssertTrue(app.staticTexts["gpu-workstation"].exists, "a runner wasn't named: \(app.debugDescription)")
        element(app, "join-teammate-cancel").tap()
        XCTAssertTrue(element(app, "teammate-cancelled").waitForExistence(timeout: 30), app.debugDescription)
    }

    /// Long names wrap, and Join stays on screen to reach.
    func testLongNamesWrapAndJoinIsReachable() {
        let app = launch(["-teammate-join-long"])
        let title = element(app, "join-teammate-title")
        XCTAssertTrue(title.waitForExistence(timeout: XCUIApplication.firstFrame), app.debugDescription)
        XCTAssertFalse(title.label.contains("@"), "the email is in the headline: \(title.label)")
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "alexandra.montgomery-fitzgerald@")).firstMatch.exists, app.debugDescription)
        keep(app, "teammate-join-long", expecting: .shows(title, "the join prompt with the long email"))
        let join = element(app, "join-teammate-join")
        XCTAssertTrue(app.reveal(join), "Join is out of reach: \(app.debugDescription)")
        join.tap()
        XCTAssertTrue(element(app, "teammate-joined").waitForExistence(timeout: 30), app.debugDescription)
    }
}
