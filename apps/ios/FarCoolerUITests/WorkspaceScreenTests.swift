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
