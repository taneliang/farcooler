import XCTest

/// The phone's workspace screen (ov-616): no segment control. A workspace
/// with an orchestrator is its conversation, under the Now/Next card that
/// opens the Plan sheet; one without is the Plan itself, with Start
/// Orchestrator at its top.
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
        XCTAssertTrue(app.navigationBars[name].waitForExistence(timeout: 10), "the workspace screen did not open")
    }

    /// Down Billing's Plan to bil-9's own worktree: No Theme › bil-9.
    private func openWebhooksLevel(_ app: XCUIApplication) {
        for id in ["tree-row-No Theme", "tree-row-bil-9"] {
            let row = app.buttons[id]
            XCTAssertTrue(row.waitForExistence(timeout: 10), "no \(id): \(app.debugDescription)")
            row.tap()
        }
    }

    /// The Plan sheet, opened from the Now/Next card.
    private func openPlan(_ app: XCUIApplication) {
        let card = element(app, "plan-strip")
        XCTAssertTrue(card.waitForExistence(timeout: 10), "no Now/Next card: \(app.debugDescription)")
        card.tap()
        XCTAssertTrue(element(app, "plan-sheet").waitForExistence(timeout: 10), "the Plan sheet did not open")
    }

    /// **No segment control**: Main is its conversation, with the Now/Next
    /// card over it, and Billing, which has no orchestrator, is its Plan.
    func testNoSegmentsAWorkspaceIsItsConversationOrItsPlan() throws {
        let app = launch()
        openWorkspace(app, "Main")
        XCTAssertTrue(element(app, "orchestrator-pane").waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "plan-strip").exists, "no Now/Next card over the conversation")
        XCTAssertFalse(element(app, "workspace-segments").exists, "the segment control is back")
        for segment in ["orchestrator", "tree", "board", "worktrees"] {
            XCTAssertFalse(app.buttons["segment-\(segment)"].exists, "a \(segment) segment")
        }
        XCTAssertTrue(app.navigationBars["Main"].staticTexts["Orchestrator"].exists, "the seat isn't named under the title")

        // Back by its name, not as the bar's first button: on an iPhone Duo
        // (iOS 27.1) Back and the bar's items can stand in a rail at the
        // screen's edge, outside the navigation bar (ov-709).
        app.tapBack()
        openWorkspace(app, "Billing")
        XCTAssertTrue(element(app, "plan-filters").waitForExistence(timeout: 10), "Billing isn't its Plan: \(app.debugDescription)")
        XCTAssertTrue(app.buttons["start-orchestrator"].exists, "no Start Orchestrator over the Plan")
        XCTAssertFalse(element(app, "plan-strip").exists, "a Now/Next card with no conversation under it")
        XCTAssertFalse(element(app, "orchestrator-pane").exists)
        XCTAssertTrue(app.buttons["tree-row-No Theme"].waitForExistence(timeout: 5), "no tree")
        // scratch is unclaimed and fc-3-webhooks hangs under bil-9, so
        // Billing has nothing loose: a Loose Worktrees group here would be
        // listing another's worktree (review 6). The repository's checkout is
        // the Main Checkout group every workspace shows, never one of
        // Billing's worktrees.
        XCTAssertFalse(app.buttons["tree-row-Loose Worktrees"].exists, "a worktree that isn't Billing's is loose in its tree")
        XCTAssertFalse(app.buttons["worktree-row-scratch"].exists)
        XCTAssertTrue(app.reveal(app.buttons["tree-row-Main Checkout"]), "no Main Checkout")
        XCTAssertFalse(app.buttons["worktree-row-overnight"].exists, "the checkout is listed as one of Billing's worktrees")
        XCTAssertTrue(app.reveal(app.buttons["new-worktree"]), "no New Worktree…")
        let top = app.buttons["tree-row-No Theme"]
        for _ in 0..<8 where !(top.exists && top.isHittable) { app.swipeDown() }

        // Its levels are pushed on the stack, and Billing's worktree hangs
        // under the card it's for.
        openWebhooksLevel(app)
        XCTAssertTrue(app.buttons["worktree-row-fc-3-webhooks"].waitForExistence(timeout: 5))
        XCTAssertFalse(element(app, "plan-sheet").exists, "a workspace with no conversation opened a sheet")
    }

    /// **The Plan sheet keeps its own stack** (ov-615, ov-616): a level
    /// pushed in it stays in it, Back comes back to the Plan, and Done
    /// closes it onto the conversation, which was never built again.
    func testThePlanSheetDrillsInBacksOutAndCloses() throws {
        let app = launch(["-phone-billing-led"])
        openWorkspace(app, "Billing")
        let mount = element(app, "orchestrator-mount")
        XCTAssertTrue(mount.waitForExistence(timeout: 10), "no orchestrator pane")
        let before = try XCTUnwrap(mount.value as? String)

        openPlan(app)
        XCTAssertTrue(app.navigationBars["Plan"].exists)
        let noTheme = app.buttons["tree-row-No Theme"]
        // A lazy list draws only what is near its window: on a short screen (the
        // iPhone Duo's 678 pt) No Theme is below the first screen (ov-713).
        XCTAssertTrue(app.reveal(noTheme) || noTheme.waitForExistence(timeout: 10), "no tree in the sheet: \(app.debugDescription)")
        noTheme.tap()
        XCTAssertTrue(app.navigationBars["No Theme"].waitForExistence(timeout: 10), "the level wasn't pushed")
        XCTAssertTrue(element(app, "plan-sheet").exists, "the level left the sheet")
        app.buttons["tree-row-bil-9"].tap()
        XCTAssertTrue(app.buttons["worktree-row-fc-3-webhooks"].waitForExistence(timeout: 10), "bil-9's level wasn't pushed")
        let level = app.navigationBars.matching(NSPredicate(format: "identifier BEGINSWITH 'bil-9'")).firstMatch
        XCTAssertTrue(level.exists, "bil-9's level has no bar: \(app.debugDescription)")

        // Back, twice, is the Plan; Done is the conversation.
        // Back by name: on the Duo it stands in a rail, outside the bar (ov-710).
        app.tapBack()
        XCTAssertTrue(app.navigationBars["No Theme"].waitForExistence(timeout: 10))
        app.tapBack()
        XCTAssertTrue(app.navigationBars["Plan"].waitForExistence(timeout: 10), "Back didn't return to the Plan")
        app.buttons["plan-sheet-done"].tap()
        XCTAssertTrue(element(app, "plan-sheet").waitForNonExistence(timeout: 10), "Done didn't close the sheet")
        XCTAssertTrue(app.navigationBars["Billing"].exists)
        XCTAssertEqual(mount.value as? String, before, "the pane was built again")
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

    /// **A workspace with no orchestrator can start one from the phone**
    /// (ruling 8): from the top of its Plan, the harness is chosen from a
    /// menu, the screen becomes the conversation and says it's starting, and
    /// the pane takes its place once the runner has it.
    func testStartOrchestratorFromAnEmptyWorkspace() throws {
        let app = launch()
        openWorkspace(app, "Billing")
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
        XCTAssertTrue(element(app, "plan-strip").exists, "no Now/Next card over the new conversation")
        // A start, not a replace: the menu's harness, and replace false.
        XCTAssertEqual(
            element(app, "harness-sent").value as? String, "start billing claude replace=false")
    }

    /// **Covered, the orchestrator stops; back on it, it's read again**
    /// (ov-66 fix). A decision push over Billing's orchestrator opens the
    /// task in the Plan sheet (ov-616): the pane stays mounted and stops
    /// streaming. Back in the sheet is the Plan, and Done is Billing's
    /// orchestrator again, streaming and held as the pane being read.
    func testTheOrchestratorPausesUnderATaskAndResumesOnBack() throws {
        let app = launch(["-phone-billing-led"])
        openWorkspace(app, "Billing")
        let probe = phoneProbe(app)
        XCTAssertTrue(wait(probe, "live=\(Self.billingOrchestrator) watch=\(Self.billingOrchestrator)"))

        Self.notify("decision")
        XCTAssertTrue(
            element(app, "task-heading").waitForExistence(timeout: 10), "the push opened nothing")
        XCTAssertTrue(element(app, "plan-sheet").exists, "the task isn't in the Plan sheet")
        XCTAssertTrue(wait(probe, "live= watch="), "still streaming under the task: \(probe.value ?? "")")

        app.tapBack()
        XCTAssertTrue(app.navigationBars["Plan"].waitForExistence(timeout: 10), "Back didn't come back to the Plan")
        app.buttons["plan-sheet-done"].tap()
        XCTAssertTrue(
            wait(probe, "live=\(Self.billingOrchestrator) watch=\(Self.billingOrchestrator)"),
            "not read again on Back: \(probe.value ?? "")")
    }

    /// **Under a worktree, the same**: an agent's notification covers the
    /// workspace with its worktree, the orchestrator stops, and one Back
    /// brings it back live and read (ov-617: the list is two Backs away).
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
        XCTAssertTrue(app.navigationBars["Billing"].waitForExistence(timeout: 10), "Back isn't the workspace")
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


    /// **No shell bar, and no swipe-up** (ov-617): a worktree is its pane
    /// under the pane's own bar, its other tabs are in that bar's More menu,
    /// and Back is the workspace, one step from the list.
    func testAWorktreeHasNoShellBarAndItsTabsAreInItsMenu() throws {
        let app = launch(["-phone-billing-led"])
        openWorkspace(app, "Billing")
        Self.notify("agent")
        let back = app.buttons["worktree-back"].firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 20), "the agent's worktree did not open")
        XCTAssertFalse(element(app, "shell-bar").exists, "the shell's bar is back")
        let before = try app.shellState()
        XCTAssertGreaterThan(before["tabs"] ?? 0, 1, "the worktree has one tab: \(before)")

        // Another tab, from the More menu.
        let other = ((before["tab"] ?? 0) + 1) % (before["tabs"] ?? 1)
        app.chooseTab(other)
        let moved = NSPredicate { _, _ in (try? app.shellState())?["tab"] == other }
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: moved, object: nil)], timeout: 10), .completed,
            "the menu didn't move to tab \(other)")

        // A swipe up from the bottom edge does nothing of the shell's.
        let window = app.windows.firstMatch
        window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.97))
            .press(forDuration: 0.05, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)))
        XCTAssertEqual(try app.shellState()["tab"], other, "a swipe up moved the worktree")

        // Back twice is the list.
        app.buttons.matching(identifier: "worktree-back").allElementsBoundByIndex.first { $0.isHittable }?.tap()
        XCTAssertTrue(app.navigationBars["Billing"].waitForExistence(timeout: 10), "Back isn't the workspace")
        app.tapBack()
        XCTAssertTrue(app.navigationBars["Needs You"].waitForExistence(timeout: 10), "the list isn't two Backs away")
    }
}
