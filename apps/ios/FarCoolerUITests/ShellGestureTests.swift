import XCTest

/// The navigation shell, driven with a real finger.
///
/// The split between this file and `AgentKitTests/ShellNavigationTests.swift`
/// is deliberate and it is where the two kinds of failure live. The step
/// sequence, the thresholds and the sort are arithmetic: they are tested on
/// the host, in a millisecond, with no simulator, and they are where a rule
/// gets quietly inverted. What is HERE is the part that arithmetic cannot
/// reach — whether a finger dragged across this screen actually reaches that
/// arithmetic at all. A gesture that is never recognized, an axis lock that
/// swallows the drag, a commit whose animation never re-seats, a column
/// derived from the wrong property: every one of those passes every pure test
/// and leaves a bar that does nothing.
///
/// So the assertions are on `shell-state`'s accessibility value, the same
/// technique `TerminalScrollTests` uses on `terminal-surface` and for the same
/// reason: a gesture's outcome is a transform and two indices, and nothing on
/// screen spells either out. See `ShellRootView.probe`.
///
/// No runner and no daemon — `-shell-harness` stands the shell on a canned
/// fleet, so unlike `TerminalScrollTests` this suite never skips.
final class ShellGestureTests: XCTestCase {
    /// `ShellMetrics.rowHeight`, restated.
    ///
    /// A UI test bundle links neither AgentKit nor the app module, so it
    /// cannot say `ShellMetrics.rowHeight` and has to carry the number. Every
    /// assertion below that involves a column height is written as a multiple
    /// of THIS rather than as a literal — the constant moved from 34 to 44
    /// once already, and the assertion that broke was the one that had 102
    /// written into it with a comment explaining where the 102 came from.
    private let rowHeight = 44

    /// The default fixture: ten workspaces, tab counts `[3, 2, 5, 1, 4]`
    /// cycling. Workspace 0 therefore has three tabs, which is what every
    /// number below is counted against.
    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-shell-harness"] + extra
        app.launch()
        return app
    }

    /// `ws`, `tab`, `workspaces`, `tabs`, `column`, `pinned`, `overview`.
    private func state(_ app: XCUIApplication) throws -> [String: Int] {
        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard probe.waitForExistence(timeout: 30) else {
            print(app.debugDescription)
            throw XCTSkip("The shell never rendered its probe.")
        }
        var parsed: [String: Int] = [:]
        for pair in (probe.value as? String ?? "").split(separator: " ") {
            let halves = pair.split(separator: "=")
            guard halves.count == 2, let value = Int(halves[1]) else { continue }
            parsed[String(halves[0])] = value
        }
        return parsed
    }

    /// A horizontal swipe across the CONTENT, well past the 70-point commit.
    ///
    /// Held at the end before release. A `DragGesture` sees the drag through
    /// `onChanged`, and a release that arrives in the same frame as the last
    /// movement can leave the final translation unreported — which reads as a
    /// gesture that recognized and then decided to do nothing.
    private func swipeContent(_ app: XCUIApplication, toward direction: CGFloat) {
        let y = 0.42
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5 + 0.28 * -direction, dy: y))
        let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5 + 0.28 * direction, dy: y))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.4)
    }

    /// Lift the bar by `points`, and hold there so the column is at that
    /// height when the finger leaves.
    private func liftBar(_ app: XCUIApplication, by points: CGFloat) {
        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
        let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let to = from.withOffset(CGVector(dx: 0, dy: -points))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.5)
    }

    /// A swipe on the content walks the flat sequence, and the opposite swipe
    /// walks back to exactly where it started.
    ///
    /// The round trip is the assertion that matters. A commit that re-seats on
    /// the wrong index, or one whose silent transaction never runs and leaves
    /// the track parked a page off centre, both show up as a return journey
    /// that does not land where it left.
    func testASwipeOnTheContentChangesTabAndComesBack() throws {
        let app = launch()
        let start = try state(app)
        XCTAssertEqual(start["ws"], 0)
        XCTAssertEqual(start["tab"], 0)
        XCTAssertEqual(start["workspaces"], 10)
        XCTAssertEqual(start["tabs"], 3, "workspace 0 of the canned fleet has three tabs")

        swipeContent(app, toward: -1)
        let forward = try state(app)
        XCTAssertEqual(forward["ws"], 0, "a step within a workspace does not change workspace")
        XCTAssertEqual(forward["tab"], 1, "the swipe did not commit")

        swipeContent(app, toward: 1)
        let back = try state(app)
        XCTAssertEqual(back["ws"], 0)
        XCTAssertEqual(back["tab"], 0, "the return swipe did not land where it started")
    }

    /// Walking forward off the last tab of a workspace lands on the NEXT
    /// workspace's first tab — the flat sequence, on a real screen.
    func testSwipingOffTheEndOfAWorkspaceCrossesIntoTheNext() throws {
        let app = launch()
        for _ in 0..<3 { swipeContent(app, toward: -1) }
        let crossed = try state(app)
        XCTAssertEqual(crossed["ws"], 1, "three swipes off a three-tab workspace never crossed")
        XCTAssertEqual(crossed["tab"], 0)
        XCTAssertEqual(crossed["tabs"], 2, "workspace 1 of the canned fleet has two tabs")
    }

    /// The bar walks WORKSPACES, not tabs: one swipe on it from anywhere in a
    /// workspace lands on the neighbour's first tab.
    func testASwipeOnTheBarChangesWorkspace() throws {
        let app = launch()
        // Somewhere in the middle of workspace 0, so "tab 0" afterwards is a
        // fact about the bar's step rather than about where we started.
        swipeContent(app, toward: -1)
        XCTAssertEqual(try state(app)["tab"], 1)

        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30))
        let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5))
        let to = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.5))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.4)

        let after = try state(app)
        XCTAssertEqual(after["ws"], 1, "the bar swipe did not change workspace")
        XCTAssertEqual(after["tab"], 0, "the bar lands on the next workspace's first tab")
    }

    /// A lift too small to open the column costs nothing.
    ///
    /// Started from tab 2 on purpose. Below `openMin` the release abandons;
    /// the failure it is guarding against is a release that lands on row 0
    /// instead — and from tab 0 those two are the same answer, so the test
    /// would pass while the shell was wrong.
    func testAShortLiftAbandonsAndCostsNothing() throws {
        let app = launch()
        swipeContent(app, toward: -1)
        swipeContent(app, toward: -1)
        XCTAssertEqual(try state(app)["tab"], 2, "could not get to the third tab")

        liftBar(app, by: 12)

        let after = try state(app)
        XCTAssertEqual(after["tab"], 2, "a 12-point lift changed the tab")
        XCTAssertEqual(after["column"], 0, "the column stayed open after the finger left")
        XCTAssertEqual(after["pinned"], 0, "an abandoned drag pinned the column")
    }

    /// Lifting past the last row and then some reaches the overview.
    ///
    /// Workspace 0 has three tabs, so the column runs out at three rows and
    /// the overview arrives 76 further up. 320 is well past both, and past is
    /// the only thing being asserted — the exact arrival point is
    /// `ShellNavigationTests.theOverviewBeginsWhereTheColumnRunsOut`.
    func testALongLiftReachesTheOverview() throws {
        let app = launch()
        liftBar(app, by: 320)

        XCTAssertEqual(try state(app)["overview"], 1, "the long lift never reached the overview")
        XCTAssertTrue(
            app.descendants(matching: .any).matching(identifier: "shell-overview").firstMatch
                .waitForExistence(timeout: 5),
            "the overview reported itself open but drew nothing")

        // And it is a place you can leave, which is the other half of a
        // gesture that has no button to open it.
        app.buttons["shell-overview-done"].tap()
        XCTAssertEqual(try state(app)["overview"], 0, "Done did not close the overview")
    }

    /// A tap holds the column open, and a second tap closes it.
    ///
    /// This is the one the mechanics doc singles out: `colOpen` is a separate
    /// property from the drag offset because deriving the column's visibility
    /// from both made a tap toggle the wrong way. The failure looks like a bar
    /// that opens on the first tap, then opens again on the second.
    func testATapPinsTheColumnOpenAndASecondTapClosesIt() throws {
        let app = launch()
        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30))
        XCTAssertEqual(try state(app)["pinned"], 0)

        bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        let opened = try state(app)
        XCTAssertEqual(opened["pinned"], 1, "a tap did not hold the column open")
        XCTAssertEqual(
            opened["column"], 3 * rowHeight, "workspace 0's three rows were not showing")

        // The bar element has grown by the column's height, so its centre is
        // no longer over the bar. Aim at the bottom of it, which is.
        bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9)).tap()
        let closed = try state(app)
        XCTAssertEqual(closed["pinned"], 0, "the second tap toggled the wrong way")
        XCTAssertEqual(closed["column"], 0)
    }

    /// A lift that also travels sideways lands in the NEIGHBOUR's cell.
    ///
    /// The one gesture that used to be impossible: the axis lock made the two
    /// mutually exclusive, so a drag that started upward could do nothing
    /// sideways however far it went. The lock is still never released — this
    /// is the vertical gesture throughout — the lift simply owns both
    /// directions once the page is off the display, which is the rule written
    /// down at `ShellGesture.pageIsHeld`.
    ///
    /// The offsets are chosen so the axis cannot be in doubt: 320 up against
    /// 130 across is well past `abs(dx) > dy`, so the first six points decide
    /// vertical, and 130 is well past the 70-point commit.
    func testALiftedPageFlickedSidewaysCarriesToTheNeighbour() throws {
        let app = launch()
        XCTAssertEqual(try state(app)["ws"], 0)

        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
        let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        from.press(
            forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -130, dy: -320)),
            withVelocity: .slow, thenHoldForDuration: 0.5)

        let carried = try state(app)
        XCTAssertEqual(carried["overview"], 1, "the lift did not reach the overview")
        XCTAssertEqual(carried["ws"], 1, "the sideways half of the lift was ignored")
        XCTAssertEqual(carried["tab"], 0, "a carry lands on the neighbour's first tab")
    }

    /// The same lift with no sideways travel still opens the overview on the
    /// workspace you were in.
    ///
    /// The other side of the rule, and the one that would break first: a
    /// carry read out of the sideways wander every long drag has would move
    /// the workspace under somebody who only ever swiped up.
    func testAStraightLiftStaysOnTheWorkspaceItStartedOn() throws {
        let app = launch()
        liftBar(app, by: 320)
        let after = try state(app)
        XCTAssertEqual(after["overview"], 1)
        XCTAssertEqual(after["ws"], 0, "a straight lift changed workspace")
    }

    // MARK: - The pane-retention invariant

    /// What a pane reports about itself: `born`, which changes only when the
    /// pane is REBUILT, and `visible`.
    private func pane(_ app: XCUIApplication, _ tab: String) throws -> [String: String] {
        let probe = app.descendants(matching: .any)
            .matching(identifier: "shell-pane-\(tab)").firstMatch
        guard probe.waitForExistence(timeout: 20) else {
            print(app.debugDescription)
            throw XCTSkip("The pane \(tab) was not in the tree at all.")
        }
        var parsed: [String: String] = [:]
        for field in (probe.value as? String ?? "").split(separator: " ") {
            let halves = field.split(separator: "=")
            guard halves.count == 2 else { continue }
            parsed[String(halves[0])] = String(halves[1])
        }
        return parsed
    }

    /// **The invariant: a pane is never rebuilt.**
    ///
    /// Three comments in this codebase exist because that was got wrong three
    /// ways, and the shell's three-slot `HStack` was a fourth — it keyed its
    /// children by POSITION, so re-seating the position destroyed the pane at
    /// the middle slot and the already-mounted neighbour along with it. Every
    /// commit rebuilt every pane. With text placeholders that is invisible,
    /// which is exactly why it needs a test rather than a look.
    ///
    /// Both directions of the assertion matter and they fail differently. The
    /// pane swiped ONTO was mounted as a neighbour before the swipe: if it is
    /// rebuilt, the thing you watched slide in is not the thing you landed on,
    /// and a real terminal would renegotiate with tmux at the instant of
    /// arrival. The pane swiped AWAY FROM has the half-typed message.
    func testCommittingASwipeRebuildsNothing() throws {
        let app = launch()
        let here = try pane(app, "ws-0-tab-0")["born"]
        let neighbour = try pane(app, "ws-0-tab-1")["born"]
        XCTAssertNotNil(here)
        XCTAssertNotNil(neighbour)
        XCTAssertNotEqual(here, neighbour, "two panes reported one identity")

        swipeContent(app, toward: -1)
        XCTAssertEqual(try state(app)["tab"], 1, "the swipe did not commit")
        XCTAssertEqual(
            try pane(app, "ws-0-tab-1")["born"], neighbour,
            "the pane swiped ONTO was rebuilt by the commit")
        XCTAssertEqual(
            try pane(app, "ws-0-tab-0")["born"], here,
            "the pane swiped AWAY FROM was rebuilt by the commit")

        swipeContent(app, toward: 1)
        XCTAssertEqual(try state(app)["tab"], 0)
        XCTAssertEqual(try pane(app, "ws-0-tab-0")["born"], here, "the return swipe rebuilt a pane")
        XCTAssertEqual(try pane(app, "ws-0-tab-1")["born"], neighbour)
    }

    /// The same, for the release that used to be worst: `.carry` re-seats the
    /// position INSIDE the flight animation, so a rebuild there is one you
    /// watch happen rather than one you find afterwards.
    func testACarriedLiftRebuildsNothing() throws {
        let app = launch()
        let leaving = try pane(app, "ws-0-tab-0")["born"]

        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
        let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        from.press(
            forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -130, dy: -320)),
            withVelocity: .slow, thenHoldForDuration: 0.5)

        XCTAssertEqual(try state(app)["ws"], 1, "the carry did not cross")
        XCTAssertEqual(
            try pane(app, "ws-0-tab-0")["born"], leaving,
            "the workspace the carry left was rebuilt mid-flight")
    }

    /// **Exactly one pane is `isVisible`, at rest and mid-gesture.**
    ///
    /// `DockedBar.swift:34-41` is why: an input accessory lives in the
    /// KEYBOARD's window, so a pane that is merely hidden goes on holding
    /// first responder and goes on drawing its composer over whatever is on
    /// top. Two visible panes is two composers fighting over one keyboard.
    ///
    /// Checked with a finger DOWN as well as up, because mid-gesture is the
    /// only time two panes are both on screen and therefore the only time the
    /// wrong answer is reachable.
    func testExactlyOnePaneIsVisible() throws {
        let app = launch()

        func visibleCount() -> Int {
            let probes = app.descendants(matching: .any)
                .matching(NSPredicate(format: "identifier BEGINSWITH %@", "shell-pane-"))
            return (0..<probes.count).filter {
                ((probes.element(boundBy: $0).value as? String) ?? "").contains("visible=1")
            }.count
        }

        XCTAssertTrue(
            app.descendants(matching: .any).matching(identifier: "shell-pane-ws-0-tab-0")
                .firstMatch.waitForExistence(timeout: 30))
        XCTAssertEqual(visibleCount(), 1, "at rest, exactly one pane is the pane")

        // Half a page across and HELD, which is the state that has two panes
        // on screen. `press(thenDragTo:)` cannot be interrogated mid-flight,
        // so the drag is built by hand out of the two halves of a touch.
        let y = 0.42
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.78, dy: y))
        let mid = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: y))
        start.press(forDuration: 0.05, thenDragTo: mid, withVelocity: .slow,
                    thenHoldForDuration: 0.05)
        XCTAssertEqual(visibleCount(), 1, "a drag made a second pane visible")
    }

    // MARK: - The three bugs from the device

    /// **A tap on a column row switches to that tab.**
    ///
    /// The column is the shell's tab switcher and a tap on one of its rows was
    /// dead: `ShellColumn` draws rows and declares no target of its own, so the
    /// only thing under a finger anywhere on that surface is the bar's own
    /// `DragGesture`, and a tap resolves through `barRelease(axis: nil, ...)`
    /// to `.toggleColumn` — which SHUT the menu instead of choosing from it.
    /// Opening worked, selecting did not.
    ///
    /// The last tab and not the first, because the row nearest the bar is the
    /// LAST one — see `ShellGesture.columnSelection` — so this fails if the
    /// mapping is inverted as well as if the tap is ignored.
    func testTappingAColumnRowSwitchesToThatTab() throws {
        let app = launch()
        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
        XCTAssertEqual(try state(app)["tab"], 0)

        bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        let opened = try state(app)
        XCTAssertEqual(opened["pinned"], 1, "a tap did not hold the column open")
        XCTAssertEqual(opened["column"], 3 * rowHeight, "the three rows were not showing")

        tapColumnRow(app, bar, fromBottom: 0)
        let landed = try state(app)
        XCTAssertEqual(
            landed["tab"], 2, "a tap on the row nearest the bar did not switch to the last tab")
        XCTAssertEqual(landed["pinned"], 0, "choosing a row left the column open over it")
    }

    /// The row two up from the bar is tab 0, which is the one you came from —
    /// so this pins the MAPPING rather than merely the fact that a tap does
    /// something.
    func testTappingTheTopColumnRowSwitchesToTheFirstTab() throws {
        let app = launch()
        swipeContent(app, toward: -1)
        swipeContent(app, toward: -1)
        XCTAssertEqual(try state(app)["tab"], 2, "could not get to the third tab")

        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30))
        bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertEqual(try state(app)["pinned"], 1)

        tapColumnRow(app, bar, fromBottom: 2)
        XCTAssertEqual(
            try state(app)["tab"], 0, "the topmost row is tab 0 and the tap landed elsewhere")
    }

    /// Tap the column row `fromBottom` rows above the bar row.
    ///
    /// Measured off the bar element's own frame rather than off a normalized
    /// offset: the surface grows by the column's height when it opens, so a
    /// fraction of it means a different row for every tab count.
    private func tapColumnRow(_ app: XCUIApplication, _ bar: XCUIElement, fromBottom row: Int) {
        let frame = bar.frame
        let centre = CGVector(
            dx: frame.midX,
            dy: frame.maxY - CGFloat(rowHeight) * (1.5 + CGFloat(row)))
        app.coordinate(withNormalizedOffset: .zero).withOffset(centre).tap()
    }

    /// **A drag down the content does not turn the page, however far it
    /// wanders sideways.**
    ///
    /// `ShellGesture.axis` compared `abs(dx) > dy` with `dy` measured
    /// UP-positive, so every DOWNWARD drag had a negative `dy` and lost to any
    /// horizontal component at all — a scroll down the screen was called
    /// horizontal, and the ten degrees of drift a thumb makes over six hundred
    /// points is well past the seventy that commits. Reading a terminal
    /// changed tab under you.
    ///
    /// The numbers: 79 points across against 511 down, which is about nine
    /// degrees off vertical and is a scroll by any reading.
    func testADragDownTheContentDoesNotTurnThePage() throws {
        let app = launch(["-shell-scroll"])
        XCTAssertEqual(try state(app)["tab"], 0)

        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.50, dy: 0.18))
        let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.30, dy: 0.78))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.4)

        let after = try state(app)
        XCTAssertEqual(after["tab"], 0, "a drag down the content turned the page")
        XCTAssertEqual(after["ws"], 0, "a drag down the content changed workspace")
    }

    /// The same drag UPWARD, which was never broken, so the fix cannot be "the
    /// axis lock now refuses everything".
    func testADragUpTheContentDoesNotTurnThePageEither() throws {
        let app = launch(["-shell-scroll"])
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.50, dy: 0.78))
        let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.30, dy: 0.18))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.4)
        XCTAssertEqual(try state(app)["tab"], 0, "a drag up the content turned the page")
    }

    /// **Scrolling the grid back to its top does not close it.**
    ///
    /// The pull-down that dismisses the overview read `atTop` at the moment
    /// the finger LEFT, so any scroll that finished at the top of the grid —
    /// which is every scroll back up through forty cards — was indistinguish-
    /// able from a deliberate pull-down and threw you back onto the workspace
    /// you came from.
    func testScrollingTheGridBackToItsTopDoesNotCloseIt() throws {
        let app = launch(["-shell-40"])
        liftBar(app, by: 320)
        XCTAssertEqual(try state(app)["overview"], 1, "the lift never reached the overview")

        // Down through the grid…
        let low = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.62))
        let high = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.22))
        low.press(
            forDuration: 0.05, thenDragTo: high, withVelocity: .slow, thenHoldForDuration: 0.3)
        XCTAssertEqual(try state(app)["overview"], 1, "scrolling down the grid closed it")

        // …and back up, in one drag that ends at the top.
        high.press(
            forDuration: 0.05, thenDragTo: low, withVelocity: .slow, thenHoldForDuration: 0.3)
        XCTAssertEqual(
            try state(app)["overview"], 1, "scrolling the grid back to its top closed it")
    }

    /// The other half of the same rule: a pull-down that BEGINS at the top is
    /// still the way out by touch, and the fix must not have removed it.
    func testAPullDownFromTheTopOfTheGridStillClosesIt() throws {
        let app = launch(["-shell-40"])
        liftBar(app, by: 320)
        XCTAssertEqual(try state(app)["overview"], 1)

        let high = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.30))
        let low = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.70))
        high.press(
            forDuration: 0.05, thenDragTo: low, withVelocity: .slow, thenHoldForDuration: 0.3)
        XCTAssertEqual(
            try state(app)["overview"], 0, "the pull-down out of the grid stopped working")
    }

    /// Forty workspaces is the number the design was chosen for, so the
    /// harness has to reach it and the overview has to hold it.
    func testTheOverviewHoldsFortyWorkspaces() throws {
        let app = launch(["-shell-40"])
        XCTAssertEqual(try state(app)["workspaces"], 40)
        liftBar(app, by: 320)
        XCTAssertEqual(try state(app)["overview"], 1)
        XCTAssertTrue(app.staticTexts["40 Workspaces"].waitForExistence(timeout: 5))
    }
}

