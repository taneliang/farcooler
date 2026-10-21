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

    /// A screenshot kept in the result bundle for review, once `state` shows: a shot of the wrong state fails the
    /// test and is not kept (ov-642).
    private func keep(_ name: String, expecting state: CaptureState) {
        guard requireState(state, for: name) else { return }
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func openWorkspace(_ app: XCUIApplication, _ name: String) {
        let row = app.buttons["workspace-row-\(name)"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no \(name) row: \(app.debugDescription)")
        row.tap()
        XCTAssertTrue(app.navigationBars[name].waitForExistence(timeout: 10), "\(name) didn't open")
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

    /// **A blank Plan with an orchestrator says it files the cards**: Main's
    /// orchestrator is running and its board is empty, so its Plan sheet's
    /// tree says who fills it, and offers nothing to start.
    func testABlankPlanWithAnOrchestratorSaysItFilesTheCards() throws {
        let app = launch([])
        openWorkspace(app, "Main")
        app.buttons["plan-strip"].tap()
        XCTAssertTrue(element(app, "tree-empty").waitForExistence(timeout: 10), "no empty tree: \(app.debugDescription)")
        XCTAssertEqual(element(app, "tree-empty").label, "No cards yet. The orchestrator files them as it plans the work.")
        XCTAssertFalse(app.buttons["start-orchestrator"].exists)
    }

    /// **A blank workspace with no orchestrator offers Start Orchestrator**
    /// at the top of its Plan, which is its screen (ov-616).
    func testABlankWorkspaceWithoutAnOrchestratorOffersStart() throws {
        let app = launch(["-phone-billing-blank"])
        openWorkspace(app, "Billing")
        XCTAssertTrue(app.buttons["start-orchestrator"].waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "tree-empty").waitForExistence(timeout: 10), "no empty tree under it")
    }

    /// **An empty Needs You with no orchestrator anywhere offers the next
    /// step** (ov-519): Start Orchestrator opens the workspace on the screen
    /// that starts one, rather than leaving "start an orchestrator" as text.
    func testAnEmptyNeedsYouWithNoOrchestratorOffersStartOrchestrator() throws {
        let app = launch(["-phone-no-orchestrators"])
        XCTAssertTrue(element(app, "needs-you-no-orchestrator").waitForExistence(timeout: 30), "no words: \(app.debugDescription)")
        let show = app.buttons["needs-you-start-orchestrator"]
        XCTAssertTrue(show.exists, "no button: \(app.debugDescription)")
        XCTAssertEqual(show.label, "Start Orchestrator")
        keep("needs-you-empty-no-orchestrator", expecting: .reads(show, "Start Orchestrator"))
        show.tap()
        XCTAssertTrue(app.buttons["start-orchestrator"].waitForExistence(timeout: 10), "not on Start: \(app.debugDescription)")
        keep("show-orchestrator-lands-on-start", expecting: .shows(app.buttons["start-orchestrator"]))
    }

    /// **The same invitation in dark**: the button is there and readable, and
    /// the lede names the orchestrator's job on the screen it leads to.
    func testTheNoOrchestratorInvitationHoldsInDark() throws {
        let app = launch(["-phone-no-orchestrators", "-phone-appearance", "dark"])
        XCTAssertTrue(app.buttons["needs-you-start-orchestrator"].waitForExistence(timeout: 30), "no button: \(app.debugDescription)")
        keep("needs-you-empty-no-orchestrator-dark", expecting: .shows(app.buttons["needs-you-start-orchestrator"]))
        app.buttons["needs-you-start-orchestrator"].tap()
        XCTAssertTrue(app.buttons["start-orchestrator"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["The orchestrator plans the work and starts agents on it."].exists)
        keep("start-orchestrator-dark", expecting: .shows(app.staticTexts["The orchestrator plans the work and starts agents on it."]))
    }

    /// **With an orchestrator running, the empty Needs You offers no such
    /// button**: there is nothing to start.
    func testAnEmptyNeedsYouWithAnOrchestratorOffersNoButton() throws {
        let app = launch([])
        XCTAssertTrue(element(app, "needs-you-nothing").waitForExistence(timeout: 30), "no empty state: \(app.debugDescription)")
        XCTAssertFalse(app.buttons["needs-you-start-orchestrator"].exists)
        XCTAssertFalse(element(app, "needs-you-no-orchestrator").exists)
    }

    /// **No Orchestrator is a lede and two icon rows, not a paragraph**
    /// (ov-245), above the Start Orchestrator button.
    func testNoOrchestratorIsALedeAndIconRows() throws {
        let app = launch(["-phone-billing-blank"])
        openWorkspace(app, "Billing")
        XCTAssertTrue(app.buttons["start-orchestrator"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["The orchestrator plans the work and starts agents on it."].exists)
        let rows = element(app, "empty-rows").descendants(matching: .any).matching(identifier: "empty-row")
        XCTAssertEqual(rows.count, 2)
        XCTAssertFalse(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Instead of running'")).firstMatch.exists)
    }

    private func openTask(_ app: XCUIApplication, _ key: String) {
        openWorkspace(app, "Billing")
        // Led, Billing is its conversation, and its Plan is the sheet (ov-616).
        let card = app.buttons["plan-strip"]
        if card.waitForExistence(timeout: 5) {
            card.tap()
            let bar = app.navigationBars["Plan"]
            if bar.waitForExistence(timeout: 10) { bar.swipeUp() }
        }
        for id in ["tree-row-No Theme", "tree-row-\(key)", "tree-own"] {
            if !app.buttons[id].waitForExistence(timeout: 5) { app.reveal(app.buttons[id]) }
            XCTAssertTrue(app.buttons[id].waitForExistence(timeout: 10), "no \(id): \(app.debugDescription)")
            app.buttons[id].tap()
        }
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

    /// **Ask the Orchestrator from a task lands in the conversation** (ov-664):
    /// the Plan sheet the task was open in closes onto it, the draft is pasted,
    /// and no raw worktree pane covers it (it has a Back).
    func testAskTheOrchestratorLandsInTheConversation() throws {
        let app = launch(["-phone-billing-led"])
        openTask(app, "bil-9")
        let ask = app.buttons["ask-orchestrator"]
        XCTAssertTrue(ask.waitForExistence(timeout: 10), "no row: \(app.debugDescription)")
        ask.tap()
        let drafted = NSPredicate { _, _ in
            ((self.element(app, "harness-sent").value as? String) ?? "").contains("draft billing About bil-9")
        }
        expectation(for: drafted, evaluatedWith: nil)
        waitForExpectations(timeout: 10)
        XCTAssertTrue(element(app, "workspace-body").waitForExistence(timeout: 10), "the conversation isn't showing")
        XCTAssertTrue(app.navigationBars["Plan"].waitForNonExistence(timeout: 10), "the Plan sheet stayed up")
        XCTAssertFalse(app.buttons["worktree-back"].firstMatch.exists, "a raw worktree pane covers the conversation")
        Thread.sleep(forTimeInterval: 1)
        keep("ask-orchestrator-lands-in-chat", expecting: .all(.shows(element(app, "workspace-body")), .gone(app.navigationBars["Plan"])))
    }

    /// **A dialog in the orchestrator's pane holds the draft, and the pane
    /// says so with Withdraw rather than the reference being copied**
    /// (ov-385). Withdraw reaches the runner, and the bar goes.
    func testAskTheOrchestratorWaitsForADialogAndCanBeWithdrawn() throws {
        let app = launch(["-phone-billing-led", "-phone-draft-held"])
        openTask(app, "bil-9")
        let ask = app.buttons["ask-orchestrator"]
        XCTAssertTrue(ask.waitForExistence(timeout: 10), "no row: \(app.debugDescription)")
        ask.tap()
        XCTAssertTrue(
            app.staticTexts["Waiting for the dialog to close"].waitForExistence(timeout: 10),
            "no waiting bar: \(app.debugDescription)")
        XCTAssertFalse(app.staticTexts["Copied a reference to bil-9. Paste it into the orchestrator."].exists)
        // The waiting state, kept in the result bundle for review.
        keep("held-draft-waiting", expecting: .shows(app.staticTexts["Waiting for the dialog to close"]))
        // The orchestrator's pane is mounted under its worktree's cover as
        // well as in it (ov-616): the one on screen is the one to tap.
        let withdraw = app.buttons.matching(identifier: "held-draft-withdraw").allElementsBoundByIndex.first { $0.isHittable }
        XCTAssertNotNil(withdraw, "no Withdraw on screen")
        withdraw?.tap()
        XCTAssertTrue(
            app.staticTexts["Waiting for the dialog to close"].firstMatch.waitForNonExistence(timeout: 10),
            "still waiting: \(app.debugDescription)")
    }

    /// **An agent the runner doesn't have is listed and can't be chosen.**
    func testAMissingAgentIsDisabledInTheStartMenu() throws {
        let app = launch(["-phone-claude-missing"])
        openWorkspace(app, "Billing")
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
