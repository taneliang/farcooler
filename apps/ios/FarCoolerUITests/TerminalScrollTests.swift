import XCTest

/// A swipe on a terminal moves it into the pane's scrollback.
///
/// This is the regression that could not be caught anywhere else. The gesture
/// works and always did — a `UIPanGestureRecognizer` on the keyboard sink,
/// converting the drag to whole lines — but a pane on the capture-polling
/// fallback HELD a `VTCore` exactly as tall as the screen, so `scroll` had
/// nothing to move through and the swipe did nothing at all, silently,
/// forever. Nothing in the Rust tests could see that: the daemon was answering
/// every question it was asked correctly, and the phone was not asking for the
/// scrollback.
///
/// **The tense in that paragraph is load-bearing, and getting it wrong costs a
/// day.** The phone asks now — `TerminalScreenAsk`, on every open and on every
/// return to the live screen — so a POLLED pane carries the same history a
/// streamed one does. `source=poll` is therefore a fact about which painter is
/// feeding the pane and says nothing whatsoever about whether it can scroll. A
/// lane read it the old way, saw `source=poll history=2` on a fleet whose
/// runner held 1986 lines, and concluded scrollback was broken in the product.
/// It was not. What was broken was WHICH PANE the suite was reading — see
/// `visibleSurface`.
///
/// **And what the pane is has to be checked, every time.** These assertions are
/// satisfiable by a pane with nothing above it: a swipe that moves two lines
/// "scrolled into the scrollback", and a flick that clamps at the top
/// "travelled further than the finger". Measured against unmodified `main` on
/// the real fixture: one of these tests passed and three skipped, all four on a
/// bare two-line pane, while the pane they were written for sat two tabs away
/// with 1986 lines in it. So the walk is `openAPaneWithScrollback`, which keeps
/// going until it finds a pane worth measuring and goes RED — with a census of
/// every pane it saw — rather than quiet when the fleet has none.
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
        runner = LiveRunner.address
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
    /// neighboring tabs alive off-screen, so that query resolves to several
    /// elements and every use of it raises "Multiple matching elements found".
    /// The identifier names a KIND of element here, not one element.
    ///
    /// **It asks the pane, and falls back to the frame.** This used to be the
    /// frame alone — "the surface under the middle of the screen" — which is
    /// right for a neighboring TAB, laid out beside this one, and wrong for a
    /// neighboring WORKTREE, which is mounted at the same rect. The moment
    /// the demo fleet grew a second worktree with a terminal in it, two
    /// surfaces contained the middle and this returned whichever the
    /// accessibility tree listed first.
    ///
    /// It cost a day and it is worth writing down. On the fixture
    /// `scripts/demo-host.sh` builds, the pane in front had 1986 lines of
    /// scrollback and the tree ALSO held a bare two-line pane belonging to the
    /// `crossing` worktree. Every test in this file read that one:
    /// `testTheGridTracksTheThumbBetweenRows` failed naming the two lines,
    /// three tests skipped saying the pane was too shallow, and
    /// `testASwipeScrollsIntoTheScrollback` PASSED — a swipe does move two
    /// lines. A whole lane was spent on "scrollback is broken in the product"
    /// off the back of that, and the runner had 1986 lines the whole time.
    ///
    /// `visible=1` is the shell's own answer — `ShellPaneSlot.isVisible`, "the
    /// pane at rest, and there is exactly one in the whole track" — published
    /// on the surface for exactly this.
    ///
    /// The frame test stays underneath it so this never returns nothing, which
    /// several polling loops here rely on. It is a fallback and not a second
    /// answer: `openATerminalInTheShell` asserts the field is there before any
    /// test reads a number off a pane, because falling back silently is how the
    /// suite spent a day measuring a worktree nobody was looking at.
    private func visibleSurface(_ app: XCUIApplication) -> XCUIElement? {
        let middle = CGPoint(x: app.frame.midX, y: app.frame.midY)
        let all = app.otherElements.matching(identifier: "terminal-surface")
        var underTheMiddle: XCUIElement?
        for i in 0..<all.count {
            let element = all.element(boundBy: i)
            guard element.exists else { continue }
            if let value = element.value as? String, Self.field(value, "visible") == "1" {
                return element
            }
            if underTheMiddle == nil, element.frame.contains(middle) { underTheMiddle = element }
        }
        return underTheMiddle
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

    /// Wait until the pane in front of us reports at least this much scrollback.
    ///
    /// A block predicate rather than `ENDSWITH`, because the ENDSWITH version
    /// of this was vacuous — see `field`. It is worth the words: without
    /// history, "the swipe did not move the view" and "the view had nowhere to
    /// go" are the same observation, and only one of them is a bug.
    private func waitForHistory(
        _ app: XCUIApplication, atLeast lines: Int = 1, timeout: TimeInterval = 30
    ) -> Bool {
        let has = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in (self.position(app)?.history ?? 0) >= lines },
            object: nil)
        return XCTWaiter.wait(for: [has], timeout: timeout) == .completed
    }

    /// A fleet with no pane deep enough to measure.
    ///
    /// Thrown rather than skipped, and that is the whole point of it — see
    /// `openAPaneWithScrollback`. XCTest records an error out of a `throws`
    /// test as a failure and prints this description, so one throw is one red
    /// with the census attached.
    private struct NothingDeepEnough: Error, CustomStringConvertible {
        let description: String
    }

    /// Every `terminal-surface` in the tree and what it says about itself.
    ///
    /// Only ever used in a failure message, and it is the difference between
    /// "the pane reported no scrollback" and knowing WHICH pane was read and
    /// what the alternatives were. The whole of this lane's confusion fits in
    /// one of these lines.
    private func census(_ app: XCUIApplication) -> String {
        let all = app.otherElements.matching(identifier: "terminal-surface")
        var seen: [String] = []
        for i in 0..<all.count {
            let element = all.element(boundBy: i)
            guard element.exists, let value = element.value as? String else { continue }
            let front = Self.field(value, "visible") == "1" ? "in front" : "mounted"
            seen.append(
                "  \(front): history=\(Self.field(value, "history") ?? "?") "
                    + "source=\(Self.field(value, "source") ?? "?") "
                    + "mouse=\(Self.field(value, "mouse") ?? "?")")
        }
        return seen.isEmpty ? "  (no terminal-surface in the tree at all)" : seen.joined(separator: "\n")
    }

    /// Walk from wherever the app opens to a terminal pane.
    ///
    /// This used to tap "Worktrees" on the inbox and then a `fleet-terminal-`
    /// row in the list behind it. Neither exists: the app opens INTO the shell,
    /// on a worktree, and the way to another of its tabs is a swipe. So the
    /// walk is `openATerminalInTheShell`, which is now the only walk there is —
    /// kept as a name of its own so the three tests below go on reading as
    /// "reach a terminal, then assert about the terminal".
    @discardableResult
    private func openATerminal(_ app: XCUIApplication) throws -> XCUIElement {
        try openATerminalInTheShell(app)
    }

    /// Walk to a pane this suite can actually measure — one with scrollback
    /// above it — and go RED rather than quiet when the fleet has none.
    ///
    /// **This is the difference between a scroll suite and a check that cannot
    /// fail.** A pane with nothing above it is a pane on which every assertion
    /// in this file is vacuously satisfiable: a swipe that moves nothing is
    /// correct, and a swipe that moves two lines "scrolled into the
    /// scrollback". Both read as green. Measured, on the fixture
    /// `scripts/demo-host.sh` builds and against unmodified `main`:
    /// `testASwipeScrollsIntoTheScrollback` passed on a two-line pane while the
    /// pane the fixture built for it, two tabs away, held 1986 lines.
    ///
    /// So the walk keeps going. The demo fleet's terminals are not all equal —
    /// a worktree the daemon made carries a bare login shell, and the UI suite
    /// adds another every time it runs, because `NewTerminalTests` creates one
    /// and iOS has no way to close it — and any of those can sit in front of
    /// the pane the assertions were written for.
    ///
    /// Failing and not skipping when the walk comes up empty, deliberately. A
    /// laptop with no runner is already handled a level down, by
    /// `openATerminalInTheShell`, which skips; arriving HERE means the shell
    /// rendered, a terminal exists, and not one pane in the fleet has a line
    /// above its screen. That is a broken fixture or a broken product, never a
    /// legitimate configuration, and both deserve a red. The census says which.
    /// A hundred lines, not one, and the number is the whole guard.
    ///
    /// One line is a bar that a bare login shell clears — measured: the pane
    /// this walk first landed on reported `history=2`, which is more than zero
    /// and is not scrollback. A hundred is comfortably more than any phone
    /// screen is tall, so nothing that has merely printed a prompt can pass it,
    /// and comfortably under the 400 lines `scripts/demo-host.sh` prints, so a
    /// pane that has had some of them scroll off still counts.
    private static let enoughToMeasure = 100

    @discardableResult
    private func openAPaneWithScrollback(
        _ app: XCUIApplication, atLeast lines: Int = TerminalScrollTests.enoughToMeasure
    ) throws -> XCUIElement {
        _ = try openATerminalInTheShell(app)

        // One pass per tab in the flat sequence, plus slack, exactly as
        // `findAPaneThatWantsTheMouse` walks. The walk wraps, so a longer loop
        // would keep revisiting panes it has already rejected.
        for _ in 0..<8 {
            if let surface = visibleSurface(app), waitForHistory(app, atLeast: lines, timeout: 5) {
                return surface
            }
            let y = 0.42
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.78, dy: y))
            let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.22, dy: y))
            from.press(
                forDuration: 0.05, thenDragTo: to, withVelocity: .slow,
                thenHoldForDuration: 0.4)
            _ = waitForVisibleSurface(app, timeout: 3)
        }
        print(app.debugDescription)
        throw NothingDeepEnough(
            description: """
                No pane in this fleet has \(lines) line(s) of scrollback, so every scroll \
                assertion below would have measured nothing and reported success. \
                What the shell is holding:
                \(census(app))
                `scripts/demo-host.sh` puts 400 lines in the 'scrolling' worktree's two \
                panes. Re-run it. If it has been run and this still says two lines, the \
                history is being lost between tmux and the phone — compare \
                `tmux -L farcooler-$(cat "$TMPDIR/farcooler-demo-host/fc/install-id") \
                list-panes -a -F '#{history_size}'` against the numbers above.
                """)
    }

    func testASwipeScrollsIntoTheScrollback() throws {
        let app = launch()
        // The pane must HAVE history, or this test proves nothing — a swipe
        // that does not move on a pane with nothing above it is correct, and a
        // swipe that moves two lines on a bare prompt is this assertion passing
        // while measuring nothing. `openAPaneWithScrollback` walks past both
        // and fails, loudly, if the whole fleet is like that.
        try openAPaneWithScrollback(app)

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
        let surface = try openAPaneWithScrollback(app)

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

    /// One integer field off `terminal-surface` — `cell`, `grain`, `band`,
    /// `over`. See `TerminalScrollReadout` for what each one means.
    private func metric(_ app: XCUIApplication, _ key: String) -> Int? {
        surfaceValue(app).flatMap { Self.field($0, key) }.flatMap(Int.init)
    }

    /// Where the view came to rest, rather than where it happened to be when
    /// the finger left.
    ///
    /// A throw keeps moving for a couple of seconds after the gesture ends —
    /// that is the entire feature — so reading the offset straight after a
    /// swipe reads the middle of the coast. This waits for the number to stop
    /// changing, which is the only honest definition of "where it ended":
    /// hard-coding a sleep would either be too short (and measure the coast) or
    /// pin a duration the deceleration curve is free to change.
    private func settledOffset(_ app: XCUIApplication, timeout: TimeInterval = 10) -> Int? {
        // **Wait the coast out before asking anything.**
        //
        // Measured on the iPhone 17 simulator: one repaint of a 55x38 grid
        // through `TerminalView.draw` costs 35ms, idle or scrolling — the
        // renderer's own cost, not the physics'. A throw redraws for as long as
        // it coasts, so the app's main thread is busy for those seconds, and
        // every accessibility query XCUITest makes has to run on that same
        // thread ("XCTPerformOnMainRunLoop ... waiting with 30.00s
        // responsiveness timeout"). Polling through a coast is therefore
        // queueing behind the drawing, and it is how this test managed to get
        // its runner killed with `Test crashed with signal kill` four times in
        // a row while the app itself was perfectly alive.
        //
        // Three seconds is `UIScrollView.DecelerationRate.normal` from a hard
        // flick plus the spring that lands it, with room to spare.
        Thread.sleep(forTimeInterval: 3.0)
        let deadline = Date().addingTimeInterval(timeout)
        var last = position(app)?.offset
        var unchanged = 0
        while Date() < deadline {
            // Paced. A tight loop here queries the accessibility tree as fast
            // as the bridge will answer, against a view that is redrawing every
            // frame — which is a lot of pressure on the runner for no extra
            // information, since the thing being waited for takes seconds.
            Thread.sleep(forTimeInterval: 0.08)
            let now = position(app)?.offset
            unchanged = (now == last) ? unchanged + 1 : 0
            last = now
            // Four consecutive reads. Each round trip through the accessibility
            // tree is tens of milliseconds, so this is a few hundred
            // milliseconds of stillness — longer than a dropped frame and
            // shorter than the tail of any coast that is still moving.
            if unchanged >= 4 { return now }
        }
        return last
    }

    /// A drag with a real release velocity: no hold at the end, so the finger
    /// leaves the glass still moving and the content inherits it.
    ///
    /// **12000, and not `.fast`, and the difference is measured.** The velocity
    /// asked for here is what XCUITest interpolates the synthetic touches at;
    /// what the content actually inherits is whatever
    /// `UIPanGestureRecognizer.velocity(in:)` estimates from the events that
    /// arrive, and the two are nothing like each other. Measured on the iPhone
    /// 17 simulator against a 300-point drag, printing the recognizer's own
    /// number: `.fast` — nominally 3000 points a second — reaches the
    /// recognizer as **508**, which is a slow deliberate drag, not a flick.
    /// 12000 reaches it as **4453**, which is an ordinary hard thumb flick.
    ///
    /// So this is not a test cheating by asking for a superhuman gesture. It is
    /// the number that makes XCUITest produce a human one, and without it the
    /// momentum assertion below would be measuring a throw nobody would ever
    /// make on purpose.
    private func flick(_ surface: XCUIElement, fromY: CGFloat, toY: CGFloat) {
        let from = surface.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: fromY))
        let to = surface.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: toY))
        from.press(
            forDuration: 0.02, thenDragTo: to,
            withVelocity: XCUIGestureVelocity(rawValue: 12000), thenHoldForDuration: 0)
    }

    /// A drag that ends stopped: the hold at the end drains the velocity, so
    /// the content goes exactly as far as the thumb did and no further.
    private func dragAndStop(_ surface: XCUIElement, fromY: CGFloat, toY: CGFloat) {
        let from = surface.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: fromY))
        let to = surface.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: toY))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .default, thenHoldForDuration: 0.35)
    }

    /// **A flick goes further than the finger did, and nothing else here says so.**
    ///
    /// This is the one assertion momentum can be pinned by. Every other scroll
    /// test in this file asks whether the offset moved AT ALL, which a
    /// translation-to-lines conversion with no `.ended` branch satisfies
    /// perfectly — and did, for as long as the terminal had no physics at all.
    ///
    /// The number it compares against is the finger's own travel, converted to
    /// rows through `cell`. Without `TerminalScrollPhysics.projection` the
    /// content stops where the thumb stopped and `settled` lands within a row
    /// of `finger`; with it, `UIScrollView.DecelerationRate.normal` carries it
    /// several times further. WWDC 2018 803, on the version without: *"those
    /// same swipes wouldn't get you very far… you'd have to do these long,
    /// laborious swipes that would require a lot more manual input."*
    ///
    /// Negative control, run: with `release` taking `velocity = 0` — the
    /// release velocity thrown away, which is exactly the old behavior —
    ///
    ///     XCTAssertGreaterThan failed: ("0") is not greater than ("51")
    ///
    /// while `testTheGridTracksTheThumbBetweenRows` passed in the same run.
    func testAFlickTravelsFurtherThanTheFingerDid() throws {
        let app = launch()
        let surface = try openAPaneWithScrollback(app)

        let cell = try XCTUnwrap(metric(app, "cell"), "the pane never published its row height")
        try XCTSkipUnless(cell > 0, "the pane reported a zero row height")
        let before = try XCTUnwrap(position(app))
        XCTAssertEqual(before.offset, 0, "a pane opens at the live screen")

        // 45% of the pane, downward, which on this surface is back into the
        // scrollback. Stated in the surface's own frame so the finger's travel
        // is known in points and can be compared in rows.
        let fromY: CGFloat = 0.26
        let toY: CGFloat = 0.71
        let travel = surface.frame.height * (toY - fromY)
        let finger = Int(travel / CGFloat(cell))
        // Thrown, not skipped. A pane with room for the finger and not for the
        // throw makes this assertion unfalsifiable, and a skip says so where
        // nobody reads it — see `openAPaneWithScrollback`.
        guard before.history > finger * 4 else {
            throw NothingDeepEnough(
                description: """
                    \(before.history) lines of scrollback is not enough room for a throw \
                    worth \(finger) rows of finger to be told apart from a clamp at the top. \
                    What the shell is holding:
                    \(census(app))
                    """)
        }

        flick(surface, fromY: fromY, toY: toY)
        let settled = try XCTUnwrap(settledOffset(app))

        // Three times the finger, against a measured seven and a half.
        //
        // The margin is deliberately wide in both directions, because the two
        // outcomes this separates are not close. Measured with the projection:
        // 130 rows, for 17 rows of finger. Measured with `release` patched to
        // throw the release velocity away — the old behavior, exactly: **0**.
        //
        // Zero rather than seventeen, and the reason is worth knowing before
        // anybody reads too much into the ratio: XCUITest synthesizes a
        // 12000-point-a-second drag as almost no intermediate touches at all,
        // so the recognizer reports a release velocity and hardly any
        // `.changed`. A real thumb produces a dense stream and the tracked
        // distance is real. What this test can therefore say honestly is that
        // the content went a long way further than the gesture's own
        // displacement, and that removing the projection takes all of it away.
        XCTAssertGreaterThan(
            settled, finger * 3,
            """
            the flick carried no momentum: the finger travelled \(Int(travel)) points \
            (\(finger) rows of \(cell)) and the content came to rest \(settled) rows back, \
            which is no further than the gesture itself moved. That is the terminal \
            with no `.ended` branch.
            """
        )
    }

    /// **The key row's Hide Keyboard key puts the keyboard away.**
    ///
    /// The ruling on the bar behind the keyboard (the bar stays where it is,
    /// because an un-opted-out `GeometryReader` once made a terminal 0 points
    /// tall) makes dismissing the keyboard the way back to the bar, so the
    /// key that does it has to be there, be called something, and work.
    ///
    /// Asserted on the key itself as well as on the keyboard. XCUITest can
    /// attach a hardware keyboard to the simulator, and then no software
    /// keyboard is ever up and `app.keyboards.count == 0` holds before the tap
    /// as well as after it. The key row is the input accessory whatever
    /// keyboard is attached, so it is there only while the terminal is first
    /// responder, and its going is what proves the tap resigned it.
    func testTheHideKeyboardKeyPutsTheKeyboardAway() throws {
        let app = launch()
        let surface = try openATerminalInTheShell(app)
        hideKeyboard(app, raising: surface)
    }

    /// **A finger put down on a coasting pane stops it, and that touch is not
    /// also the tap that raises the keyboard.**
    ///
    /// Native `UIScrollView` resolves a tap-during-a-throw the same way every
    /// time: the touch kills the coast and goes no further. This view is not a
    /// `UIScrollView` — it runs its own momentum — so `touchesBegan` has to
    /// reproduce both halves on purpose: stop `motion` on touch DOWN rather
    /// than waiting for `focus()` to fire, and mark that touch spent so
    /// `focus()` declines to raise the keyboard for it. Getting only the first
    /// half right is worse than doing nothing: a pane that stops but still
    /// pops the keyboard open reads as the app answering a tap nobody made.
    ///
    /// A second, separate tap — made once the pane is actually at rest — must
    /// still raise the keyboard, or the fix would have traded one broken
    /// interaction for another (a pane that can never be focused by touch).
    /// Both halves are asserted here so a regression in either direction goes
    /// red.
    ///
    /// Flung the same way as `testAFlickTravelsFurtherThanTheFingerDid`, and
    /// the interrupted settle is checked against the same `finger * 3`
    /// threshold that test uses to prove momentum carried — inverted here to
    /// prove momentum did NOT carry once the second finger landed.
    func testTappingAMovingPaneStopsItWithoutRaisingTheKeyboard() throws {
        let app = launch()
        let surface = try openAPaneWithScrollback(app)

        // A pane raises the keyboard the moment it appears (see the note on
        // `testThePaneIsPaintedOnTheTerminalsOwnGround`), which would sit in
        // `app.keyboards.count` for the rest of the test and make the "did
        // NOT raise the keyboard" assertion below vacuous — true whether or
        // not the interrupting tap behaved, because something else already
        // put a keyboard up before either tap ran.
        //
        // A failure, not a skip, when it will not go: a skip here is a test
        // that can never go red on this path, only quiet.
        hideKeyboard(app, raising: surface)

        let cell = try XCTUnwrap(metric(app, "cell"), "the pane never published its row height")
        try XCTSkipUnless(cell > 0, "the pane reported a zero row height")
        let before = try XCTUnwrap(position(app))
        XCTAssertEqual(before.offset, 0, "a pane opens at the live screen")

        let fromY: CGFloat = 0.26
        let toY: CGFloat = 0.71
        let travel = surface.frame.height * (toY - fromY)
        let finger = Int(travel / CGFloat(cell))
        // Thrown, not skipped, for the reason `openAPaneWithScrollback` gives.
        guard before.history > finger * 4 else {
            throw NothingDeepEnough(
                description: """
                    \(before.history) lines of scrollback is not enough room for a throw \
                    worth \(finger) rows of finger to be told apart from a clamp at the top. \
                    What the shell is holding:
                    \(census(app))
                    """)
        }

        flick(surface, fromY: fromY, toY: toY)
        // No wait between the flick and the tap: `flick` returns the instant
        // the synthetic release lands, which is the moment the coast starts —
        // exactly when a real second finger landing "while it's still
        // scrolling" would arrive. XCUITest's own event-synthesis overhead
        // still puts a few hundred milliseconds between the two (measured:
        // ~0.6s), which is enough for a healthy coast to have already crossed
        // dozens of rows — so "how far back it ended up" cannot be the
        // assertion; see below for why it is "did it stop moving" instead.
        surface.tap()

        // Half a second is generous for a keyboard that a working `focus()`
        // never asked to raise — nothing here is waiting on network or the
        // runner, just a gesture recognizer's action closure running.
        Thread.sleep(forTimeInterval: 0.5)
        XCTAssertEqual(
            app.keyboards.count, 0,
            "the tap that stopped the coast also raised the keyboard — that touch "
                + "should have been spent stopping the scroll, not spent twice")

        // **"Stopped" is read off the clock, not off a distance.**
        //
        // A distance threshold (row N or fewer) assumes the interrupting tap
        // always lands at roughly the same point in the coast, but it does
        // not: XCUITest's own synthesis latency between the flick and the
        // follow-up tap varies run to run, and the coast is exponential — the
        // commit that added momentum measured 82 rows crossed by 0.54s into a
        // ~130-row throw, so a tap arriving anywhere from 0.4s to 0.8s in can
        // legitimately catch the pane already dozens of rows back. What a
        // WORKING interruption promises is not a row number, it is that the
        // row number stops changing the instant the tap lands. So this reads
        // the offset once shortly after the tap (letting the tiny
        // settle-onto-a-row spring finish; see `settleAfterATouchThatDidNotDrag`)
        // and again 1.5s later — long enough that an uninterrupted coast would
        // have crossed another several dozen rows (105 at 1.07s, 117 at 1.51s
        // in that same measurement) — and asks whether anything moved between
        // the two reads.
        Thread.sleep(forTimeInterval: 0.8)
        let stopped = try XCTUnwrap(position(app)?.offset)
        Thread.sleep(forTimeInterval: 1.5)
        let stillThere = try XCTUnwrap(position(app)?.offset)
        XCTAssertEqual(
            stillThere, stopped,
            """
            the pane kept moving after a finger landed on it: it was at \(stopped) \
            rows back shortly after the tap and \(stillThere) 1.5s later. A touch \
            landing mid-coast must stop it immediately and leave it stopped, not \
            merely slow it down.
            """
        )

        // The pane the interrupting tap left behind is landed on a row (see
        // `settleAfterATouchThatDidNotDrag`), so this second tap starts from
        // rest — the ordinary case `focus()` still has to serve.
        surface.tap()
        let raised = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in app.keyboards.count > 0 }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [raised], timeout: 10), .completed,
            "a plain tap on a pane already at rest must still raise the keyboard")
    }

    /// **The grid sits between two rows while a thumb is between two rows.**
    ///
    /// One-to-one tracking, which the talk calls "one of the principles of
    /// iOS": *"the moment the touch and content stop tracking one-to-one, we
    /// immediately notice it."* The emulator can only ever be on a whole row —
    /// `display_offset` is a line count — so the sub-row remainder is drawn as
    /// an offset instead of being rounded away, and `grain` is the deepest that
    /// offset got during the drag just made.
    ///
    /// A pane that quantises to `cellHeight`, which is what this file used to
    /// test, reports `grain=0` for every drag it will ever see. That is the
    /// negative control and it is exact: there is no way to produce a non-zero
    /// `grain` by rounding. Run, with the drawn offset forced to zero:
    ///
    ///     the grid never left a row boundary during a drag of several rows:
    ///     grain=0 against a 17-point row
    ///
    /// and `testASwipeScrollsIntoTheScrollback` passed in the same run, which
    /// is the pair that makes it evidence: quantising does not stop a swipe
    /// scrolling, it stops it tracking.
    ///
    /// Slowly and with a hold at the end, deliberately: this is about tracking,
    /// so the gesture must not also be a throw, and `band` is asserted to be
    /// zero so that a rubberband cannot stand in for the thing being measured.
    func testTheGridTracksTheThumbBetweenRows() throws {
        let app = launch()
        let surface = try openAPaneWithScrollback(app)

        let cell = try XCTUnwrap(metric(app, "cell"), "the pane never published its row height")
        try XCTSkipUnless(cell >= 8, "a row of \(cell) points has no room to be between")

        // Named, because the `band` sentence below has to be able to say how
        // many rows this gesture is worth. Written down once and used by both
        // the drag and the message, so the two cannot come to disagree.
        let fromY: CGFloat = 0.30
        let toY: CGFloat = 0.68
        let travel = surface.frame.height * (toY - fromY)
        let rowsNeeded = Int((travel / CGFloat(cell)).rounded(.up))
        let depth = position(app)?.history ?? -1

        dragAndStop(surface, fromY: fromY, toY: toY)

        let band = try XCTUnwrap(metric(app, "band"))
        // **The sentence names the pane, because the pane is usually the
        // reason, and the old one blamed the tracking.**
        //
        // The assertion is unchanged and deliberately so: a drag that
        // rubberbands has not measured tracking, and `grain` read after one is
        // a number about the wrong thing. What was wrong was the report. "This
        // drag ran over an end of the scrollback" is true of a broken clamp and
        // equally true of a pane that simply has nothing above it, and only the
        // first is a defect in the app — so the failure read as a terminal bug
        // and was, twice, a fleet whose first worktree held a bare prompt.
        //
        // `openATerminalInTheShell` stops at the FIRST terminal in the flat
        // sequence, whatever that pane happens to hold. `scripts/demo-host.sh`
        // puts 400 lines into `scrolling`'s panes and nothing into anybody
        // else's, and the app grows bare panes on its own: NewTerminalTests
        // creates one in the worktree the app opens on and iOS has no Close
        // Terminal to undo it with, so a suite run leaves one behind for the
        // next run to walk into. Printing both numbers is what tells the two
        // apart without another run.
        XCTAssertEqual(
            band, 0,
            """
            this drag ran over an end of the scrollback (band=\(band)), so `grain` would be \
            measuring the rubberband rather than the tracking. The drag is \(Int(travel)) \
            points, which is \(rowsNeeded) rows of \(cell), and this pane reported \(depth) \
            rows of history before it: if that is the smaller number then the pane is too \
            shallow for this gesture and the tracking has not been measured at all. \
            `scripts/demo-host.sh` puts 400 lines in `scrolling`; a bare prompt standing \
            in front of it in the fleet is the usual reason this pane is not that one.
            """)
        let grain = try XCTUnwrap(metric(app, "grain"))
        XCTAssertGreaterThan(
            grain, cell / 4,
            """
            the grid never left a row boundary during a drag of several rows: grain=\(grain) \
            against a \(cell)-point row. The content is quantising to whole rows and \
            lurching a row at a time under a thumb that moved smoothly.
            """
        )
        // Rounding to the NEAREST row is what bounds this: the drawn offset can
        // never be more than half a row from the row the emulator is on, and a
        // `grain` above that would mean the drawing and the emulator had come
        // apart rather than that the tracking was especially good.
        XCTAssertLessThanOrEqual(
            grain, cell / 2 + 2,
            "the grid was drawn \(grain) points from the row the emulator is on, which is "
                + "more than half of a \(cell)-point row — the drawing and `display_offset` "
                + "have drifted apart")
    }

    /// **The live screen resists too, and it is the end people hit.**
    ///
    /// The same rubberband as the test below, at the other end of the same
    /// clamp, and it is a separate test because it is a separate branch: over
    /// the top of the scrollback the content is held at `span`, and under the
    /// live screen it is held at zero. A pane opens AT this end, which is what
    /// makes this the cheap half — there is nothing to climb — and it is also
    /// the end a reader arrives at most often, by swiping back down to the
    /// prompt and going one swipe further.
    ///
    /// Before this, that swipe did nothing whatsoever: `VTCore.scroll` clamps
    /// at zero, so the pane sat still under a moving thumb. The talk, on
    /// exactly that: *"It would feel super harsh and disconcerting. You kind of
    /// hit a wall there."*
    ///
    /// Negative control, run, with `shown` reduced to a hard clamp:
    ///
    ///     the live screen is a wall: a 427-point drag below the newest line
    ///     moved the content 0 points, under one 17-point row
    func testOverDraggingPastTheLiveScreenResistsAndReturns() throws {
        let app = launch()
        let surface = try openATerminalInTheShell(app)
        _ = waitForVisibleSurface(app, timeout: 10)

        let cell = try XCTUnwrap(metric(app, "cell"), "the pane never published its row height")
        try XCTSkipUnless(cell > 0, "the pane reported a zero row height")
        let before = try XCTUnwrap(position(app))
        XCTAssertEqual(before.offset, 0, "a pane opens at the live screen, which is this end")

        // Upward, which is towards the newest line — and there is nothing newer
        // than the live screen, so every point of this is past the end.
        let fromY: CGFloat = 0.86
        let toY: CGFloat = 0.22
        let travel = surface.frame.height * (fromY - toY)
        dragAndStop(surface, fromY: fromY, toY: toY)

        let band = try XCTUnwrap(metric(app, "band"))
        XCTAssertGreaterThan(
            band, cell,
            """
            the live screen is a wall: a \(Int(travel))-point drag below the newest line moved \
            the content \(band) points, under one \(cell)-point row. Nothing on screen \
            distinguishes that from an app that has stopped responding.
            """
        )
        XCTAssertLessThan(
            CGFloat(band), travel * 0.55,
            """
            the content followed the finger \(band) points past the end of \(Int(travel)) \
            dragged, so there is no resistance at this boundary at all.
            """
        )
        XCTAssertEqual(
            position(app)?.offset, 0,
            "the emulator was pushed off the live screen by a drag that had nowhere to go")

        let returned = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in (self.metric(app, "over") ?? -1) == 0 }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [returned], timeout: 5), .completed,
            "the content was left hanging \(metric(app, "over") ?? -1) points below the live "
                + "screen instead of springing back")
    }

    /// **The top of the scrollback resists, and lets go.**
    ///
    /// The talk's rubberbanding, and the reason it is not a nicety: *"you
    /// actually wouldn't know the difference between a frozen phone, and phone
    /// that's just at the top of the edge of the screen."* A terminal that
    /// stopped dead at the oldest line read exactly like one that had stopped
    /// responding.
    ///
    /// Two assertions, because there are two ways to get this wrong and they
    /// are opposites. `band > cell` says the content MOVED — a wall reports
    /// zero. `band` well under the finger's travel says it RESISTED — content
    /// that follows the thumb one-to-one past the end has no boundary at all.
    /// And then `over` returning to zero is the elastic half: the content pulls
    /// itself back inside rather than being left hanging off the end.
    ///
    /// The peaks reset on every gesture, so what is measured is the one
    /// over-drag this test makes and not whatever the climb did.
    ///
    /// Negative control, run: with `shown` reduced to `min(max(position, 0),
    /// span)` — a hard clamp, which is what `VTCore.scroll` does on its own and
    /// what this pane did before —
    ///
    ///     the top of the scrollback is a wall: a 427-point drag past the
    ///     oldest line moved the content 0 points
    ///
    /// and `testOverDraggingPastTheLiveScreenResistsAndReturns` failed with the
    /// same sentence about the other end, in the same run.
    func testOverDraggingPastTheTopOfTheScrollbackResistsAndReturns() throws {
        let app = launch()
        let surface = try openAPaneWithScrollback(app)

        let cell = try XCTUnwrap(metric(app, "cell"), "the pane never published its row height")
        try XCTSkipUnless(cell > 0, "the pane reported a zero row height")

        // Climb until a flick stops buying anything, which is the oldest line
        // whatever number it turns out to be — this pane's history is whatever
        // its tmux scrollback has accumulated, not the 400 lines
        // `scripts/demo-host.sh` prints into it, and a streamed pane replays
        // the lot. Flicks rather than drags because a flick is worth a hundred
        // rows and a drag twenty: the bound is on iterations, and the number of
        // lines to climb is not something this test gets to decide.
        var top = 0
        var climbed = false
        for _ in 0..<8 {
            // Ten at a time, then one look. Reading between every flick would
            // spend most of the test waiting for coasts to finish, and a
            // finger put down on moving content stops it anyway — which is the
            // interruptibility this is relying on rather than working around.
            for _ in 0..<10 { flick(surface, fromY: 0.15, toY: 0.88) }
            let now = settledOffset(app) ?? 0
            if now == top {
                climbed = top > 0
                break
            }
            top = now
        }
        try XCTSkipUnless(
            climbed,
            "never reached the top of this pane's scrollback (stopped at offset \(top)); "
                + "the swipe assertion is testASwipeScrollsIntoTheScrollback's")

        // One more drag, all of which is past the end. Slow and ending
        // stopped, so that what is measured is resistance and not a throw, and
        // so the peaks below describe this gesture alone — they reset on every
        // `.began`.
        let fromY: CGFloat = 0.22
        let toY: CGFloat = 0.86
        let travel = surface.frame.height * (toY - fromY)
        dragAndStop(surface, fromY: fromY, toY: toY)

        let band = try XCTUnwrap(metric(app, "band"))
        XCTAssertGreaterThan(
            band, cell,
            """
            the top of the scrollback is a wall: a \(Int(travel))-point drag past the oldest \
            line moved the content \(band) points, which is less than one \(cell)-point row. \
            Nothing on screen distinguishes that from an app that has stopped responding.
            """
        )
        XCTAssertLessThan(
            CGFloat(band), travel * 0.55,
            """
            the content followed the finger \(band) points past the end of \(Int(travel)) \
            points dragged, so there is no resistance at the boundary at all — the scrollback \
            simply keeps going where there is nothing to show.
            """
        )

        let returned = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in (self.metric(app, "over") ?? -1) == 0 }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [returned], timeout: 5), .completed,
            "the content was left hanging \(metric(app, "over") ?? -1) points off the top "
                + "instead of springing back inside")
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
        let surface = try openAPaneWithScrollback(app)

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

    /// Reach a terminal in the 'scrolling' worktree.
    ///
    /// One swipe along the worktree, because `ShellFleetMap` puts Changes
    /// first and the terminals after it in fleet order. Repeated a few times
    /// rather than once, since the worktree may open on its Changes.
    private func openATerminalInTheShell(_ app: XCUIApplication) throws -> XCUIElement {
        // Needs You, then the demo's 'scrolling' worktree: the app opens on
        // the inbox now (ov-55), and a scoped shell holds one worktree, so
        // this is the one whose panes this suite measures. 180 s for a first
        // launch after an install, and a runner that isn't there is a
        // `LiveRunner.missing` skip, which the script turns red.
        try LiveRunner.openWorktree(app, named: "scrolling")
        for _ in 0..<6 {
            if let surface = waitForVisibleSurface(app, timeout: 3) {
                // The pane has to say which one it is, or nothing below this is
                // a measurement.
                //
                // `visibleSurface` falls back to the frame when no surface
                // publishes `visible=`, and the frame is ambiguous the moment a
                // second worktree is mounted — which is the whole defect this
                // field was added for. The fallback is there so the helper
                // never returns nothing; it is NOT a state this suite may run
                // in, because the app it reads is the app `xcodebuild test`
                // just built from this checkout. Silence here would put the
                // suite back to reading a pane in another worktree and
                // reporting green about it.
                XCTAssertNotNil(
                    Self.field(surface.value as? String ?? "", "visible"),
                    """
                    `terminal-surface` published no `visible=` field, so which pane is in \
                    front is a guess from frames — see `visibleSurface`. Restore it in \
                    TerminalView's accessibilityValue; every number this suite reads \
                    depends on it.
                    """)
                return surface
            }
            let y = 0.42
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.78, dy: y))
            let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.22, dy: y))
            from.press(
                forDuration: 0.05, thenDragTo: to, withVelocity: .slow,
                thenHoldForDuration: 0.4)
        }
        print(app.debugDescription)
        throw XCTSkip(
            "\(LiveRunner.missing) 'scrolling' has no terminal on \(runner); run ./scripts/demo-host.sh.")
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
    /// Failed, not skipped, when the pane is on any other painter. It used to
    /// skip "on a runner without the capability", but the runner here is the
    /// demo host, which runs this checkout's daemon, and every daemon this
    /// checkout builds advertises `terminal_stream` (`capability::ALL`). So the
    /// only way to reach the skip was the very fallback this test exists to
    /// catch — reported as a skip, which `xcodebuild` calls a success (ov-127).
    func testAPaneOnACapableRunnerStreams() throws {
        let app = launch()
        try openATerminal(app)

        // Read by name, not by suffix. This was `value ENDSWITH "source=stream"`,
        // which was correct only for as long as `source` happened to be the
        // last field — adding `mouse=` after it would have turned this test
        // into a permanent skip, quietly, with a message blaming the runner.
        let painter = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in self.source(app) == "stream" }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [painter], timeout: 30), .completed,
            """
            This pane is on \(source(app) ?? "an unknown painter"), not `stream`. The \
            demo host's daemon advertises `terminal_stream`, so the pane fell back. \
            If the demo host predates streaming, rebuild it with ./scripts/demo-host.sh.
            """)
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
    /// paints the same color on its own scroll view, which fills the pane;
    /// a grid is a `GeometryReader` in the SAFE region and fills nothing else.
    ///
    /// **Asserted on pixels, because there is nothing else to assert on.** A
    /// background is not an element and has no accessibility value; the only
    /// honest question is what color came out of the renderer. Three samples
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
    /// demo worktree, the same 400 lines in both, one left at an ordinary
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

        // Thrown, not skipped. `scripts/demo-host.sh` prints the same 400 lines
        // into this pane as into the one beside it, so "no scrollback here" is
        // a broken fixture or a broken wire and never a configuration.
        guard waitForHistory(app) else {
            throw NothingDeepEnough(
                description: """
                    The pane that asked for the mouse reported no scrollback, so a swipe \
                    has nowhere to go and this assertion cannot fail. What the shell is \
                    holding:
                    \(census(app))
                    """)
        }

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
    /// `scripts/demo-host.sh` puts exactly one of those in the demo worktree,
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
        hideKeyboard(app, raising: surface)

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

    // MARK: - What the pane reserves at the bottom

    /// **The grid runs down to the bar, not to a home indicator above it.**
    ///
    /// `ShellPaneRealView` insets its content by the shell's furniture, and
    /// the furniture is `safeArea.bottom + bar + gap` — one number that
    /// already contains the home indicator. The pane's own `NavigationStack`
    /// is a `UINavigationController` and re-derives the window's bottom inset
    /// on the far side of it, so reserving the whole of that number reserved
    /// the home indicator TWICE and the grid stopped 34 points short of the
    /// bar. It looks like nothing: a strip of the pane's own ground, in the
    /// pane's own color, which is why the pixel test above cannot see it.
    ///
    /// Asserted against the bar's measured frame rather than against 784,
    /// because the number is the device's and the RELATIONSHIP is the design:
    /// `chrome.bottom` is defined as reaching exactly the bar's top edge.
    func testTheGridRunsDownToTheBarsTopEdge() throws {
        let app = launch()
        let surface = try openATerminalInTheShell(app)
        XCTAssertTrue(surface.waitForExistence(timeout: 30))

        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 20), "the shell's bar never appeared")

        // The keyboard down first: with one up the correct answer is the
        // keyboard's top edge, which is the other test.
        hideKeyboard(app, raising: surface)

        // Waited for by watching the GRID, not by asking whether a keyboard
        // exists. `app.keyboards` is true the instant a field takes focus and
        // stays true for an accessory with no keyboard behind it, so a wait on
        // it is a wait on nothing. The grid's own bottom edge settling is the
        // thing this test is about, and the threshold is what makes the wait
        // able to fail: four fifths of the display is below every
        // keyboard-up answer and above every keyboard-down one, so a
        // predicate that fired before the keyboard moved would not be
        // satisfied by the frame it was looking at.
        //
        // Re-resolved on every poll rather than held: `visibleSurface` binds
        // by INDEX into a query that matches every mounted pane, and the
        // shell keeps neighbors alive, so an element captured once is not
        // guaranteed to still be the pane in front of you.
        var last: CGFloat = 0
        let settled = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in
                guard let now = self.visibleSurface(app)?.frame else { return false }
                defer { last = now.maxY }
                return abs(now.maxY - last) < 0.5 && now.maxY > app.frame.height * 0.8
            }, object: nil)
        _ = XCTWaiter.wait(for: [settled], timeout: 10)

        let grid = try XCTUnwrap(visibleSurface(app)?.frame, "no pane in front of us")
        print(
            "PANE-BOTTOM keyboard-down: grid \(grid.minY)…\(grid.maxY), "
                + "bar \(bar.frame.minY)…\(bar.frame.maxY), screen \(app.frame.height)")
        XCTAssertEqual(
            grid.maxY, bar.frame.minY, accuracy: 2,
            """
            the grid ends at \(grid.maxY) and the bar's top edge is at \(bar.frame.minY) \
            on a \(app.frame.height)-point display — \(bar.frame.minY - grid.maxY) points of \
            the pane reserved for furniture that is not there
            """
        )
    }

    /// **The pane's content stops at the nearest thing standing over it, and
    /// nothing is reserved for a bar the keyboard is covering.**
    ///
    /// Two things can be over the bottom of a pane: the shell's bar, which
    /// never moves for a keyboard (`ShellRootView.body`,
    /// `ShellPaneTrack.swift:53-61`), and the key row with whatever is under
    /// it. The content has to end at whichever is HIGHER, and reserving room
    /// for both is a strip of nothing — on a phone, while you are typing.
    ///
    /// One assertion covers both configurations this runs in, and it has to,
    /// because they are not a choice the test gets to make. With a hardware
    /// keyboard attached — which is the simulator's default — the software
    /// keyboard never rises, `app.keyboards` still reports an element parked
    /// BELOW the display, and the key row sits on the home indicator under the
    /// bar; the nearest thing is then the bar. With one detached the key row
    /// rides the keyboard up the screen and the nearest thing is the key row.
    /// Asserting on `min` of the two is the same sentence in both, and it is
    /// the sentence the design makes.
    func testThePaneStopsAtTheNearestThingOverIt() throws {
        let app = launch()
        let surface = try openATerminalInTheShell(app)
        XCTAssertTrue(surface.waitForExistence(timeout: 30))

        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 20), "the shell's bar never appeared")

        // The pane raises the keyboard on appear; tapping the grid asks again
        // for a run that arrived with it down.
        //
        // This test wants the row UP and measures it, so it waits for the key
        // to be tappable — the keyboard finished sliding in — and does not tap
        // it. Failing, not skipping, when the row never comes: the old skip
        // ("this pane has no key row") also caught a row read mid-slide.
        let dismiss = waitForHideKeyboardKey(app, raising: surface)
        // Existing is not being on screen. Measured on this simulator with a
        // hardware keyboard attached: `app.keyboards` reports one element at
        // y=891 on an 874-point display — an element that exists, has a frame,
        // and is nowhere anybody can see. So the key row is found through the
        // one control that is genuinely on screen in both configurations, and
        // its position is read rather than assumed.

        // The BUTTON's own top, with nothing subtracted for the row's padding
        // around it — measured, after a version of this that subtracted
        // `TerminalKeyRow`'s 7 points of vertical padding failed by exactly
        // those 7. With a software keyboard up the content ends at y=514 and
        // this button's top edge is y=514: whatever the row does with its
        // padding, the edge SwiftUI reserves to is the one the button starts
        // at. The hardware-keyboard case never noticed, because there the bar
        // is the nearer of the two and the row's number is not used.
        let rowTop = dismiss.frame.minY
        let nearest = min(bar.frame.minY, rowTop)

        let grid = try XCTUnwrap(visibleSurface(app)?.frame, "no pane in front of us")
        print(
            "PANE-BOTTOM nearest: grid \(grid.minY)…\(grid.maxY), bar top \(bar.frame.minY), "
                + "key row top \(rowTop), nearest \(nearest), screen \(app.frame.height)")
        XCTAssertEqual(
            grid.maxY, nearest, accuracy: 2,
            """
            the pane's last line is at \(grid.maxY) and the nearest thing over it is at \(nearest)             — the bar's top edge is \(bar.frame.minY) and the key row's is \(rowTop), so             \(nearest - grid.maxY) points of this pane are reserved for furniture nobody can see
            """
        )
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
    /// which on a strip 34 points tall is a different color and a passing test.
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
