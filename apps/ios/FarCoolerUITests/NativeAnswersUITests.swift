import XCTest

/// A held ask answered from its row in the phone's conversation view
/// (ov-370, R-33), on `-native-agent-harness -native-held-ask <kind>`: the
/// real `NativeAgentView` over a canned runner reached through the client
/// core's own call path. Each button sends the runner's
/// `terminal.agent_answer` for the row's held id; the next follow shows the
/// ask answered on this phone. No runner.
final class NativeAnswersUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ kind: String, _ flags: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-native-agent-harness", "-native-held-ask", kind] + flags
        app.launchDrawn()
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// The screen, to `FARCOOLER_CAPTURE_OUT/held-<name>.png` when that's
    /// set (as `TEST_RUNNER_FARCOOLER_CAPTURE_OUT`): captures for a review,
    /// off in CI.
    private func capture(_ name: String) {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        let url = URL(fileURLWithPath: out).appendingPathComponent("held-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: url)
    }

    private func wait(_ timeout: TimeInterval, _ done: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if done() { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return done()
    }

    /// What the canned runner was answered: `answered=<ask> <option> <answers>|…`.
    private func answered(_ app: XCUIApplication) -> String {
        let said = element(app, "native-harness").value as? String ?? ""
        guard let range = said.range(of: "answered=") else { return "" }
        return String(said[range.upperBound...])
    }

    /// The ask's row, once the conversation shows it.
    private func ask(_ app: XCUIApplication) -> XCUIElement {
        // `NativeRowView` names each row by its id.
        let row = element(app, "native-row-ask:h1")
        XCTAssertTrue(row.waitForExistence(timeout: 60), "the held ask never showed")
        return row
    }

    private func tap(_ app: XCUIApplication, _ id: String) {
        let button = element(app, id)
        XCTAssertTrue(button.waitForExistence(timeout: 30), "\(id) isn't there")
        XCTAssertTrue(wait(30) { button.isHittable && button.isEnabled }, "\(id) can't be tapped")
        button.tap()
    }

    /// Allow sends the runner `allow` for the held id, once, and the row then
    /// says this phone answered it and offers nothing more.
    func testAPermissionIsAllowedFromItsRow() {
        let app = launch("permission")
        _ = ask(app)
        XCTAssertTrue(element(app, "native-ask-deny").exists, "Deny beside Allow")
        capture("permission")
        tap(app, "native-ask-allow")
        XCTAssertTrue(wait(30) { answered(app) == "hook-ask-h1 allow " }, "the runner heard \(answered(app))")
        XCTAssertTrue(wait(60) { !element(app, "native-ask-allow").exists }, "Allow still offered once answered")
        XCTAssertTrue(app.staticTexts["Answered on iPhone"].waitForExistence(timeout: 30))
        capture("permission-answered")
    }

    /// Send Answer waits for a pick, then sends the question's answer.
    func testAQuestionIsAnsweredWithAnOption() {
        let app = launch("question")
        _ = ask(app)
        let send = element(app, "native-ask-send-answer")
        XCTAssertTrue(send.waitForExistence(timeout: 30))
        XCTAssertFalse(send.isEnabled, "nothing picked yet")
        tap(app, "native-ask-option-0-1")
        capture("question")
        tap(app, "native-ask-send-answer")
        XCTAssertTrue(
            wait(30) { answered(app) == "hook-ask-h1 answer Which color should the button be?=Blue" },
            "the runner heard \(answered(app))")
    }

    /// The plan is on the row, and Keep Planning sends `deny`.
    func testAPlanIsKeptInPlanning() {
        let app = launch("plan")
        _ = ask(app)
        XCTAssertTrue(element(app, "native-ask-plan").waitForExistence(timeout: 30), "the plan isn't shown")
        XCTAssertTrue(element(app, "native-ask-approve").exists)
        capture("plan")
        tap(app, "native-ask-keep-planning")
        XCTAssertTrue(wait(30) { answered(app) == "hook-ask-h1 deny " }, "the runner heard \(answered(app))")
    }

    /// Answered elsewhere first: the row says so, and the buttons stay for
    /// the hold's own end to take away.
    func testAnAnswerTakenElsewhereIsSaid() {
        let app = launch("permission", ["-native-answer-taken"])
        _ = ask(app)
        tap(app, "native-ask-allow")
        XCTAssertTrue(element(app, "native-ask-issue").waitForExistence(timeout: 30), "the refusal isn't said")
        capture("taken")
    }
}
