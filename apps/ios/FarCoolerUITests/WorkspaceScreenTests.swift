import XCTest

/// The phone's workspace screen (ov-55 4A.2): Orchestrator, Board and
/// Worktrees, one at a time, under the workspace's name.
///
/// No runner and no daemon: `-phone-harness` stands the app's own stack on a
/// canned runner (`PhoneHarness`) with two workspaces, Main, whose
/// orchestrator is running, and Billing, which has none. Its answers check
/// what they're sent, so a screen sending the wrong workspace is refused and
/// the test watching it fails. Nothing here waits on a connection, and
/// nothing skips: every wait that times out is a failure.
final class WorkspaceScreenTests: XCTestCase {
    private func launch(_ extra: [String] = []) -> XCUIApplication {
        .phoneHarness(["-phone-empty-inbox"] + extra)
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    /// Open a workspace from Needs You's Workspaces list.
    private func openWorkspace(_ app: XCUIApplication, _ name: String) {
        let row = app.buttons["workspace-row-\(name)"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no \(name) row: \(app.debugDescription)")
        row.tap()
        XCTAssertTrue(
            app.buttons["segment-board"].waitForExistence(timeout: 10),
            "the workspace screen did not open")
    }

    private func choose(_ app: XCUIApplication, _ segment: String) {
        let button = app.buttons["segment-\(segment.lowercased())"]
        XCTAssertTrue(button.waitForExistence(timeout: 5), "no \(segment) segment")
        button.tap()
    }

    /// **Three segments, each showing its own thing**: the orchestrator's
    /// pane, the board as a list with Needs Decision first, and the
    /// workspace's worktrees, with the one it doesn't own left out.
    func testWorkspaceShowsOrchestratorBoardAndWorktrees() throws {
        let app = launch()
        openWorkspace(app, "Main")
        // Main's orchestrator is running, and a workspace opens on it.
        XCTAssertTrue(element(app, "orchestrator-pane").waitForExistence(timeout: 10))
        XCTAssertEqual(app.navigationBars.firstMatch.identifier, "Main")

        app.navigationBars.buttons.firstMatch.tap()
        openWorkspace(app, "Billing")
        choose(app, "Board")
        let decision = element(app, "board-section-needs_decision")
        let progress = element(app, "board-section-in_progress")
        XCTAssertTrue(decision.waitForExistence(timeout: 10), "the board did not show")
        XCTAssertTrue(progress.exists)
        XCTAssertLessThan(decision.frame.minY, progress.frame.minY, "Needs Decision is not first")
        XCTAssertTrue(element(app, "board-card-bil-7").exists, "bil-7 is not on the board")
        XCTAssertTrue(element(app, "board-card-bil-9").exists, "bil-9 is not on the board")

        choose(app, "Worktrees")
        XCTAssertTrue(app.buttons["worktree-row-fc-3-webhooks"].waitForExistence(timeout: 5))
        // Unclaimed, and Main's checkout: neither is Billing's.
        XCTAssertFalse(app.buttons["worktree-row-scratch"].exists)
        XCTAssertFalse(app.buttons["worktree-row-overnight"].exists)
        XCTAssertTrue(app.buttons["new-worktree"].exists, "no New Worktree…")

        // The choice is kept per workspace: Billing comes back on Worktrees.
        app.navigationBars.buttons.firstMatch.tap()
        openWorkspace(app, "Billing")
        XCTAssertTrue(app.buttons["worktree-row-fc-3-webhooks"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["segment-worktrees"].isSelected)
    }

    /// **A workspace with no orchestrator can start one from the phone**
    /// (ruling 8): the harness is chosen from a menu, the screen says it's
    /// starting, and the pane takes its place once the runner has it.
    func testStartOrchestratorFromAnEmptyWorkspace() throws {
        let app = launch()
        openWorkspace(app, "Billing")
        choose(app, "Orchestrator")
        let start = app.buttons["start-orchestrator"]
        XCTAssertTrue(start.waitForExistence(timeout: 10), "no Start Orchestrator")
        XCTAssertFalse(element(app, "orchestrator-pane").exists)
        start.tap()
        let claude = app.buttons["Claude"]
        XCTAssertTrue(claude.waitForExistence(timeout: 5), "no harness menu")
        // A menu's rows report themselves unhittable while it opens.
        claude.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(
            element(app, "orchestrator-starting").waitForExistence(timeout: 6),
            "no Starting Orchestrator…")
        XCTAssertTrue(
            element(app, "orchestrator-pane").waitForExistence(timeout: 20),
            "the orchestrator's pane did not appear: \(app.debugDescription)")
        XCTAssertFalse(element(app, "orchestrator-refusal").exists, "the runner refused the start")
        // A start, not a replace: the menu's harness, and replace false.
        XCTAssertEqual(
            element(app, "harness-sent").value as? String, "start billing claude replace=false")
    }

    /// **The orchestrator's pane stays mounted while the board is up**
    /// (ov-66, ruling 2): back on Orchestrator it's the same pane, not one
    /// built again, which is what would reconnect it and lose its place.
    func testTheOrchestratorStaysMountedAcrossSegments() throws {
        let app = launch()
        openWorkspace(app, "Main")
        let mount = element(app, "orchestrator-mount")
        XCTAssertTrue(mount.waitForExistence(timeout: 10), "no orchestrator pane")
        let before = try XCTUnwrap(mount.value as? String)
        choose(app, "Board")
        XCTAssertTrue(element(app, "board").waitForExistence(timeout: 10), "the board did not show")
        choose(app, "Worktrees")
        choose(app, "Orchestrator")
        XCTAssertTrue(mount.waitForExistence(timeout: 10), "the pane did not come back")
        XCTAssertEqual(mount.value as? String, before, "the pane was built again")
    }

    /// **A second tap on Add Task while the first is out files nothing
    /// more.** The runner takes a second to answer; the button is gone
    /// under the finger for that second, and one task is filed.
    func testAddTaskTappedTwiceFilesOneTask() throws {
        let app = launch()
        openWorkspace(app, "Billing")
        choose(app, "Board")
        let add = app.buttons["new-task"]
        XCTAssertTrue(add.waitForExistence(timeout: 10), "no New Task…")
        add.tap()
        let title = app.textFields["new-task-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5), "no New Task sheet")
        title.tap()
        title.typeText("Email the invoice")
        // Where the button is now, fixed: after the first tap there's a
        // spinner there instead, and the second tap lands on whatever is.
        let frame = app.buttons["new-task-add"].frame
        let spot = app.coordinate(withNormalizedOffset: .zero)
            .withOffset(CGVector(dx: frame.midX, dy: frame.midY))
        // Both touches in one gesture: two `tap()`s would wait out the
        // first create between them, which is no test at all.
        spot.doubleTap()
        XCTAssertTrue(
            element(app, "board-card-bil-20").waitForExistence(timeout: 10),
            "the filed task isn't on the board")
        // Past the second create's answer, had there been one.
        Thread.sleep(forTimeInterval: 2)
        XCTAssertEqual(
            element(app, "harness-sent").value as? String, "task.create billing Email the invoice",
            "Add Task filed more than once")
        XCTAssertFalse(element(app, "board-card-bil-21").exists, "a second task was filed")
    }

    /// **Covered, the orchestrator stops; back on it, it's read again**
    /// (ov-66 fix). A decision push over Billing's orchestrator pushes the
    /// task: the pane stays mounted and stops streaming. Back is Billing's
    /// orchestrator again, streaming and held as the pane being read.
    func testTheOrchestratorPausesUnderATaskAndResumesOnBack() throws {
        let app = launch(["-phone-billing-led"])
        openWorkspace(app, "Billing")
        let probe = phoneProbe(app)
        XCTAssertTrue(wait(probe, "live=\(Self.billingOrchestrator) watch=\(Self.billingOrchestrator)"))

        Self.notify("decision")
        XCTAssertTrue(
            element(app, "task-heading").waitForExistence(timeout: 10), "the push opened nothing")
        XCTAssertTrue(wait(probe, "live= watch="), "still streaming under the task: \(probe.value ?? "")")

        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        XCTAssertTrue(
            wait(probe, "live=\(Self.billingOrchestrator) watch=\(Self.billingOrchestrator)"),
            "not read again on Back: \(probe.value ?? "")")
    }

    /// **Under a worktree, the same**: an agent's notification covers the
    /// stack with its worktree, the orchestrator stops, and two Backs (the
    /// worktree, then its task) bring it back live and read.
    func testTheOrchestratorPausesUnderAWorktreeAndResumesOnBack() throws {
        let app = launch(["-phone-billing-led"])
        openWorkspace(app, "Billing")
        let probe = phoneProbe(app)
        XCTAssertTrue(wait(probe, "live=\(Self.billingOrchestrator) watch=\(Self.billingOrchestrator)"))

        Self.notify("agent")
        let paneBack = app.buttons["worktree-back"].firstMatch
        XCTAssertTrue(paneBack.waitForExistence(timeout: 10), "the agent's worktree did not open")
        let covered = phoneProbe(app)
        XCTAssertTrue(
            wait(covered, "live= watch=\(Self.agent)"),
            "still streaming under the worktree: \(covered.value ?? "")")

        paneBack.tap()
        XCTAssertTrue(element(app, "task-heading").waitForExistence(timeout: 10), "no task under it")
        app.navigationBars.buttons["BackButton"].firstMatch.tap()
        XCTAssertTrue(
            wait(probe, "live=\(Self.billingOrchestrator) watch=\(Self.billingOrchestrator)"),
            "not read again on Back: \(probe.value ?? "")")
    }

    private static let billingOrchestrator = "0198f2c0-0000-7000-8000-00000000d004"

    /// A notification tapped while the app is open, as the harness takes
    /// one: a Darwin notification, `com.farcooler.harness.<which>`. Not a
    /// tap on a control, which never reached one laid over a terminal, and
    /// not `XCUIApplication.open(_:)`, which relaunches the app.
    private static func notify(_ which: String) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName("com.farcooler.harness.\(which)" as CFString), nil, nil, true)
    }

    private static let agent = "0198f2c0-0000-7000-8000-00000000d002"

    private func phoneProbe(_ app: XCUIApplication) -> XCUIElement {
        let probe = app.descendants(matching: .any).matching(identifier: "phone-probe").firstMatch
        XCTAssertTrue(probe.waitForExistence(timeout: 10), "no phone probe")
        return probe
    }

    /// Whether `probe` reads `live=… watch=…` as given, within ten seconds.
    private func wait(_ probe: XCUIElement, _ prefix: String) -> Bool {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if (probe.value as? String ?? "").hasPrefix(prefix + " kept=") { return true }
            Thread.sleep(forTimeInterval: 0.25)
        }
        return false
    }

    /// **New Task… files a task on the board** (ov-66, ruling 4): a title
    /// past 200 scalars says the Mac's line and can't be added, a refusal
    /// is a sentence that keeps the sheet up, and a filed task is on the
    /// board when the sheet closes.
    func testNewTaskFilesATaskOnTheBoard() throws {
        let app = launch()
        openWorkspace(app, "Billing")
        choose(app, "Board")
        let add = app.buttons["new-task"]
        XCTAssertTrue(add.waitForExistence(timeout: 10), "no New Task…")
        add.tap()

        let title = app.textFields["new-task-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5), "no New Task sheet")
        title.tap()
        title.typeText(String(repeating: "a", count: 201))
        let tooLong = element(app, "new-task-too-long")
        XCTAssertTrue(tooLong.waitForExistence(timeout: 5), "a title past 200 says nothing")
        XCTAssertEqual(tooLong.label, "That title is too long. Shorten it to add the task.")
        XCTAssertFalse(app.buttons["new-task-add"].isEnabled, "Add Task takes a title too long")
        title.typeText(XCUIKeyboardKey.delete.rawValue)
        XCTAssertFalse(tooLong.exists, "200 scalars is still too long")
        XCTAssertTrue(app.buttons["new-task-add"].isEnabled)

        // The runner says no: its sentence, and the sheet stays.
        title.clearText()
        title.typeText("Refuse me")
        app.buttons["new-task-add"].tap()
        // The label's words, not its icon, which carries the same name.
        let failure = app.staticTexts["new-task-failure"].firstMatch
        XCTAssertTrue(failure.waitForExistence(timeout: 10), "the refusal said nothing")
        XCTAssertEqual(
            failure.label, "This device can only look at this runner, so it can’t add tasks.")

        title.clearText()
        title.typeText("Email the invoice")
        app.buttons["new-task-add"].tap()
        let card = element(app, "board-card-bil-20")
        XCTAssertTrue(card.waitForExistence(timeout: 10), "the filed task isn't on the board")
        XCTAssertFalse(title.exists, "the sheet is still up")
        XCTAssertEqual(
            element(app, "harness-sent").value as? String, "task.create billing Email the invoice")
    }

    /// **An empty status is a header reading its zero, and it doesn't open.**
    /// A status with tasks opens; Done starts collapsed.
    func testAnEmptyStatusIsACollapsedZeroHeader() throws {
        let app = launch()
        openWorkspace(app, "Billing")
        choose(app, "Board")
        let todo = element(app, "board-section-todo")
        XCTAssertTrue(todo.waitForExistence(timeout: 10), "an empty status is not on the board")
        XCTAssertEqual(todo.label, "To Do 0")
        XCTAssertEqual(todo.value as? String, "Empty")
        for status in ["backlog", "in_review", "cancelled"] {
            XCTAssertTrue(element(app, "board-section-\(status)").exists, "\(status) is missing")
        }

        // Done has a task, and starts collapsed.
        let done = element(app, "board-section-done")
        XCTAssertEqual(done.label, "Done 1")
        XCTAssertEqual(done.value as? String, "Collapsed")
        XCTAssertFalse(element(app, "board-card-bil-5").exists, "Done did not start collapsed")
        done.tap()
        XCTAssertTrue(element(app, "board-card-bil-5").waitForExistence(timeout: 5))
        XCTAssertEqual(element(app, "board-section-done").value as? String, "Expanded")

        // Tapping the empty one opens nothing.
        todo.tap()
        XCTAssertEqual(element(app, "board-section-todo").value as? String, "Empty")
    }
}

private extension XCUIElement {
    /// Delete everything in a text field, one key at a time: what a person
    /// holding Delete does. Tapped first, since a sheet that was sending
    /// took the focus away.
    func clearText() {
        tap()
        let typed = (value as? String) ?? ""
        typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: typed.count))
    }
}
