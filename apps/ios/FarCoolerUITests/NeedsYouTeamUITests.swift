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
            XCTAssertFalse(said.contains { $0.contains("You") }, "Yours already says it, so the row doesn't repeat it: \(said)")
            XCTAssertFalse(said.contains { $0.contains("Alice") }, "your own item doesn't use your name: \(said)")
            shoot("ios-needs-you-team-\(appearance)", expecting: .all(.shows(yours), .shows(others)))
            app.terminate()
        }
    }

    /// Bob's decision is not this phone's: tapping an answer asks first, and
    /// Cancel sends nothing (ov-737).
    func testAnsweringForSomeoneElseAsksFirstAndCancelSendsNothing() {
        let app = XCUIApplication.phoneHarness(["-phone-team"])
        let option = app.buttons["needs-you-action-decision:0198f2c0-0000-7000-8000-00000000e007-pdfkit"]
        XCTAssertTrue(option.waitForExistence(timeout: 30), "no answer on Bob's decision: \(app.debugDescription)")
        option.tap()
        let prompt = app.alerts["This is for Bartholomew Okafor-Lindqvist. Answer for them?"]
        XCTAssertTrue(prompt.waitForExistence(timeout: 10), "no question before answering for Bob: \(app.debugDescription)")
        XCTAssertTrue(prompt.buttons["Answer"].exists, "no Answer in the question")
        prompt.buttons["Cancel"].tap()
        XCTAssertFalse(prompt.waitForExistence(timeout: 2), "the question stayed")
        XCTAssertNotEqual(element(app, "harness-sent").value as? String, "task.note bil-7 pdfkit", "Cancel sent the answer")
        XCTAssertTrue(option.exists, "Bob's decision left on Cancel")
        app.terminate()
    }

    /// Confirming sends it; and this phone's own ask answers with no question.
    func testConfirmingSendsAndYourOwnAsksNothing() {
        let app = XCUIApplication.phoneHarness(["-phone-team"])
        let allow = app.buttons["needs-you-action-ask:hook-ask-1-allow"]
        XCTAssertTrue(allow.waitForExistence(timeout: 30), "no Allow on your ask")
        let option = app.buttons["needs-you-action-decision:0198f2c0-0000-7000-8000-00000000e007-pdfkit"]
        option.tap()
        let prompt = app.alerts["This is for Bartholomew Okafor-Lindqvist. Answer for them?"]
        XCTAssertTrue(prompt.waitForExistence(timeout: 10), "no question before answering for Bob")
        prompt.buttons["Answer"].firstMatch.tap()
        let sent = NSPredicate(format: "value == %@", "task.note bil-7 pdfkit")
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation(for: sent, evaluatedWith: element(app, "harness-sent"))], timeout: 10),
            .completed, "confirming didn't send")
        allow.tap()
        let ask = element(app, "needs-you-item-ask:hook-ask-1")
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: ask)], timeout: 10),
            .completed, "your own ask didn't go on one tap (it asked first?)")
        app.terminate()
    }

    private static let decisionID = "decision:0198f2c0-0000-7000-8000-00000000e007"
    private static let forBob = "This is for Bartholomew Okafor-Lindqvist. Answer for them?"

    /// A decision with no options is answered in one alert for someone else's:
    /// its title asks, and Answer sends what was typed (ov-737).
    func testATypedAnswerForSomeoneElseAsksInTheAlertItself() {
        let app = XCUIApplication.phoneHarness(["-phone-team", "-phone-free-answer"])
        let write = app.buttons["needs-you-answer-\(Self.decisionID)"]
        XCTAssertTrue(write.waitForExistence(timeout: 30), "no Answer… on Bob's decision: \(app.debugDescription)")
        write.tap()
        let alert = app.alerts[Self.forBob]
        XCTAssertTrue(alert.waitForExistence(timeout: 10), "no question before answering for Bob: \(app.debugDescription)")
        alert.textFields.firstMatch.typeText("pdfkit")
        alert.buttons["Answer"].firstMatch.tap()
        let sent = NSPredicate(format: "value == %@", "task.note bil-7 pdfkit")
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation(for: sent, evaluatedWith: element(app, "harness-sent"))], timeout: 10),
            .completed, "Answer didn't send what was typed")
        app.terminate()
    }

    /// The task page says whose Bob's decision is and asks before answering it.
    func testTheTaskPageSaysWhoseAndAsksFirst() {
        let app = XCUIApplication.phoneHarness(["-phone-team"])
        let item = element(app, "needs-you-item-\(Self.decisionID)")
        XCTAssertTrue(item.waitForExistence(timeout: 30), "no Bob's decision: \(app.debugDescription)")
        item.tap()
        let whose = element(app, "task-whose")
        XCTAssertTrue(whose.waitForExistence(timeout: 10), "the task page doesn't say whose: \(app.debugDescription)")
        XCTAssertTrue(whose.label.contains("Bartholomew Okafor-Lindqvist"), whose.label)
        app.buttons["task-option-0"].tap()
        app.buttons["task-answer-send"].tap()
        let alert = app.alerts[Self.forBob]
        XCTAssertTrue(alert.waitForExistence(timeout: 10), "no question on the task page: \(app.debugDescription)")
        alert.buttons["Cancel"].tap()
        XCTAssertNotEqual(element(app, "harness-sent").value as? String, "task.note bil-7 pdfkit", "Cancel sent it")
        app.buttons["task-answer-send"].tap()
        app.alerts[Self.forBob].buttons["Answer"].firstMatch.tap()
        let sent = NSPredicate(format: "value == %@", "task.note bil-7 pdfkit")
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation(for: sent, evaluatedWith: element(app, "harness-sent"))], timeout: 10),
            .completed, "confirming didn't send")
        app.terminate()
    }
}
