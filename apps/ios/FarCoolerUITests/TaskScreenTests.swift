import XCTest

/// A task and the places its work is (ov-55 4A.3): the task screen's Agent,
/// Changes and Worktree rows, a notification landing with its workspace and
/// task under it, and a decision answered from Needs You.
///
/// No runner: `-phone-harness` stands the stack on a canned one
/// (`PhoneHarness`). bil-9 is in progress in `fc-3-webhooks`, with a claude
/// pane on it that's waiting on an ask; bil-7 waits on a decision. The
/// harness refuses an answer sent to the wrong task, or the wrong kind of
/// note, so a wired-wrong button leaves the item standing.
final class TaskScreenTests: XCTestCase {
    private static let agent = "0198f2c0-0000-7000-8000-00000000d002"
    private static let decision = "decision:0198f2c0-0000-7000-8000-00000000e007"

    private func launch(_ extra: [String]) -> XCUIApplication {
        .phoneHarness(extra)
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    /// The top screen's Back: the stack's own, or a pane's on a worktree.
    private func back(_ app: XCUIApplication) {
        let pane = app.buttons["worktree-back"].firstMatch
        if pane.exists && pane.isHittable {
            pane.tap()
        } else {
            app.navigationBars.buttons["BackButton"].firstMatch.tap()
        }
    }

    /// **Back from a task's agent returns to the task**, not to the board and
    /// not to wherever the shell last was: the agent was pushed over it.
    func testGoingToATasksAgentAndBackReturnsToTheTask() throws {
        let app = launch(["-phone-empty-inbox"])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        app.buttons["segment-board"].tap()
        let card = element(app, "board-card-bil-9")
        XCTAssertTrue(card.waitForExistence(timeout: 10), "bil-9 is not on the board")
        card.tap()

        let heading = element(app, "task-heading")
        XCTAssertTrue(heading.waitForExistence(timeout: 10), "the task did not open")
        XCTAssertTrue(heading.label.contains("bil-9"), heading.label)
        let agent = app.buttons["task-agent"]
        XCTAssertTrue(agent.waitForExistence(timeout: 5), "no Agent row")
        XCTAssertTrue(app.buttons["task-changes"].exists, "no Changes row")
        XCTAssertTrue(app.buttons["task-worktree"].exists, "no Worktree row")
        agent.tap()

        // The agent's worktree, with its pane's task named on its bar.
        let paneBack = app.buttons["worktree-back"].firstMatch
        XCTAssertTrue(paneBack.waitForExistence(timeout: 10), "the agent's worktree did not open")
        XCTAssertTrue(element(app, "pane-task-chip").waitForExistence(timeout: 5), "no task chip")
        back(app)

        XCTAssertTrue(heading.waitForExistence(timeout: 10), "Back did not return to the task")
        XCTAssertTrue(element(app, "task-heading").label.contains("bil-9"))
        XCTAssertFalse(app.buttons["worktree-back"].firstMatch.exists, "the worktree is still up")

        // The pane's task chip goes back to the task under it, rather than
        // stacking a second copy of it: Back from the task is the board.
        app.buttons["task-agent"].tap()
        // Every mounted pane has a bar, so the chip on screen is the one a
        // finger can reach.
        let chips = app.descendants(matching: .any).matching(identifier: "pane-task-chip")
        XCTAssertTrue(chips.firstMatch.waitForExistence(timeout: 10), "no task chip")
        let chip = try XCTUnwrap(
            chips.allElementsBoundByIndex.first { $0.isHittable }, "no chip on screen")
        chip.tap()
        XCTAssertTrue(heading.waitForExistence(timeout: 10), "the chip did not open the task")
        back(app)
        XCTAssertTrue(
            app.buttons["segment-board"].waitForExistence(timeout: 10),
            "Back from the task was not the board: the chip stacked a second task")
        XCTAssertFalse(element(app, "task-heading").exists)
    }

    /// **A notification lands on its pane with the workspace and task under
    /// it**, so Back walks up: the pane, bil-9, Billing, Needs You.
    func testANotificationTapLandsWithTheWorkspaceUnderIt() throws {
        let app = launch(["-deep-link", Self.agent])
        XCTAssertTrue(
            app.buttons["worktree-back"].firstMatch.waitForExistence(timeout: 30),
            "the link did not land on the pane: \(app.debugDescription)")
        back(app)
        let heading = element(app, "task-heading")
        XCTAssertTrue(heading.waitForExistence(timeout: 10), "no task under the pane")
        XCTAssertTrue(heading.label.contains("bil-9"), heading.label)
        back(app)
        XCTAssertTrue(
            app.buttons["segment-board"].waitForExistence(timeout: 10), "no workspace under the task")
        XCTAssertEqual(app.navigationBars.firstMatch.identifier, "Billing")
        back(app)
        XCTAssertTrue(
            app.navigationBars["Needs You"].waitForExistence(timeout: 10), "Needs You is not the root")
    }

    /// **Answering a decision from Needs You takes it off Needs You**, and
    /// leaves the ask beside it alone.
    func testAnsweringADecisionRemovesItFromNeedsYou() throws {
        let app = launch([])
        let item = element(app, "needs-you-item-\(Self.decision)")
        XCTAssertTrue(item.waitForExistence(timeout: 30), "the decision is not in Needs You")
        XCTAssertTrue(element(app, "needs-you-item-ask:hook-ask-1").exists, "the ask is missing")
        app.buttons["needs-you-action-\(Self.decision)-pdfkit"].tap()

        let gone = NSPredicate(format: "exists == false")
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation(for: gone, evaluatedWith: item)], timeout: 10),
            .completed, "the answered decision is still in Needs You")
        XCTAssertTrue(element(app, "needs-you-item-ask:hook-ask-1").exists, "the ask went too")
        XCTAssertFalse(
            element(app, "needs-you-failure-\(Self.decision)").exists, "the answer was refused")
        // The option it was answered with, not the other one, and not a title.
        XCTAssertEqual(element(app, "harness-sent").value as? String, "task.note bil-7 pdfkit")
    }

    /// **An answer someone else got to first says so**, on the row, in the
    /// runner's own terms (`not_held`), and the ask stays until the list moves.
    func testAnAnswerSomeoneElseGaveSaysSo() throws {
        let app = launch(["-phone-answer-taken"])
        let allow = app.buttons["needs-you-action-ask:hook-ask-1-allow"]
        XCTAssertTrue(allow.waitForExistence(timeout: 30), "no Allow on the ask")
        allow.tap()
        let failure = element(app, "needs-you-failure-ask:hook-ask-1")
        XCTAssertTrue(failure.waitForExistence(timeout: 10), "the refusal said nothing")
        XCTAssertEqual(failure.label, "Someone already answered this.")
    }

    /// **While a runner's list can't be read, Needs You shows its blocked
    /// agent**, as the widget counts it, rather than nothing.
    func testAnUnreadRunnersBlockedAgentIsShown() throws {
        let app = launch(["-phone-list-fails"])
        let item = element(app, "needs-you-item-blocked:\(Self.agent)")
        XCTAssertTrue(
            item.waitForExistence(timeout: 180),
            "the blocked agent isn't shown: \(app.debugDescription)")
        XCTAssertFalse(element(app, "needs-you-item-\(Self.decision)").exists)
    }

    /// **The glances count what Needs You counts**: the snapshot the widget,
    /// the complication and the watch read carries the list, and drops an
    /// answered decision with it (spec §7).
    func testTheGlancesCountWhatNeedsYouCounts() throws {
        let app = launch([])
        let probe = element(app, "snapshot-probe")
        XCTAssertTrue(probe.waitForExistence(timeout: 30), "no snapshot probe")
        func counted(_ n: Int) -> Bool {
            let deadline = Date().addingTimeInterval(10)
            while Date() < deadline {
                if probe.value as? String == "needsYou=\(n)" { return true }
                Thread.sleep(forTimeInterval: 0.3)
            }
            return false
        }
        XCTAssertTrue(counted(2), "the snapshot doesn't count 2: \(probe.value ?? "")")
        let answer = app.buttons["needs-you-action-\(Self.decision)-pdfkit"]
        XCTAssertTrue(answer.waitForExistence(timeout: 10))
        answer.tap()
        XCTAssertTrue(counted(1), "the snapshot kept the answered decision: \(probe.value ?? "")")
    }

    /// **A card says who is on it and when it starts** (ov-212, ov-213), from
    /// the one fixture's words: a running subagent is "Subagent", not "No
    /// Agent", and opens the orchestrator's pane; a card second in the build
    /// line says why it stopped and raises no alarm; a blocked card names its
    /// blocker. The task screen carries the same lines and a way to the
    /// orchestrator.
    func testACardSaysWhoIsOnItAndWhenItStarts() throws {
        let app = launch(["-phone-empty-inbox", "-phone-start-states"])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        app.buttons["segment-board"].tap()

        XCTAssertTrue(element(app, "board-card-bil-11").waitForExistence(timeout: 10))
        let subagent = element(app, "board-subagent-bil-11")
        XCTAssertTrue(subagent.exists, "a running subagent should read Subagent")
        XCTAssertFalse(element(app, "board-no-agent-bil-11").exists, "No Agent beside a subagent")
        XCTAssertTrue(
            element(app, "board-start-bil-11").label.hasPrefix("Claude subagent working, "),
            element(app, "board-start-bil-11").label)

        XCTAssertEqual(element(app, "board-start-bil-12").label, "Waiting to build, 2nd in line")
        XCTAssertFalse(element(app, "board-no-agent-bil-12").exists, "a ranked wait is no alarm")
        XCTAssertEqual(element(app, "board-blocked-bil-13").label, "Waiting on bil-9")

        element(app, "board-card-bil-11").tap()
        XCTAssertTrue(element(app, "task-heading").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "task-orchestrator").exists, "no way to the orchestrator")
    }
}

/// A phone with a Read grant (ov-55 4A fix): it sees what needs you and
/// where, and it can't answer, start or replace anything (spec §2.5).
final class ReadScopeTests: XCTestCase {
    private static let decision = "decision:0198f2c0-0000-7000-8000-00000000e007"

    /// **A Read grant sees Open, never an answer**, on Needs You, on the
    /// task, and on a workspace with no orchestrator.
    func testAReadScopedPhoneSeesNoAnswersAndNoOrchestratorControls() throws {
        let app = XCUIApplication.phoneHarness(["-phone-read-scope"])
        let item = app.descendants(matching: .any)["needs-you-item-\(Self.decision)"]
        XCTAssertTrue(item.waitForExistence(timeout: 180), "the decision is not in Needs You")
        XCTAssertTrue(app.buttons["needs-you-open-\(Self.decision)"].exists, "no Open")
        XCTAssertFalse(app.buttons["needs-you-answer-\(Self.decision)"].exists, "Answer… offered")
        XCTAssertTrue(app.buttons["needs-you-open-ask:hook-ask-1"].exists, "the ask has no Open")
        XCTAssertFalse(app.buttons["needs-you-action-ask:hook-ask-1-allow"].exists)

        app.buttons["workspace-row-Billing"].tap()
        XCTAssertTrue(app.buttons["segment-orchestrator"].waitForExistence(timeout: 10))
        app.buttons["segment-orchestrator"].tap()
        XCTAssertTrue(
            app.staticTexts["No Orchestrator"].waitForExistence(timeout: 10), "no empty state")
        XCTAssertFalse(app.buttons["start-orchestrator"].exists, "Start Orchestrator offered")

        app.buttons["segment-board"].tap()
        let card = app.descendants(matching: .any)["board-card-bil-7"]
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        card.tap()
        XCTAssertTrue(
            app.descendants(matching: .any)["task-question"].waitForExistence(timeout: 10),
            "the question isn't shown")
        XCTAssertFalse(app.buttons["task-answer-pdfkit"].exists, "an answer is offered")
    }
}
