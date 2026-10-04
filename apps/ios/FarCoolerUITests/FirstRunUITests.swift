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
        XCTAssertTrue(element(app, "no-repositories-body").label.contains("Git repository"))
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
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Tell the orchestrator'"))
                .firstMatch.exists)
        XCTAssertFalse(app.buttons["board-show-orchestrator"].exists)
    }

    /// **A blank board with no orchestrator offers Show Orchestrator**, which
    /// switches to the segment that starts one.
    func testABlankBoardWithoutAnOrchestratorOffersShowOrchestrator() throws {
        let app = launch(["-phone-billing-blank"])
        openWorkspace(app, "Billing")
        app.buttons["segment-board"].tap()
        XCTAssertTrue(element(app, "board-empty").waitForExistence(timeout: 10))
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Start the orchestrator'"))
                .firstMatch.exists)
        app.buttons["board-show-orchestrator"].tap()
        XCTAssertTrue(app.buttons["start-orchestrator"].waitForExistence(timeout: 10))
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
