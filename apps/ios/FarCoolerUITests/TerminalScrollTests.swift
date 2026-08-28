import XCTest

/// A swipe on a terminal moves it into the pane's scrollback.
///
/// This is the regression that could not be caught anywhere else. The gesture
/// works and always did — a `UIPanGestureRecognizer` on the keyboard sink,
/// converting the drag to whole lines — but a pane on the capture-polling
/// fallback holds a `VTCore` that is exactly as tall as the screen, so
/// `scroll` had nothing to move through and the swipe did nothing at all,
/// silently, forever. Nothing in the Rust tests can see that: the daemon was
/// answering every question it was asked correctly, and the phone was not
/// asking for the scrollback.
///
/// So the assertion is on the emulator's own numbers, published through
/// `terminal-surface`'s accessibility value. `history` says whether there is
/// anywhere to go, and `offset` says whether the swipe went there — and the
/// two failures they tell apart are exactly the two that look identical on a
/// screen, which is a terminal that will not scroll.
///
/// Needs a runner. `./scripts/demo-host.sh` stands one up on 127.0.0.1:2222
/// with the FENCED `authorized_keys` line — the one every enrolled device
/// gets, and the configuration this bug lives in. Skipped, not failed, when
/// nothing answers: a test suite that goes red on a laptop with no demo host
/// running teaches people to ignore it.
final class TerminalScrollTests: XCTestCase {
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        // The Mac's user and host, forwarded by xcodebuild as TEST_RUNNER_*.
        //
        // NOT `NSUserName()`: this code runs in the SIMULATOR, whose user is
        // not the account the demo sshd authenticates. That mistake produced
        // "@127.0.0.1" with an empty user and an app sitting on "Not
        // Authorized Yet" while the test waited for a terminal that was never
        // going to appear.
        let user = ProcessInfo.processInfo.environment["DEMO_USER"] ?? ""
        let host = ProcessInfo.processInfo.environment["DEMO_HOST"] ?? "127.0.0.1:2222"
        app.launchArguments += ["-farcoolerDemoHost", "\(user)@\(host)"]
        app.launch()
        return app
    }

    /// `offset=N history=M` off the surface, or nil while it has not appeared.
    private func position(_ app: XCUIApplication) -> (offset: Int, history: Int)? {
        let surface = app.otherElements["terminal-surface"]
        guard surface.exists, let value = surface.value as? String else { return nil }
        let parts = value.split(separator: " ")
        guard
            parts.count == 2,
            let offset = Int(parts[0].replacingOccurrences(of: "offset=", with: "")),
            let history = Int(parts[1].replacingOccurrences(of: "history=", with: ""))
        else { return nil }
        return (offset, history)
    }

    /// Walk from wherever the app opens to a terminal pane.
    ///
    /// Defensive about the route rather than pinned to it: the phone opens on
    /// "Needs You" when nothing is running and straight into a workspace when
    /// something is, and neither is this test's subject.
    /// Walk from wherever the app opens to a terminal pane.
    ///
    /// Straight at `fleet-terminal-<id>` (`FleetView.swift:2140`) rather than
    /// walking cells: the fleet list interleaves repository headers, workspaces
    /// and terminals, so "the first cell" is a repository and tapping it goes
    /// nowhere useful. The identifier is the only stable handle.
    private func openATerminal(_ app: XCUIApplication) throws {
        let workspaces = app.buttons.matching(
            NSPredicate(format: "label BEGINSWITH %@", "Workspaces")).firstMatch
        if workspaces.waitForExistence(timeout: 30) {
            workspaces.tap()
        }
        let pane = app.descendants(matching: .any).matching(
            NSPredicate(format: "identifier BEGINSWITH %@", "fleet-terminal-")).firstMatch
        guard pane.waitForExistence(timeout: 20) else {
            print(app.debugDescription)
            throw XCTSkip("No terminal in this fleet; run ./scripts/demo-host.sh first.")
        }
        pane.tap()
        guard app.otherElements["terminal-surface"].waitForExistence(timeout: 40) else {
            print(app.debugDescription)
            throw XCTSkip("The terminal pane never rendered.")
        }
    }

    func testASwipeScrollsIntoTheScrollback() throws {
        let app = launch()
        try openATerminal(app)

        // The pane must HAVE history, or this test proves nothing — a swipe
        // that does not move on a pane with nothing above it is correct.
        let hasHistory = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "NOT (value ENDSWITH %@)", "history=0"),
            object: app.otherElements["terminal-surface"]
        )
        guard XCTWaiter.wait(for: [hasHistory], timeout: 30) == .completed else {
            let seen = position(app).map { "offset=\($0.offset) history=\($0.history)" } ?? "nothing"
            XCTFail(
                """
                The pane reported no scrollback (\(seen)), so a swipe has nowhere to go. \
                That IS the bug this test exists for: the poll path asked for no history.
                """
            )
            return
        }

        let before = try XCTUnwrap(position(app))
        XCTAssertEqual(before.offset, 0, "a pane opens at the live screen")

        app.otherElements["terminal-surface"].swipeDown(velocity: .slow)

        let after = try XCTUnwrap(position(app))
        XCTAssertGreaterThan(
            after.offset, before.offset,
            "swiping down did not move the view back into \(before.history) lines of scrollback"
        )
    }

    /// And swiping back the other way returns to the live screen, which is what
    /// resumes the poll loop — a pane that stayed frozen would look alive and
    /// be stale.
    func testSwipingBackDownReturnsToTheLiveScreen() throws {
        let app = launch()
        try openATerminal(app)

        let surface = app.otherElements["terminal-surface"]
        let hasHistory = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "NOT (value ENDSWITH %@)", "history=0"),
            object: surface
        )
        try XCTSkipUnless(
            XCTWaiter.wait(for: [hasHistory], timeout: 30) == .completed,
            "This pane has no scrollback to leave and come back from."
        )

        surface.swipeDown(velocity: .slow)
        try XCTSkipUnless(
            (position(app)?.offset ?? 0) > 0,
            "Could not get the view off the bottom; the scroll assertion is the other test."
        )

        for _ in 0..<6 where (position(app)?.offset ?? 0) > 0 {
            surface.swipeUp(velocity: .fast)
        }
        XCTAssertEqual(position(app)?.offset, 0, "the view never came back to the live screen")
    }
}
