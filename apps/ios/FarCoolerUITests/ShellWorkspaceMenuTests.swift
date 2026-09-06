import XCTest

/// The overview card's long press, and the two things it offers.
///
/// **Why a UI test and not arithmetic.** `ShellNavigationTests` is where a
/// rule about the fleet belongs, and there is no rule here: hiding is a flag
/// the card reads and removal is a call the screen makes. What can be wrong is
/// entirely about the gesture and the menu — a `contextMenu` attached to a view
/// the button style has already consumed the press of, an item that reads
/// `Hide` on a workspace that is already hidden, and above all `Remove
/// Worktree…` appearing on the repository's own checkout. None of those is
/// reachable without pressing a card.
///
/// No runner and no daemon. `-shell-harness` stands the shell on a canned
/// fleet — workspace 0 is the primary checkout in every fixture, and
/// `-shell-hidden` puts every fifth one from index 3 away — so this suite
/// never skips, the way `ShellGestureTests` never does and for the same
/// reason.
///
/// What is deliberately NOT here is the removal ceremony itself. The typed
/// name and the confirmation before it are `RemoveWorktreeFlow`, they are
/// driven by what `workspace.remove_worktree` answers, and a fixture has no
/// runner to answer. Asserting them against a harness would be asserting that
/// this file's own stub says yes.
final class ShellWorkspaceMenuTests: XCTestCase {
    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-shell-harness", "-shell-overview", "-shell-4"] + extra
        app.launch()
        return app
    }

    /// Long-press a card and wait for its menu to be up.
    ///
    /// A press of `1.2` seconds rather than the platform's minimum. A context
    /// menu recognizes at around half a second, and the card is inside a
    /// `ButtonStyle` that highlights on touch-down and a grid that has a
    /// simultaneous drag gesture over it — a press near the threshold is a
    /// press that intermittently resolves as a tap, which OPENS the workspace
    /// and leaves the assertion below failing for a reason that has nothing to
    /// do with what it is about.
    private func openMenu(on card: XCUIElement, in app: XCUIApplication) {
        XCTAssertTrue(card.waitForExistence(timeout: 15), "the grid never drew a card")
        card.press(forDuration: 1.2)
        // The menu is the app's own, so its items are buttons in the same
        // element tree. Waiting on the one item BOTH menus carry, rather than
        // on a container: a context menu is not an `alert` and is not reliably
        // an element of its own.
        let anyItem = app.buttons.matching(
            NSPredicate(format: "label == %@ OR label == %@", "Hide", "Unhide")
        ).firstMatch
        XCTAssertTrue(
            anyItem.waitForExistence(timeout: 5),
            "long-pressing a workspace card put no menu up: \(app.debugDescription)")
    }

    /// **The two things a card can do besides open.**
    ///
    /// Both, in one test, because the menu is one object: a version that
    /// offered only `Hide` and a version that offered only `Remove Worktree…`
    /// are both wrong, and separate tests would let one of them pass alone.
    func testLongPressingAWorkspaceCardOffersHideAndRemoveWorktree() throws {
        let app = launch()
        // `ws-1` and not `ws-0`: workspace 0 is the fixture's primary checkout,
        // which is the case the next test is about.
        openMenu(on: app.buttons["shell-card-ws-1"], in: app)

        XCTAssertTrue(
            app.buttons["Hide"].exists,
            "a workspace that is showing was not offered Hide")
        XCTAssertTrue(
            app.buttons["Remove Worktree…"].exists,
            "an ordinary worktree was not offered removal: \(app.debugDescription)")
    }

    /// **`Remove Worktree…` is absent, not disabled, for the repository's own
    /// checkout.**
    ///
    /// The Mac's rule, kept: *"A daemon-side refusal is a safety net; the
    /// button should not be there to press."* Removing the primary checkout
    /// would offer to delete the directory the repository itself lives in, and
    /// `Workspace.isPrimaryCheckout` records that the phone once offered it on
    /// every workspace in the fleet — the flag was decoded under the Mac's
    /// spelling and was nil for all of them.
    ///
    /// `Hide` is asserted present in the same breath, so a menu that failed to
    /// build at all cannot pass this as a menu that correctly left one item
    /// out.
    func testThePrimaryCheckoutIsNotOfferedRemoval() throws {
        let app = launch()
        openMenu(on: app.buttons["shell-card-ws-0"], in: app)

        XCTAssertTrue(
            app.buttons["Hide"].exists,
            "the primary checkout's menu did not build at all")
        XCTAssertFalse(
            app.buttons["Remove Worktree…"].exists,
            "the repository's own checkout was offered removal")
    }

    /// **A worktree that is already away is offered the way back.**
    ///
    /// One control in two states rather than two controls, which is the Mac's
    /// arrangement in `SidebarViews`. Hiding is reversible and this menu is
    /// now the phone's only way to reverse it: the grid's Hidden section
    /// reveals a card, and before this there was nothing to do to the card it
    /// revealed.
    func testAHiddenWorkspaceIsOfferedUnhideInsteadOfHide() throws {
        let app = launch(["-shell-hidden"])

        // The fixture hides every fifth workspace from index 3, and the
        // section it puts them in is collapsed until it is asked for.
        let section = app.descendants(matching: .any)
            .matching(identifier: "shell-hidden-section").firstMatch
        XCTAssertTrue(section.waitForExistence(timeout: 15), "no hidden section to open")
        section.tap()

        openMenu(on: app.buttons["shell-card-ws-3"], in: app)

        XCTAssertTrue(
            app.buttons["Unhide"].exists,
            "a hidden worktree was not offered the way back: \(app.debugDescription)")
        XCTAssertFalse(
            app.buttons["Hide"].exists,
            "a hidden worktree was offered Hide, which would do nothing")
    }
}
