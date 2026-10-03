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
        app.launchDrawn()

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

    /// **The pane at rest is claimed as read** (ov-27, kept when the overview
    /// it was written against went).
    ///
    /// `Notifier.visibleTerminal` is what suppresses a banner about the pane
    /// you are looking at and what the poll's `markVisibleSeen` claims on the
    /// runner and marks `terminal.seen` from. The agent layout harness mounts
    /// the shell over one worktree, landing on the agent pane, so the claim
    /// has to arrive without a finger touching anything: it is the shell's own
    /// `markVisible` and the pane's own mount task saying the same thing.
    func testTheAgentPaneAtRestIsClaimedAsRead() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain"]
        app.launchDrawn()

        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        XCTAssertTrue(probe.waitForExistence(timeout: 30), "the shell never stood up")
        let watch = app.descendants(matching: .any).matching(identifier: "shell-watch").firstMatch
        XCTAssertTrue(watch.waitForExistence(timeout: 10), "the screen has no watch probe")

        let claimed = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                (watch.value as? String ?? "") == "watch=harness"
            }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [claimed], timeout: 15), .completed,
            "the pane at rest is not claimed as read: \(watch.value ?? "") "
                + "(\(probe.value ?? ""))")
    }
}
