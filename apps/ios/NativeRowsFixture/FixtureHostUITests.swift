import XCTest

final class FixtureHostUITests: XCTestCase {
    func testVerifiedFixtureAndDisclosure() {
        let app = XCUIApplication(bundleIdentifier: "com.farcooler.fixture.native-rows")
        app.launch()
        guard app.staticTexts["fixture-ready-0f963393dc148ad45290ce4b90116d80ca76ef9294e72882cd3e287d5c4419de"].waitForExistence(timeout: 30),
              !app.staticTexts["fixture-error"].exists else { XCTFail("host did not validate fixture"); return }
        let group = app.descendants(matching: .any)["native-tool-group"].firstMatch
        XCTAssertTrue(group.exists)
        group.buttons.firstMatch.tap()
        XCTAssertTrue(app.descendants(matching: .any)["native-tool-tool:exec-fccc8116-d738-4d7f-ac0f-f655c8a13542"].waitForExistence(timeout: 5))
    }
}
