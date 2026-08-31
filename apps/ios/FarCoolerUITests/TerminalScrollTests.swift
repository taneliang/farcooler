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
///
/// Which is why `./scripts/ios-ui-tests.sh` is the way to run this, and not a
/// convenience. Skipping is the right answer for one laptop and the wrong
/// answer for a suite: eight of these skipped for a night while the owner was
/// looking at a terminal that would not scroll, and the run still printed
/// `** TEST SUCCEEDED **`. That script invokes xcodebuild the one way that
/// forwards the runner (see `launch` below) and then refuses a run in which
/// nothing executed.
final class TerminalScrollTests: XCTestCase {
    /// The runner string the app was last launched against.
    ///
    /// Only so the skip below can say it. "The shell never rendered" is true of
    /// a runner that is down, a runner on the wrong port, and a runner reached
    /// as `@127.0.0.1:2222` because `DEMO_USER` never arrived — three causes,
    /// one sentence, and the third is invisible unless the sentence prints the
    /// string. It cost a night.
    private var runner = "<never launched>"

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
        //
        // The empty user came back a second way, and the second way leaves no
        // mark at all. xcodebuild forwards `TEST_RUNNER_<VAR>` from ITS OWN
        // ENVIRONMENT — the assignment goes BEFORE the command:
        //
        //     TEST_RUNNER_DEMO_USER=$(whoami) xcodebuild test …    ✅
        //     xcodebuild test … TEST_RUNNER_DEMO_USER=$(whoami)    ❌
        //
        // Written after it, it is not an environment variable, it is a
        // command-line build setting. xcodebuild takes it without complaint,
        // `-showBuildSettings` reports it set to exactly the value you meant,
        // and it reaches nothing: `DEMO_USER` is unset here, the app launches
        // on "@127.0.0.1:2222", sshd logs `Invalid user`, and every test in
        // this file skips. Use `./scripts/ios-ui-tests.sh`, which has the
        // assignment in the right place and fails a run where nothing ran.
        let user = ProcessInfo.processInfo.environment["DEMO_USER"] ?? ""
        let host = ProcessInfo.processInfo.environment["DEMO_HOST"] ?? "127.0.0.1:2222"
        runner = "\(user)@\(host)"
        app.launchArguments += ["-farcoolerDemoHost", runner]
        app.launch()
        return app
    }

    /// One `key=value` off `terminal-surface`, or nil.
    ///
    /// By NAME, never by position or by count. The parser this replaces
    /// required exactly three space-separated parts and read them by index, and
    /// the waits beside it asked `NOT (value ENDSWITH "history=0")` — which was
    /// already false of every value the app has ever published, because the
    /// string ends with `source=`. So three "wait until this pane has
    /// scrollback" guards were passing instantly on any pane at all, including
    /// one with nothing above it, which is the difference between a swipe that
    /// is broken and a swipe that correctly did nothing.
    ///
    /// A string that several tests read positionally is a string nobody can add
    /// a field to. Reading it by name is what let `mouse=` be added at all.
    static func field(_ value: String, _ key: String) -> String? {
        value.split(separator: " ")
            .first { $0.hasPrefix(key + "=") }
            .map { String($0.dropFirst(key.count + 1)) }
    }

    /// The `terminal-surface` the shell is actually showing.
    ///
    /// `app.otherElements["terminal-surface"]` is not it, and stopped being it
    /// the moment the demo fleet grew a second terminal: the shell keeps
    /// neighbouring tabs alive off-screen, so that query resolves to several
    /// elements and every use of it raises "Multiple matching elements found".
    /// The identifier names a KIND of element here, not one element.
    ///
    /// The visible one is the one under the middle of the screen. Chosen by
    /// frame rather than by `firstMatch`, which returns whichever the
    /// accessibility tree happens to list first — off-screen panes included,
    /// and their values are live, so a test could read `mouse=on` off a pane
    /// nobody is looking at and swipe a different one entirely.
    private func visibleSurface(_ app: XCUIApplication) -> XCUIElement? {
        let centre = CGPoint(x: app.frame.midX, y: app.frame.midY)
        let all = app.otherElements.matching(identifier: "terminal-surface")
        for i in 0..<all.count {
            let element = all.element(boundBy: i)
            guard element.exists else { continue }
            if element.frame.contains(centre) { return element }
        }
        return nil
    }

    private func surfaceValue(_ app: XCUIApplication) -> String? {
        visibleSurface(app)?.value as? String
    }

    /// `visibleSurface`, polled, for the moment just after a tab has moved.
    private func waitForVisibleSurface(
        _ app: XCUIApplication, timeout: TimeInterval
    ) -> XCUIElement? {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if let surface = visibleSurface(app) { return surface }
        } while Date() < deadline
        return nil
    }

    /// `offset=N history=M` off the surface, or nil while it has not appeared.
    private func position(_ app: XCUIApplication) -> (offset: Int, history: Int)? {
        guard
            let value = surfaceValue(app),
            let offset = Self.field(value, "offset").flatMap(Int.init),
            let history = Self.field(value, "history").flatMap(Int.init)
        else { return nil }
        return (offset, history)
    }

    /// "stream" or "poll" — which painter is feeding the pane.
    private func source(_ app: XCUIApplication) -> String? {
        surfaceValue(app).flatMap { Self.field($0, "source") }
    }

    /// Whether the program in this pane has asked for mouse events.
    private func wantsMouse(_ app: XCUIApplication) -> Bool? {
        surfaceValue(app).flatMap { Self.field($0, "mouse") }.map { $0 == "on" }
    }

    /// Wait until the pane in front of us reports scrollback above it.
    ///
    /// A block predicate rather than `ENDSWITH`, because the ENDSWITH version
    /// of this was vacuous — see `field`. It is worth the words: without
    /// history, "the swipe did not move the view" and "the view had nowhere to
    /// go" are the same observation, and only one of them is a bug.
    private func waitForHistory(_ app: XCUIApplication, timeout: TimeInterval = 30) -> Bool {
        let has = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in (self.position(app)?.history ?? 0) > 0 },
            object: nil)
        return XCTWaiter.wait(for: [has], timeout: timeout) == .completed
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
        guard waitForHistory(app) else {
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

        try XCTUnwrap(visibleSurface(app)).swipeDown(velocity: .slow)

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

        let surface = try XCTUnwrap(waitForVisibleSurface(app, timeout: 10))
        try XCTSkipUnless(
            waitForHistory(app), "This pane has no scrollback to leave and come back from.")

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

        try XCTSkipUnless(
            waitForHistory(app), "This pane has no scrollback, so a swipe has nowhere to go.")

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
        // 180 seconds, not 60, and the number is measured rather than chosen.
        //
        // `xcodebuild test` installs a fresh build, and the first launches of a
        // freshly installed app on the simulator are far slower than the rest:
        // in one run of TerminalScrollTests the first three launches never rendered
        // inside 60s, the fourth took about 40, and the last four took about
        // five each. So a 60-second probe did not test the app, it tested how
        // recently the app had been installed — and it failed that test by
        // SKIPPING, which is the one outcome that looks like nothing is wrong.
        //
        // The cost is that a genuinely absent runner now takes three minutes
        // per test to say so. That is the right way round: a slow correct
        // answer beats a fast one that reads as success, and
        // `scripts/ios-ui-tests.sh` now makes an all-skipped run red, so this
        // path is only reached when something really is broken.
        guard probe.waitForExistence(timeout: 180) else {
            print(app.debugDescription)
            throw XCTSkip(
                "The shell never rendered against \(runner); run "
                    + "./scripts/demo-host.sh first, then ./scripts/ios-ui-tests.sh.")
        }
        for _ in 0..<6 {
            if let surface = waitForVisibleSurface(app, timeout: 3) { return surface }
            let y = 0.42
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.78, dy: y))
            let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.22, dy: y))
            from.press(
                forDuration: 0.05, thenDragTo: to, withVelocity: .slow,
                thenHoldForDuration: 0.4)
        }
        print(app.debugDescription)
        throw XCTSkip("No terminal in the fleet on \(runner); run ./scripts/demo-host.sh first.")
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

        // Read by name, not by suffix. This was `value ENDSWITH "source=stream"`,
        // which was correct only for as long as `source` happened to be the
        // last field — adding `mouse=` after it would have turned this test
        // into a permanent skip, quietly, with a message blaming the runner.
        let painter = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in self.source(app) == "stream" }, object: nil)
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

    // MARK: - What the pane is painted on

    /// **A terminal's own ground runs the whole pane, top edge to bottom.**
    ///
    /// The owner's report was "there are black bars above and below terminals
    /// which looks very weird", and the two strips it names are the two the VT
    /// grid does not cover: the pane's navigation bar at the top, and whatever
    /// the shell's furniture or the keyboard has left at the bottom.
    ///
    /// `ShellPaneRealView` does say `.background(TerminalPalette.background
    /// .ignoresSafeArea())` — and it says it OUTSIDE the pane's
    /// `NavigationStack`, which is a `UINavigationController` and paints its
    /// own opaque `systemBackground` over the top. Under a terminal's dark
    /// scheme that is pure black. A diff never showed it because `ChangesView`
    /// paints the same colour on its own scroll view, which fills the pane;
    /// a grid is a `GeometryReader` in the SAFE region and fills nothing else.
    ///
    /// **Asserted on pixels, because there is nothing else to assert on.** A
    /// background is not an element and has no accessibility value; the only
    /// honest question is what colour came out of the renderer. Three samples
    /// down the left margin — inside the grid's own padding, so no glyph can
    /// land on any of them — and the two outside the grid have to match the one
    /// inside it.
    ///
    /// A pane whose program asked for the mouse must STILL scroll.
    ///
    /// This is the assertion every other scroll test in this file was
    /// structurally incapable of making, and the gap was reported from a real
    /// phone as "scroll is broken for terminals" while all of them stayed
    /// green.
    ///
    /// `TerminalSession.scroll` used to hand the wheel to the program whenever
    /// the program would take it — that is, whenever mouse reporting was on.
    /// Every test here reaches the demo runner's idle prompt, which has mouse
    /// reporting OFF, so the suite only ever ran the local branch. The other
    /// branch was live on the phone, where a full-screen TUI turns the mouse on
    /// and then does nothing visible with a wheel event: the swipe went out
    /// over ssh, the program ignored it, and the pane could not be scrolled at
    /// all. A phone has no second scroll affordance to fall back on.
    ///
    /// **The mouse mode is set on the runner, not typed here.** The first
    /// version of this tapped the pane, waited for the software keyboard and
    /// called `app.typeText("printf '\\e[?1000h'")`. That cannot work:
    /// `KeystrokeSink` is a plain `UIView` holding first responder so the
    /// keyboard has somewhere to attach, and XCUITest will not synthesize
    /// typing without a focused element it recognizes as a text input — it
    /// fails with "Neither element nor any descendant has keyboard focus". It
    /// is not a flake and no amount of waiting fixes it; a terminal pane has no
    /// text element to type into, by construction.
    ///
    /// So `scripts/demo-host.sh` builds the pane instead: two terminals in the
    /// demo workspace, the same 400 lines in both, one left at an ordinary
    /// prompt and one whose shell has written `\e[?1000h`. The mode then
    /// travels the whole path a real program's would — tmux, the daemon, the
    /// wire, the phone's VT core — rather than being simulated at the near end,
    /// and this walks the shell's tabs until it finds the pane that reports
    /// `mouse=on`.
    ///
    /// That the pane reports `mouse=on` at all is half the evidence, and it is
    /// asserted rather than assumed. Without it a runner that quietly dropped
    /// the escape would leave this swiping an ordinary pane and passing for the
    /// wrong reason — which is how the suite got into last night's state.
    ///
    /// Negative control, run: with the alternate-screen guard in
    /// `TerminalSession.scroll` reverted to the old "encode a wheel event, and
    /// fall back only if the core will not" rule, this fails with
    ///
    ///     XCTAssertGreaterThan failed: ("0") is not greater than ("0") — the
    ///     pane asked for the mouse and then could not be scrolled: offset=0,
    ///     with 1578 lines above it
    ///
    /// while `testASwipeScrollsIntoTheScrollback`, which swipes the pane beside
    /// it that has NOT asked for the mouse, passes in the same run. That pair
    /// is the point: one revert, two panes differing in one bit, and only the
    /// one the rule is about goes red. A test that failed on both would be
    /// telling you something had broken, not which thing.
    func testAPaneThatAskedForTheMouseStillScrolls() throws {
        let app = launch()
        let surface = try findAPaneThatWantsTheMouse(app)

        try XCTSkipUnless(
            waitForHistory(app),
            "The pane reported no scrollback, so a swipe has nowhere to go.")

        // Re-read after the wait rather than trusting the walk: arriving at a
        // pane and its first full paint are not the same moment, and `mouse=`
        // is read off the emulator's live state.
        XCTAssertEqual(
            wantsMouse(app), true,
            "this pane stopped reporting mouse=on before the swipe, so whatever happens "
                + "next says nothing about a pane that wants the mouse")

        let before = try XCTUnwrap(position(app))
        XCTAssertEqual(before.offset, 0, "a pane opens at the live screen")

        surface.swipeDown(velocity: .slow)

        let after = try XCTUnwrap(position(app))
        XCTAssertGreaterThan(
            after.offset, before.offset,
            """
            the pane asked for the mouse and then could not be scrolled: \
            offset=\(after.offset), with \(after.history) lines above it
            """
        )
    }

    /// Walk the shell's tabs to the pane whose program has asked for the mouse.
    ///
    /// `scripts/demo-host.sh` puts exactly one of those in the demo workspace,
    /// beside an otherwise identical pane that has not. Skipped, with the
    /// script named, when there is no such pane: an older demo host has one
    /// terminal and this test has nothing to say about it.
    private func findAPaneThatWantsTheMouse(_ app: XCUIApplication) throws -> XCUIElement {
        _ = try openATerminalInTheShell(app)

        // One pass per tab in the flat sequence, plus a little slack. The walk
        // wraps, so a longer loop would keep revisiting panes it has rejected.
        for _ in 0..<8 {
            if let surface = visibleSurface(app), wantsMouse(app) == true { return surface }
            let y = 0.42
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.78, dy: y))
            let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.22, dy: y))
            from.press(
                forDuration: 0.05, thenDragTo: to, withVelocity: .slow,
                thenHoldForDuration: 0.4)
            _ = waitForVisibleSurface(app, timeout: 3)
        }
        print(app.debugDescription)
        throw XCTSkip(
            "No pane in this fleet reports mouse=on. Re-run ./scripts/demo-host.sh, which "
                + "creates one; a demo host from before it did has only the plain pane.")
    }

    /// Negative control, run: with the `.background` inside `TerminalView`
    /// removed, this fails with
    /// `above the grid the pane is #000000, inside it #2E3440`.
    func testThePaneIsPaintedOnTheTerminalsOwnGround() throws {
        let app = launch()
        let surface = try openATerminalInTheShell(app)
        XCTAssertTrue(surface.waitForExistence(timeout: 30))
        // With the keyboard down, so the strip below the grid is the shell's
        // furniture rather than the key row. The pane raises the keyboard on
        // appear; the key row's own button is the way back down.
        let dismiss = app.buttons["keyboard.chevron.compact.down"]
        if dismiss.exists { dismiss.tap() }
        let down = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in app.keyboards.count == 0 }, object: nil)
        _ = XCTWaiter.wait(for: [down], timeout: 5)

        let grid = surface.frame
        try XCTSkipUnless(
            grid.minY > 24 && grid.maxY < app.frame.height - 24,
            "The grid fills the display, so there are no strips to be wrong.")

        let shot = XCUIScreen.main.screenshot().image
        // x = 3, which is inside `TerminalMetrics.padding` — the grid's own
        // 6-point margin — so the sample inside the grid is ground and never a
        // glyph.
        let x: CGFloat = 3
        let inside = try XCTUnwrap(shot.colorAt(x: x, y: grid.minY + 8), "no pixel inside")
        let above = try XCTUnwrap(shot.colorAt(x: x, y: grid.minY - 20), "no pixel above")
        let below = try XCTUnwrap(shot.colorAt(x: x, y: grid.maxY + 10), "no pixel below")
        XCTAssertEqual(
            above, inside,
            "above the grid the pane is \(above.hex), inside it \(inside.hex)")
        XCTAssertEqual(
            below, inside,
            "below the grid the pane is \(below.hex), inside it \(inside.hex)")
    }

}

/// One pixel of a screenshot, as three bytes.
///
/// `Equatable` with no tolerance on purpose: the question this answers is
/// "which of two flat fills is this", and the two candidates in the bug it
/// exists for are `#2E3440` and `#000000`. A tolerance wide enough to survive
/// compression is wide enough to call those two the same on a dark theme.
struct ScreenPixel: Equatable {
    var red: UInt8
    var green: UInt8
    var blue: UInt8

    var hex: String { String(format: "#%02X%02X%02X", red, green, blue) }
}

extension UIImage {
    /// The pixel at a point in the app's own coordinates.
    ///
    /// A screenshot is in PIXELS and a frame is in POINTS, so the scale is
    /// applied here rather than at each call site — getting that wrong reads a
    /// point a third of the way down the screen from the one being asked about,
    /// which on a strip 34 points tall is a different colour and a passing test.
    func colorAt(x: CGFloat, y: CGFloat) -> ScreenPixel? {
        guard let cg = cgImage else { return nil }
        let px = Int((x * scale).rounded())
        let py = Int((y * scale).rounded())
        guard px >= 0, py >= 0, px < cg.width, py < cg.height else { return nil }
        var bytes = [UInt8](repeating: 0, count: 4)
        guard
            let space = CGColorSpace(name: CGColorSpace.sRGB),
            let context = CGContext(
                data: &bytes, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                space: space,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        context.draw(
            cg, in: CGRect(x: -CGFloat(px), y: -CGFloat(cg.height - py - 1), width: CGFloat(cg.width), height: CGFloat(cg.height)))
        return ScreenPixel(red: bytes[0], green: bytes[1], blue: bytes[2])
    }
}
