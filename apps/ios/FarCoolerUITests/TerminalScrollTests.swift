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
    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += extra
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
            parts.count == 3,
            let offset = Int(parts[0].replacingOccurrences(of: "offset=", with: "")),
            let history = Int(parts[1].replacingOccurrences(of: "history=", with: ""))
        else { return nil }
        return (offset, history)
    }

    /// "stream" or "poll" — which painter is feeding the pane.
    private func source(_ app: XCUIApplication) -> String? {
        let surface = app.otherElements["terminal-surface"]
        guard surface.exists, let value = surface.value as? String else { return nil }
        return value.split(separator: " ").last.map {
            $0.replacingOccurrences(of: "source=", with: "")
        }
    }

    /// Walk from wherever the app opens to a terminal pane.
    ///
    /// This used to tap "Workspaces" on the inbox and then a `fleet-terminal-`
    /// row in the list behind it. Neither exists: the app opens INTO the shell,
    /// on a workspace, and the way to another of its tabs is a swipe. So the
    /// walk is `openATerminalInTheShell`, which is now the only walk there is —
    /// kept as a name of its own so the three tests below go on reading as
    /// "reach a terminal, then assert about the terminal".
    @discardableResult
    private func openATerminal(_ app: XCUIApplication) throws -> XCUIElement {
        try openATerminalInTheShell(app)
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

    // MARK: - The same pane, inside the navigation shell

    /// **The shell must not steal the gesture the pane already owns.**
    ///
    /// The shell's content swipe is a horizontal `DragGesture` over the whole
    /// pane, and the terminal's scroll is a `UIPanGestureRecognizer` on the
    /// keystroke sink — the view that already owns every touch landing on the
    /// terminal (`TerminalView.swift:945-970`). Two recognizers over one
    /// surface is a race, and the way it is lost is silent: the swipe is
    /// recognized as a page turn, or as nothing, and a terminal that will not
    /// scroll looks exactly like a terminal with no scrollback.
    ///
    /// So this is `testASwipeScrollsIntoTheScrollback`'s assertion made about
    /// the arbitration rather than about the emulator, and it is the reason the
    /// shell can be trusted to carry a real pane at all. The two were one
    /// launch argument apart while the shell was behind `-shell-live`; the
    /// shell is the app now, so they are the same launch and the difference is
    /// only what each one is watching. Kept separate deliberately: they fail
    /// for different reasons, and a suite that merged them would report a
    /// stolen gesture as a broken scrollback.
    func testTheShellDoesNotStealTheTerminalsScroll() throws {
        let app = launch()
        let surface = try openATerminalInTheShell(app)

        let hasHistory = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "NOT (value ENDSWITH %@)", "history=0"),
            object: surface
        )
        try XCTSkipUnless(
            XCTWaiter.wait(for: [hasHistory], timeout: 30) == .completed,
            "This pane has no scrollback, so a swipe has nowhere to go."
        )

        let before = try XCTUnwrap(position(app))
        XCTAssertEqual(before.offset, 0, "a pane opens at the live screen")

        surface.swipeDown(velocity: .slow)

        let after = try XCTUnwrap(position(app))
        XCTAssertGreaterThan(
            after.offset, before.offset,
            "the shell swallowed the pane's own scroll: \(before.history) lines above and the "
                + "view never left the bottom"
        )
    }

    /// And the other half of the same race: the shell's own swipe still works
    /// with a live terminal under it.
    ///
    /// The two failures are opposite and both are silent. If the terminal's
    /// pan always wins, the shell's page turn is unreachable from a terminal
    /// pane — you can get into one and never swipe out. If the shell always
    /// wins, the pane will not scroll. Only a runner can tell them apart,
    /// because only a runner produces a pane with a `UIPanGestureRecognizer`
    /// on it.
    func testTheShellStillTurnsThePageOverALiveTerminal() throws {
        let app = launch()
        _ = try openATerminalInTheShell(app)

        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        func place() -> String {
            (probe.value as? String ?? "").split(separator: " ")
                .filter { $0.hasPrefix("ws=") || $0.hasPrefix("tab=") }.joined(separator: " ")
        }
        let before = place()
        XCTAssertFalse(before.isEmpty, "the shell never reported where it was")

        // BACKWARD along the sequence, which is the direction that is
        // guaranteed to have somewhere to go: the pane was reached by swiping
        // forward, so the tab behind it exists. Forward from here may be the
        // end of the fleet, where the correct answer is a rubber band and
        // nothing else — a test that swiped that way would pass or fail on
        // how many terminals the demo runner happens to have.
        let y = 0.42
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.22, dy: y))
        let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.78, dy: y))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.4)

        // Polled rather than read once. The commit animates for a third of a
        // second and re-seats in its completion — deliberately, so the page
        // does not bounce — so the shell is still settling when the finger
        // comes up.
        let moved = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in place() != before }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [moved], timeout: 5), .completed,
            "a horizontal swipe over a live terminal did not move the shell: \(probe.value ?? "")")
    }

    /// **Coming back to a workspace reopens the tab you left it on.**
    ///
    /// The owner's case, and it needs a runner because it runs through the one
    /// memory the app already keeps: `Connection.lastFocus`, written when
    /// somebody moves between the tabs of a workspace and read back by
    /// `ShellFleetMap.resume` as the tab a deliberate arrival lands on. The
    /// pure half is `ShellNavigationTests.theBarStepLandsOnTheRememberedTab`;
    /// what only a runner can show is that the two halves are wired together.
    ///
    /// **It comes back to the DIFF, and that is the whole design of the test.**
    /// Landing on the terminal would prove nothing: `PaneFocus.rule` — the
    /// fallback for a workspace nobody has chosen a tab in — picks the
    /// top-ranked agent, which on this runner is that same terminal, so a
    /// memory that was never written and a memory that was read back give the
    /// same answer. Parking on the diff first is the one choice the rule would
    /// not have made. A negative control confirms it: with the write to
    /// `lastFocus` removed, this fails and the earlier shape passed.
    ///
    /// The bar swipe rather than the content swipe, deliberately: the content
    /// walks a continuum and must stay literal, which
    /// `theContentStepIgnoresTheRememberedTab` pins from the other side.
    func testCrossingBackToAWorkspaceReopensTheTabYouLeft() throws {
        let app = launch()
        _ = try openATerminalInTheShell(app)

        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        func field(_ name: String) -> Int? {
            (probe.value as? String ?? "").split(separator: " ")
                .first { $0.hasPrefix("\(name)=") }
                .flatMap { Int($0.split(separator: "=")[1]) }
        }
        let home = try XCTUnwrap(field("ws"))
        try XCTSkipUnless(
            (field("tab") ?? 0) > 0,
            "This runner's terminal is already the first tab, so there is no tab behind it "
                + "to choose.")
        try XCTSkipUnless(
            (field("workspaces") ?? 0) > 1, "One workspace: there is nowhere to cross to.")

        // Back one tab, inside this workspace. That is the move that is a
        // CHOICE, and the only kind of move the memory records.
        let y = 0.42
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.22, dy: y)).press(
            forDuration: 0.05,
            thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.78, dy: y)),
            withVelocity: .slow, thenHoldForDuration: 0.4)
        let chose = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in field("tab") == 0 && field("ws") == home },
            object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [chose], timeout: 5), .completed,
            "could not get onto the diff: \(probe.value ?? "")")

        // Away along the BAR, and back the same way. Two arrivals, each
        // landing on whatever that workspace's `resume` resolved to.
        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 20))
        func swipeBar(_ from: CGFloat, _ to: CGFloat) {
            bar.coordinate(withNormalizedOffset: CGVector(dx: from, dy: 0.5)).press(
                forDuration: 0.05,
                thenDragTo: bar.coordinate(withNormalizedOffset: CGVector(dx: to, dy: 0.5)),
                withVelocity: .slow, thenHoldForDuration: 0.4)
        }
        let away = home > 0 ? (0.1, 0.85) : (0.85, 0.1)
        swipeBar(away.0, away.1)
        let left = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in field("ws") != home }, object: nil)
        try XCTSkipUnless(
            XCTWaiter.wait(for: [left], timeout: 5) == .completed,
            "The bar swipe did not cross; there is nothing to come back from.")

        swipeBar(away.1, away.0)
        let returned = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in field("ws") == home }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [returned], timeout: 5), .completed,
            "never came back: \(probe.value ?? "")")
        XCTAssertEqual(
            field("tab"), 0,
            "coming back landed on tab \(field("tab") ?? -1) — the rule's answer — rather than "
                + "on the diff it was left on: \(probe.value ?? "")")
    }

    /// Reach a terminal from the shell, which opens on the Diff tab.
    ///
    /// One swipe along the flat sequence, because `ShellFleetMap` puts Changes
    /// first in every workspace and the terminals after it in fleet order.
    /// Repeated a few times rather than once: the demo fleet's first workspace
    /// may have no terminal at all, in which case the sequence spills into the
    /// next workspace, which is the behaviour rather than a failure.
    private func openATerminalInTheShell(_ app: XCUIApplication) throws -> XCUIElement {
        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard probe.waitForExistence(timeout: 60) else {
            print(app.debugDescription)
            throw XCTSkip("The shell never rendered; run ./scripts/demo-host.sh first.")
        }
        let surface = app.otherElements["terminal-surface"]
        for _ in 0..<6 {
            if surface.waitForExistence(timeout: 3) { return surface }
            let y = 0.42
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.78, dy: y))
            let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.22, dy: y))
            from.press(
                forDuration: 0.05, thenDragTo: to, withVelocity: .slow,
                thenHoldForDuration: 0.4)
        }
        print(app.debugDescription)
        throw XCTSkip("No terminal in this fleet; run ./scripts/demo-host.sh first.")
    }

    /// On a runner that advertises `terminal_stream`, a pane must actually
    /// stream — not merely look fine.
    ///
    /// This is the assertion the whole streaming defect went years without.
    /// A polled pane repaints several times a second and reads perfectly, so
    /// "the terminal works" was true and useless: every enrolled device had
    /// silently fallen back, and the only visible symptom was scrollback that
    /// did not exist. Asserting on the PAINTER rather than on the picture is
    /// what makes that observable.
    ///
    /// Skipped rather than failed on a runner without the capability — that is
    /// a legitimate configuration and the fallback has its own tests above.
    func testAPaneOnACapableRunnerStreams() throws {
        let app = launch()
        try openATerminal(app)

        let painter = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "value ENDSWITH %@", "source=stream"),
            object: app.otherElements["terminal-surface"]
        )
        guard XCTWaiter.wait(for: [painter], timeout: 30) == .completed else {
            throw XCTSkip(
                """
                This pane is on \(source(app) ?? "an unknown painter"), which is \
                correct for a runner that does not advertise `terminal_stream`. \
                Rebuild the demo host to exercise the attach path.
                """
            )
        }
        XCTAssertEqual(source(app), "stream")
    }
}
