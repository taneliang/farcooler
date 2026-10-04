import XCTest

/// First run and empty states on the phone (ov-205), over `-phone-harness`'s
/// canned runner: no runner, no daemon, so none of these can skip itself green.
/// Each flag takes one thing away from the fixture (see `PhoneHarness`).
///
/// Run through `scripts/ios-ui-tests.sh`; a UI test skips silently otherwise.
final class FirstRunUITests: XCTestCase {
    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func launch(_ extra: [String]) -> XCUIApplication {
        .phoneHarness(["-phone-empty-inbox"] + extra)
    }

    private func openWorkspace(_ app: XCUIApplication, _ name: String) {
        let row = app.buttons["workspace-row-\(name)"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no \(name) row: \(app.debugDescription)")
        row.tap()
        XCTAssertTrue(app.buttons["segment-board"].waitForExistence(timeout: 10))
    }

    /// **A phone with no runner says what a runner is, by its job.**
    func testOnboardingTeachesWhatARunnerIsFor() throws {
        let app = XCUIApplication.phoneHarness(["-phone-onboarding"])
        XCTAssertEqual(element(app, "onboarding-title").label, "Connect to Your Agents")
        let body = element(app, "onboarding-body").label
        XCTAssertTrue(body.contains("called a runner"), body)
        XCTAssertTrue(app.buttons["Connect This Device"].exists)
        XCTAssertTrue(app.buttons["More Ways to Add…"].exists)
    }

    /// **A runner with no repositories says so, and Add Repository… opens the
    /// sheet**, which says what a repository is for.
    func testARunnerWithNoRepositoriesOffersAddRepository() throws {
        let app = launch(["-phone-no-repositories"])
        let title = element(app, "no-repositories-title")
        XCTAssertTrue(title.waitForExistence(timeout: 30), "no block: \(app.debugDescription)")
        XCTAssertTrue(title.label.hasPrefix("No Repositories on "), title.label)
        // A lede and two icon rows, not a paragraph (ov-245).
        XCTAssertTrue(app.staticTexts["Add the repository you want agents to work in."].exists)
        XCTAssertEqual(element(app, "no-repositories-body").descendants(matching: .any).matching(identifier: "empty-row").count, 2)
        app.buttons["no-repositories-add"].tap()
        XCTAssertTrue(app.navigationBars["Add Repository"].waitForExistence(timeout: 10))
        XCTAssertTrue(
            app.staticTexts["Choose a Git repository on this runner for agents to work on."].exists)
    }

    /// **With repositories, the Needs You list has no such block.**
    func testARunnerWithRepositoriesHasNoBlock() throws {
        let app = launch([])
        XCTAssertTrue(app.buttons["workspace-row-Main"].waitForExistence(timeout: 30))
        XCTAssertFalse(element(app, "no-repositories").exists)
    }

    /// **A blank board with an orchestrator says to tell it, and offers no
    /// switch**: Main's orchestrator is running and its board is empty.
    func testABlankBoardWithAnOrchestratorSaysToTellIt() throws {
        let app = launch([])
        openWorkspace(app, "Main")
        app.buttons["segment-board"].tap()
        XCTAssertTrue(element(app, "board-empty").waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["No Tasks"].exists)
        XCTAssertTrue(app.staticTexts["The orchestrator fills this board."].exists)
        XCTAssertTrue(app.staticTexts["Tell it what you want done"].exists)
        XCTAssertFalse(app.staticTexts["Start the orchestrator first"].exists)
        XCTAssertFalse(app.buttons["board-show-orchestrator"].exists)
    }

    /// **A blank board with no orchestrator offers Show Orchestrator**, which
    /// switches to the segment that starts one.
    func testABlankBoardWithoutAnOrchestratorOffersShowOrchestrator() throws {
        let app = launch(["-phone-billing-blank"])
        openWorkspace(app, "Billing")
        app.buttons["segment-board"].tap()
        XCTAssertTrue(element(app, "board-empty").waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Start the orchestrator first"].exists)
        app.buttons["board-show-orchestrator"].tap()
        XCTAssertTrue(app.buttons["start-orchestrator"].waitForExistence(timeout: 10))
    }

    /// **No Orchestrator is a lede and three icon rows, not a paragraph**
    /// (ov-245), above the Start Orchestrator button.
    func testNoOrchestratorIsALedeAndIconRows() throws {
        let app = launch(["-phone-billing-blank"])
        openWorkspace(app, "Billing")
        app.buttons["segment-orchestrator"].tap()
        XCTAssertTrue(app.buttons["start-orchestrator"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["An orchestrator runs this workspace’s board."].exists)
        let rows = element(app, "empty-rows").descendants(matching: .any).matching(identifier: "empty-row")
        XCTAssertEqual(rows.count, 3)
        XCTAssertFalse(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Instead of running'")).firstMatch.exists)
    }

    private func openTask(_ app: XCUIApplication, _ key: String) {
        openWorkspace(app, "Billing")
        app.buttons["segment-board"].tap()
        let card = element(app, "board-card-\(key)")
        XCTAssertTrue(card.waitForExistence(timeout: 10), "no card: \(app.debugDescription)")
        card.tap()
        XCTAssertTrue(element(app, "task-screen").waitForExistence(timeout: 10))
    }

    /// **Ask the Orchestrator is off, saying what to do, with none running**
    /// (ov-241): never hidden, and never starts one.
    func testAskTheOrchestratorIsOffWithoutOne() throws {
        let app = launch([])
        openTask(app, "bil-9")
        let ask = app.buttons["ask-orchestrator"]
        XCTAssertTrue(ask.waitForExistence(timeout: 10), "no row: \(app.debugDescription)")
        XCTAssertFalse(ask.isEnabled)
        XCTAssertTrue(app.staticTexts["Start an orchestrator to ask about this task"].exists)
    }

    /// **With an orchestrator it's on, and when the runner won't paste into
    /// its terminal the reference is copied and the screen says so** (ov-241).
    func testAskTheOrchestratorCopiesWhenTheRunnerWontPaste() throws {
        let app = launch(["-phone-billing-led", "-phone-draft-refused"])
        openTask(app, "bil-9")
        let ask = app.buttons["ask-orchestrator"]
        XCTAssertTrue(ask.waitForExistence(timeout: 10), "no row: \(app.debugDescription)")
        XCTAssertTrue(ask.isEnabled)
        ask.tap()
        XCTAssertTrue(
            app.staticTexts["Copied a reference to bil-9. Paste it into the orchestrator."]
                .waitForExistence(timeout: 10),
            "no notice: \(app.debugDescription)")
    }

    /// **An agent the runner doesn't have is listed and can't be chosen.**
    func testAMissingAgentIsDisabledInTheStartMenu() throws {
        let app = launch(["-phone-claude-missing"])
        openWorkspace(app, "Billing")
        app.buttons["segment-orchestrator"].tap()
        app.buttons["start-orchestrator"].tap()
        let missing = app.buttons["Claude Code · Not Installed"]
        XCTAssertTrue(missing.waitForExistence(timeout: 5), "no row: \(app.debugDescription)")
        XCTAssertFalse(missing.isEnabled)
        XCTAssertTrue(app.buttons["Codex"].isEnabled)
    }

    /// **An orchestrator that ends at once with 127 says its agent isn't
    /// installed**, rather than "Orchestrator Stopped".
    func testAnExit127SaysTheAgentIsNotInstalled() throws {
        let app = launch(["-phone-start-127"])
        openWorkspace(app, "Billing")
        app.buttons["segment-orchestrator"].tap()
        app.buttons["start-orchestrator"].tap()
        let claude = app.buttons["Claude Code"]
        XCTAssertTrue(claude.waitForExistence(timeout: 5))
        claude.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(
            app.staticTexts["Claude Code Isn’t Installed"].waitForExistence(timeout: 30),
            "no not-installed state: \(app.debugDescription)")
        XCTAssertTrue(app.buttons["orchestrator-try-again"].exists)
        XCTAssertFalse(app.staticTexts["Orchestrator Stopped"].exists)
    }

    /// **Until this device signs in, Needs You says why notifications stop
    /// with the app.**
    func testNeedsYouSaysWhyNotificationsNeedSignIn() throws {
        let app = launch([])
        XCTAssertTrue(element(app, "needs-you-push").waitForExistence(timeout: 30))
        XCTAssertTrue(app.buttons["needs-you-sign-in"].exists)
    }
}
