import XCTest

/// The phone's workspace screen (ov-55 4A.2): Orchestrator, Themes (the One
/// tree, which took Worktrees' place, ov-300) and Board, one at a time, under
/// the workspace's name.
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
        let id = segment == "Themes" ? "segment-tree" : "segment-\(segment.lowercased())"
        let button = app.buttons[id]
        XCTAssertTrue(button.waitForExistence(timeout: 5), "no \(segment) segment")
        button.tap()
    }

    /// Down the tree to bil-9's own worktree: No Theme › bil-9.
    private func openWebhooksLevel(_ app: XCUIApplication) {
        choose(app, "Themes")
        for id in ["tree-row-No Theme", "tree-row-bil-9"] {
            let row = app.buttons[id]
            XCTAssertTrue(row.waitForExistence(timeout: 10), "no \(id): \(app.debugDescription)")
            row.tap()
        }
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

        choose(app, "Themes")
        XCTAssertTrue(app.buttons["tree-row-No Theme"].waitForExistence(timeout: 5), "no tree")
        // Unclaimed: not Billing's, so not loose in its tree either.
        XCTAssertFalse(app.buttons["worktree-row-scratch"].exists)
        XCTAssertTrue(app.buttons["new-worktree"].exists, "no New Worktree…")

        // The choice is kept per workspace: Billing comes back on Themes.
        app.navigationBars.buttons.firstMatch.tap()
        openWorkspace(app, "Billing")
        XCTAssertTrue(app.buttons["tree-row-No Theme"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["segment-tree"].isSelected)

        // Billing's worktree hangs under the card it's for.
        for id in ["tree-row-No Theme", "tree-row-bil-9"] { app.buttons[id].tap() }
        XCTAssertTrue(app.buttons["worktree-row-fc-3-webhooks"].waitForExistence(timeout: 5))
    }

    /// **A worktree put away can be taken back out, and put away again**,
    /// from its row's swipe, as the Mac's sidebar does with Hide and Unhide.
    /// The runner keeps the preference; the row moves out of the workspace's
    /// Hidden section on the runner's answer. fc-3-webhooks starts hidden.
    func testAHiddenWorktreeCanBeUnhiddenAndHiddenAgain() throws {
        let app = launch(["-phone-webhooks-hidden"])
        openWorkspace(app, "Billing")
        openWebhooksLevel(app)
        let row = app.buttons["worktree-row-fc-3-webhooks"]
        XCTAssertTrue(row.waitForExistence(timeout: 10), "the hidden worktree isn't listed")
        XCTAssertTrue(row.label.contains("Hidden"), "it doesn't say it's hidden: \(row.label)")

        func swipe() {
            let from = row.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5))
            from.press(
                forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -120, dy: 0)),
                withVelocity: .slow, thenHoldForDuration: 0.3)
        }
        swipe()
        let unhide = app.buttons["unhide-fc-3-webhooks"]
        XCTAssertTrue(unhide.waitForExistence(timeout: 5), "a hidden worktree offers no Unhide")
        XCTAssertFalse(app.buttons["hide-fc-3-webhooks"].exists, "and offers Hide as well")
        unhide.tap()

        let sent = element(app, "harness-sent")
        let unhid = NSPredicate { _, _ in
            (sent.value as? String ?? "").contains("worktree.unhide fc-3-webhooks")
        }
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: unhid, object: nil)], timeout: 10),
            .completed, "Unhide sent nothing: \(sent.value ?? "")")
        let shown = NSPredicate(format: "NOT (label CONTAINS 'Hidden')")
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: shown, object: row)], timeout: 5),
            .completed, "it still says it's hidden")

        swipe()
        let hide = app.buttons["hide-fc-3-webhooks"]
        XCTAssertTrue(hide.waitForExistence(timeout: 5), "a shown worktree offers no Hide")
        hide.tap()
        let hid = NSPredicate { _, _ in
            (sent.value as? String ?? "").contains("worktree.hide fc-3-webhooks")
        }
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: hid, object: nil)], timeout: 10),
            .completed, "Hide sent nothing: \(sent.value ?? "")")
        let hidden = NSPredicate(format: "label CONTAINS 'Hidden'")
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: hidden, object: row)], timeout: 5),
            .completed, "it did not go back")
    }

    /// **The board says how many decisions are waiting, from the runner's
    /// list** (ov-69): the Mac's sentence, over the workspace's decision
    /// items. Billing has one, bil-7's, beside an ask that isn't one.
    func testBoardSaysHowManyDecisionsAreWaiting() throws {
        let app = XCUIApplication.phoneHarness([])
        openWorkspace(app, "Billing")
        choose(app, "Board")
        let line = element(app, "board-waiting")
        XCTAssertTrue(line.waitForExistence(timeout: 10), "no waiting line: \(app.debugDescription)")
        XCTAssertEqual(line.label, "1 task is waiting on you")
    }

    /// **An answered decision leaves the card in Needs Decision and the line
    /// gone** (spec 2.2): with nothing needing the person, the column still
    /// holds bil-7 and the board says nothing is waiting.
    func testAnAnsweredDecisionLeavesNoWaitingLine() throws {
        let app = launch()
        openWorkspace(app, "Billing")
        choose(app, "Board")
        XCTAssertTrue(
            element(app, "board-card-bil-7").waitForExistence(timeout: 10),
            "bil-7 is not in Needs Decision")
        XCTAssertFalse(element(app, "board-waiting").exists, "a waiting line for an empty list")
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
        let claude = app.buttons["Claude Code"]
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
        choose(app, "Themes")
        choose(app, "Orchestrator")
        XCTAssertTrue(mount.waitForExistence(timeout: 10), "the pane did not come back")
        XCTAssertEqual(mount.value as? String, before, "the pane was built again")
    }

    /// **The board files nothing** (ov-184): the orchestrator owns the task
    /// list, so a Control-scope phone on a workspace's board gets no New
    /// Task…, in the toolbar or anywhere else.
    func testTheBoardOffersNoWayToFileATask() throws {
        let app = launch()
        openWorkspace(app, "Billing")
        choose(app, "Board")
        XCTAssertTrue(element(app, "board").waitForExistence(timeout: 10), "the board did not show")
        XCTAssertTrue(element(app, "board-card-bil-9").waitForExistence(timeout: 10), "no cards")
        XCTAssertFalse(app.buttons["new-task"].exists, "New Task… is in the toolbar")
        XCTAssertFalse(app.buttons["board-empty-new-task"].exists, "New Task… is on the board")
        XCTAssertEqual(
            app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'New Task'")).count, 0,
            "a New Task button is on the board")
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

    /// **Empty statuses are said once, in a line at the end, and aren't
    /// headers.** A status with tasks opens; Done starts collapsed.
    func testEmptyStatusesAreOneLineAndDoneStartsCollapsed() throws {
        let app = launch()
        openWorkspace(app, "Billing")
        choose(app, "Board")
        let note = element(app, "board-empty-statuses")
        XCTAssertTrue(note.waitForExistence(timeout: 10), "the empty statuses are not named")
        for name in ["Backlog", "To Do", "In Review", "Canceled"] {
            XCTAssertTrue(note.label.contains(name), "\(name) is missing from \(note.label)")
        }
        for status in ["backlog", "todo", "in_review", "cancelled"] {
            XCTAssertFalse(
                element(app, "board-section-\(status)").exists, "\(status) still has a header")
        }

        // Done has a task, and starts collapsed.
        let done = element(app, "board-section-done")
        XCTAssertEqual(done.label, "Done 1")
        XCTAssertEqual(done.value as? String, "Collapsed")
        XCTAssertFalse(element(app, "board-card-bil-5").exists, "Done did not start collapsed")
        done.tap()
        XCTAssertTrue(element(app, "board-card-bil-5").waitForExistence(timeout: 5))
        XCTAssertEqual(element(app, "board-section-done").value as? String, "Expanded")
    }
}
