import XCTest

/// The shell's bar has to survive the keyboard, and still answer a finger
/// afterwards.
///
/// This was `testTabStripRemainsInteractiveAfterKeyboardDismissal`, written
/// against `TerminalTabStrip` — a floating strip of chips under a navigation
/// bar, in a `WorkspaceView`. Neither exists. What the regression was ABOUT
/// does: an input accessory lives in the keyboard's window, and a pane host
/// that lets the keyboard into its own safe area gets its furniture shoved up
/// the screen and then left there. The shell's answer is
/// `ShellRootView.body`'s full-height stack and every pane's
/// `.ignoresSafeArea(.keyboard, edges: .bottom)`, and this is the assertion
/// that they hold.
///
/// So the same five moments, against the surfaces that carry them now:
///
/// | Then | Now |
/// | --- | --- |
/// | the tab strip's frame | `shell-bar`'s frame |
/// | `worktree-tab-changes`'s `value == "current"` | `shell-state`'s `tab=` |
/// | a lift is a tap on a chip | a lift on the bar, which is how a tab is chosen |
///
/// **Two of the old assertions have no equivalent and are not replaced.** The
/// changes pane's "Review options" menu and the "Switch worktree" button were
/// items in `WorkspaceView`'s navigation-bar toolbar, and the test checked
/// their ORDER within it — that the switcher stayed rightmost as the contextual
/// menu came and went. There is no navigation bar over a pane any more and
/// neither control has a host, so there is no order left to be wrong. That is a
/// gap in the shell rather than in this test; see `ChangesToolbarMenu`, which
/// is still in the tree and is currently mounted by nothing.
final class KeyboardTabStripTests: XCTestCase {
    func testTheBarRemainsInteractiveAfterKeyboardDismissal() throws {
        #if targetEnvironment(simulator)
        throw XCTSkip("This regression needs a real iPhone with a configured worktree.")
        #endif

        let app = XCUIApplication()
        app.launchDrawn()

        let composer = app.textViews.firstMatch
        guard composer.waitForExistence(timeout: 10) else {
            throw XCTSkip("The current device state has no agent composer to exercise.")
        }

        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        guard bar.waitForExistence(timeout: 5) else {
            throw XCTSkip("The current screen is not the shell.")
        }
        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard probe.waitForExistence(timeout: 5) else {
            throw XCTSkip("The shell never reported where it was.")
        }
        let home = try XCTUnwrap(try app.shellState()["tab"])
        let initialBarFrame = bar.frame

        let transcript = app.scrollViews["agent-transcript"]
        guard transcript.waitForExistence(timeout: 5) else {
            throw XCTSkip("The current agent pane has no transcript scroll view.")
        }
        // Establish the exact precondition under test. The live transcript can
        // grow while this suite is being rebuilt, so do not depend on how far
        // a previous app session happened to leave it scrolled.
        for _ in 0..<10 {
            if (transcript.value as? String)?.hasSuffix("tail=true") == true { break }
            transcript.swipeUp(velocity: .fast)
        }
        XCTAssertTrue(
            (transcript.value as? String)?.hasSuffix("tail=true") == true,
            "Could not establish a transcript parked at its tail"
        )

        composer.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let remainsAtTail = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value ENDSWITH %@", "tail=true"),
            object: transcript
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [remainsAtTail], timeout: 2), .completed,
            "Opening the keyboard pulled a tail-following transcript off the bottom"
        )
        XCTAssertEqual(
            bar.frame.minY, initialBarFrame.minY, accuracy: 2,
            "The bar moved when the keyboard appeared"
        )

        app.swipeDown()
        let keyboardDismissed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == NO"),
            object: app.keyboards.firstMatch
        )
        XCTAssertEqual(XCTWaiter.wait(for: [keyboardDismissed], timeout: 5), .completed)

        XCTAssertEqual(
            bar.frame.minY, initialBarFrame.minY, accuracy: 2,
            "The bar moved after the keyboard disappeared"
        )
        let transcriptInsetReset = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value BEGINSWITH %@", "keyboard=0;"),
            object: transcript
        )
        XCTAssertEqual(
            XCTWaiter.wait(for: [transcriptInsetReset], timeout: 2), .completed,
            "The transcript retained the keyboard-height bottom inset after dismissal: \(String(describing: transcript.value))"
        )
        XCTAssertTrue(bar.isHittable, "The bar ended up somewhere nothing can touch")

        // And it still ANSWERS, which is the half the frame check cannot make.
        // A lift onto the first row is how a tab is chosen in the shell — the
        // column unfurls one row per 34 points, and the first row is the diff,
        // which every worktree has. That is the same move the Changes chip
        // used to be.
        try XCTSkipUnless(home != 0, "This pane is already the first tab; nothing to move to.")
        lift(bar, by: 20)
        let landed = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in (try? app.shellState())?["tab"] == 0 }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [landed], timeout: 5), .completed,
            "A lift on the bar after the keyboard had come and gone chose nothing: "
                + "\(probe.value ?? "")"
        )

        let toolbarPost = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        toolbarPost.name = "shell-bar-after-keyboard"
        toolbarPost.lifetime = .keepAlways
        add(toolbarPost)
    }

    /// Lift the bar by `points`, and hold there so the column is at that height
    /// when the finger leaves. The same helper `ShellGestureTests` uses, and it
    /// has to stay the same: a lift that releases in the frame of its last
    /// movement can leave the final translation unreported.
    private func lift(_ bar: XCUIElement, by points: CGFloat) {
        let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let to = from.withOffset(CGVector(dx: 0, dy: -points))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.5)
    }
}
