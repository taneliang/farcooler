import XCTest

/// The agent composer's Hide Keyboard key.
///
/// The ruling on the bar behind the keyboard keeps the bar where it is, so
/// putting the keyboard away is the way back to it, from an agent pane as much
/// as from a terminal. The terminal's key row had the key; the composer did
/// not, and a person typing to an agent had only the system's own dismissal,
/// which nothing on screen points to.
///
/// Stands on `-agent-layout-harness`: the real `AgentView` over a canned
/// transcript, so this needs no runner and cannot skip itself green.
final class ComposerKeyboardTests: XCTestCase {
    func testTheComposersHideKeyboardKeyPutsTheKeyboardAway() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain"]
        app.launch()

        let field = app.textViews.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 30), "the agent pane drew no composer")
        // Not there before anybody types: with the keyboard down there is
        // nothing for it to do.
        XCTAssertFalse(
            app.buttons[Self.composerHideKeyboard].exists,
            "Hide Keyboard is offered with no keyboard up")

        hideKeyboard(app, Self.composerHideKeyboard, raising: field)

        // Still the composer, still docked: only the keyboard went.
        XCTAssertTrue(field.exists, "hiding the keyboard took the composer with it")
    }

    /// **The composer is not docked over the grid** (ov-27 m1).
    ///
    /// A composer is an input accessory, so it lives in the KEYBOARD's window
    /// and hiding its pane does nothing to it: an agent pane at rest under the
    /// open grid — B's first pane after a runner switch, say — went on
    /// drawing its composer over the cards. Terminals were held back while
    /// the grid is up; the chat composer was not.
    ///
    /// Opened ON the grid (`-shell-overview`) rather than lifted to it: with
    /// the composer docked, the bar takes no touches in this harness.
    func testTheComposerIsNotDockedOverTheGrid() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain", "-shell-overview"]
        app.launch()

        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        XCTAssertTrue(probe.waitForExistence(timeout: 30), "the shell never stood up")
        let state = probe.value as? String ?? ""
        XCTAssertTrue(state.contains("overview=1"), "the harness did not open on the grid: \(state)")
        XCTAssertTrue(state.contains("tab=1"), "the agent pane is not the one at rest: \(state)")

        // Given the time a docked bar takes to appear, then asserted absent.
        let send = app.buttons["agent-send"]
        func docked() -> XCTNSPredicateExpectation {
            XCTNSPredicateExpectation(
                predicate: NSPredicate { _, _ in send.exists && send.isHittable }, object: nil)
        }
        XCTAssertEqual(
            XCTWaiter.wait(for: [docked()], timeout: 5), .timedOut,
            "the composer is docked over the grid")

        // And it docks when the grid closes onto the pane.
        app.buttons["shell-overview-done"].tap()
        XCTAssertEqual(
            XCTWaiter.wait(for: [docked()], timeout: 10), .completed,
            "closing the grid onto the agent pane left it with no composer")
    }

    /// **A pane that mounts under the grid is not claimed as read** (ov-27,
    /// the case reported). B's first pane after a runner switch mounts at
    /// rest while the grid is up, and a pane's own mount task claimed it —
    /// `Notifier.visibleTerminal`, which the poll's `markVisibleSeen` claims
    /// on the runner and marks `terminal.seen` from — whatever was over it.
    ///
    /// The agent layout harness, opened on the grid with `-late-pane`, is
    /// that mount: a seat, an empty fleet, then the real `TerminalView` and
    /// `AgentView` arriving at rest under the open grid. (Opened on the grid
    /// with the pane already there, the screen's own "nothing is read" lands
    /// after the pane's claim and hides it; that was tried, and it passed
    /// without the fix.) The claim arrives when the grid closes onto the pane.
    func testAPaneThatMountsUnderTheGridIsNotRead() throws {
        let app = XCUIApplication()
        app.launchArguments = [
            "-agent-layout-harness", "-plain", "-shell-overview", "-late-pane",
        ]
        app.launch()

        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        XCTAssertTrue(probe.waitForExistence(timeout: 30), "the shell never stood up")
        XCTAssertTrue(
            (probe.value as? String ?? "").contains("overview=1"),
            "the harness did not open on the grid: \(probe.value ?? "")")
        let watch = app.descendants(matching: .any).matching(identifier: "shell-watch").firstMatch
        XCTAssertTrue(watch.waitForExistence(timeout: 10), "the screen has no watch probe")

        // The pane has arrived: the shell rests on the agent tab (tab 1, after
        // the Diff tab), under the grid.
        let arrived = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                let value = probe.value as? String ?? ""
                return value.contains("tab=1") && value.contains("worktrees=1")
                    && value.contains("overview=1")
            }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [arrived], timeout: 15), .completed,
            "the agent pane never came to rest under the grid: \(probe.value ?? "")")

        // Given the time a mount and a poll take, then asserted unclaimed.
        func claimed() -> XCTNSPredicateExpectation {
            XCTNSPredicateExpectation(
                predicate: NSPredicate { _, _ in
                    let value = watch.value as? String ?? ""
                    return value.hasPrefix("watch=") && value != "watch="
                }, object: nil)
        }
        XCTAssertEqual(
            XCTWaiter.wait(for: [claimed()], timeout: 5), .timedOut,
            "the pane under the grid is claimed as read: \(watch.value ?? "")")

        app.buttons["shell-overview-done"].tap()
        XCTAssertEqual(
            XCTWaiter.wait(for: [claimed()], timeout: 10), .completed,
            "closing the grid onto the pane did not claim it: \(watch.value ?? "")")
    }
}
