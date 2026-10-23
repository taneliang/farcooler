import XCTest

/// Needs You on a member's phone, on a runner with a team (ov-633): their own
/// item under Yours, a teammate's and the owner's under Everyone Else, each naming
/// whose it is. Over the canned runner (`-phone-team`).
///
/// The captures are opt-in: with `FARCOOLER_CAPTURE_OUT`
/// (`TEST_RUNNER_FARCOOLER_CAPTURE_OUT`) set, light and dark go there as
/// `ios-needs-you-team-<appearance>.png`.
final class NeedsYouTeamUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func shoot(_ name: String, expecting state: CaptureState) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"].map({ URL(fileURLWithPath: $0) }) else { return }
        guard requireState(state, for: name) else { return }
        try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: out.appendingPathComponent("\(name).png"))
    }

    func testYoursComeFirstAndOthersNameWhoseTheyAre() {
        for appearance in ["light", "dark"] {
            let app = XCUIApplication.phoneHarness(["-phone-team", "-phone-appearance", appearance])
            let yours = element(app, "needs-you-group-yours")
            let others = element(app, "needs-you-group-others")
            XCTAssertTrue(yours.waitForExistence(timeout: 30), "no Yours header: \(app.debugDescription)")
            XCTAssertTrue(others.exists, "no Everyone Else header: \(app.debugDescription)")
            let ask = element(app, "needs-you-item-ask:hook-ask-1")
            XCTAssertTrue(ask.exists)
            XCTAssertLessThan(yours.frame.minY, ask.frame.minY, "the ask is under Yours")
            XCTAssertLessThan(ask.frame.maxY, others.frame.minY, "the ask comes before Everyone Else")
            let names = app.descendants(matching: .any).matching(identifier: "needs-you-whose")
            XCTAssertTrue(names.firstMatch.waitForExistence(timeout: 10))
            let said = names.allElementsBoundByIndex.map(\.label)
            XCTAssertTrue(said.contains { $0.contains("Bartholomew Okafor-Lindqvist") }, "\(said)")
            XCTAssertTrue(said.contains { $0.contains("Owner") }, "\(said)")
            XCTAssertFalse(said.contains { $0.contains("Alice") }, "your own item names nobody: \(said)")
            shoot("ios-needs-you-team-\(appearance)", expecting: .all(.shows(yours), .shows(others)))
            app.terminateRetrying()
        }
    }
}
