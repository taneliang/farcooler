import XCTest

final class FixtureHostUITests: XCTestCase {
    func testVerifiedFixtureAndDisclosure() {
        let app = XCUIApplication(bundleIdentifier: "com.farcooler.fixture.native-rows")
        app.launch()
        guard app.staticTexts["fixture-ready-0f963393dc148ad45290ce4b90116d80ca76ef9294e72882cd3e287d5c4419de"].waitForExistence(timeout: 30),
              !app.staticTexts["fixture-error"].exists,
              app.alerts.count == 0 else { XCTFail("host did not validate fixture"); return }
        let group = app.descendants(matching: .any)["native-tool-group"].firstMatch
        guard group.exists else { XCTFail("fixture group is missing"); return }
        group.buttons.firstMatch.tap()
        let child = app.descendants(matching: .any)["native-tool-tool:exec-fccc8116-d738-4d7f-ac0f-f655c8a13542"].firstMatch
        guard child.waitForExistence(timeout: 5),
              app.state == .runningForeground,
              app.staticTexts["fixture-ready-0f963393dc148ad45290ce4b90116d80ca76ef9294e72882cd3e287d5c4419de"].exists,
              !app.staticTexts["fixture-error"].exists,
              app.alerts.count == 0 else { XCTFail("fixture disclosure is not eligible for capture"); return }
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "native-rows-disclosure"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }
}
