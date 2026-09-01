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

    /// Lift the bar by `points` and let go while still moving.
    ///
    /// The same distance as `liftBar` and the opposite release: no hold, so
    /// the finger leaves the glass while still moving, which is the only thing
    /// that differs between the two.
    ///
    /// Every other gesture in this file holds for 0.4 or 0.5 seconds before
    /// releasing, and `ShellRootView.stillFor` makes that hold mean exactly
    /// zero velocity rather than nearly zero — so the rest of this suite is a
    /// negative control for the projection: a release with no momentum
    /// projects nowhere, and not one of their outcomes may change.
    private func flickBar(_ app: XCUIApplication, by points: CGFloat) {
        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
        let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let to = from.withOffset(CGVector(dx: 0, dy: -points))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .fast, thenHoldForDuration: 0)
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

    /// **A flick up from over a menu row reaches the overview, and a
    /// deliberate lift from the same place stays in the workspace.** The
    /// owner's complaint, and the whole of the momentum projection on one
    /// screen.
    ///
    /// The talk's PIP example reproduced as the *before* case — *"the issue
    /// here is that we're only looking at position, we're completely ignoring
    /// the momentum"*. The overview used to be reachable only by dragging a
    /// full 76 points past the last row and stopping there, so a flick, which
    /// is how anybody who has used a task switcher asks for a grid of cards,
    /// landed on whichever row the thumb happened to be passing.
    ///
    /// **The distances are chosen for what they prove, and they differ.** 60
    /// points pins the ROW, because that is where the two mappings disagree:
    /// the bar's centre sits 22 points below its own top edge, so a 60-point
    /// lift puts the fingertip 38 points up the column — inside the row
    /// nearest the bar, which is the LAST tab. The delta mapping this replaced
    /// read the 60 rather than the 38 and answered tab 1, a whole row above
    /// the thumb, and the further down the bar a drag began the worse it got.
    /// 140 pins the ESCAPE, because it leaves the fingertip 118 points up —
    /// squarely on the TOP row, with the column's last 14 points and the
    /// overview's whole 76-point run still ahead of it. A release there is a
    /// release from over a menu item by any reading, and it is the one the
    /// owner reported landing on the item.
    ///
    /// 140 rather than 60 for the flick because a synthesized flick's velocity
    /// is not repeatable: the same `.fast` drag measured 284 points per second
    /// on one run of this simulator and 803 on the next. From 140 the escape
    /// needs 136, so the slower of those two still clears it twice over; from
    /// 60 it needs 297 and the test would be a coin toss on the machine rather
    /// than a statement about the app.
    func testAFlickUpFromOverAMenuRowReachesTheOverview() throws {
        let app = launch()
        XCTAssertEqual(try state(app)["tabs"], 3, "workspace 0 of the canned fleet has three tabs")
        XCTAssertEqual(try state(app)["tab"], 0)

        // The row, off the finger's position rather than off its travel.
        liftBar(app, by: 60)
        let landed = try state(app)
        XCTAssertEqual(landed["overview"], 0, "a deliberate 60-point lift left the workspace")
        XCTAssertEqual(
            landed["tab"], 2,
            "the row under the finger is the one nearest the bar, which is the last tab")

        // The escape, and its negative control first: the same 140 points,
        // held still before the finger leaves, projects nowhere.
        liftBar(app, by: 140)
        let held = try state(app)
        XCTAssertEqual(held["overview"], 0, "a lift that stopped before letting go still escaped")
        XCTAssertEqual(held["tab"], 0, "and it chose the top row, which is where the finger was")

        flickBar(app, by: 140)
        XCTAssertEqual(
            try state(app)["overview"], 1,
            "a flick from the same 140 points stayed in the workspace")
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
    /// **The mechanism the rest of the redirection generalises**, and it was
    /// here first. Past the last row the page is off the display and the two
    /// axes stop competing: the lift decides whether you stay up, sideways
    /// decides which cell you land in, and `ShellGesture.lean` stops being
    /// asked for the rest of the gesture — `ShellBarDrag.holdingPage` latches
    /// and answers `.vertical` whatever the thumb does sideways. That is the
    /// one thing the axis lock was genuinely protecting, and it is protected
    /// by a narrower rule now rather than by refusing to look.
    ///
    /// The offsets are chosen so the lean cannot be in doubt anywhere along
    /// the path: this is a straight line at 130 across to 320 up, so its
    /// running ratio is 0.41 for every frame of it and no threshold in
    /// `lean` is anywhere near. 130 is also well past the 70-point commit.
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
    /// the workspace under somebody who only ever swiped up. It is also the
    /// negative control for the redirection — a gesture with no sideways
    /// component has nothing to redirect INTO, so the lean must never fire.
    func testAStraightLiftStaysOnTheWorkspaceItStartedOn() throws {
        let app = launch()
        liftBar(app, by: 320)
        let after = try state(app)
        XCTAssertEqual(after["overview"], 1)
        XCTAssertEqual(after["ws"], 0, "a straight lift changed workspace")
    }

    /// **A diagonal drag on the bar resolves by which way it leans, and it
    /// leans the way it always did.**
    ///
    /// The negative control for making the bar's axis redirectable, on a real
    /// finger. `XCUIElement.press(forDuration:thenDragTo:)` interpolates a
    /// STRAIGHT line, so its running `|dx| : |dy|` is constant — and along a
    /// constant ratio an incumbent axis is by construction already the larger
    /// of the two and can never be beaten by `ShellMetrics.redirect` times
    /// itself. Which is to say: no gesture this suite can synthesize
    /// redirects, and every one of the bar tests above therefore means today
    /// exactly what it meant when the axis was decided once and never
    /// revisited. `ShellNavigationTests.aStraightDragMeansExactlyWhatItAlwaysDid`
    /// proves that for every angle in five-degree steps; this proves the
    /// finger reaches it.
    ///
    /// **The same two numbers, swapped.** 80 across against 52 up is a
    /// workspace crossing; 52 across against 80 up is a tab chosen off the
    /// column. Both are well inside the 19° band `redirect` would hold a
    /// gesture through if either of them ever got there — they are 33° and
    /// 57° — and both are unambiguous outcomes rather than two flavours of
    /// nothing: the first changes workspace, the second changes tab while
    /// leaving the workspace alone.
    ///
    /// 52 across is also deliberately SHORT of the 70-point commit, so a
    /// gesture that leaned the wrong way would spring back and change no tab,
    /// and 80 up puts the fingertip 58 points above the bar row — inside the
    /// second row from the bar, which on a three-tab column is tab 1.
    func testADiagonalDragOnTheBarResolvesByWhichWayItLeans() throws {
        let app = launch()
        XCTAssertEqual(try state(app)["tabs"], 3, "workspace 0 of the canned fleet has three tabs")
        XCTAssertEqual(try state(app)["ws"], 0)
        XCTAssertEqual(try state(app)["tab"], 0)

        // Leaning vertical: 52 across, 80 up.
        dragBar(app, by: CGVector(dx: -52, dy: -80))
        let lifted = try state(app)
        XCTAssertEqual(lifted["ws"], 0, "a drag that leans vertical changed workspace")
        XCTAssertEqual(
            lifted["tab"], 1,
            "the fingertip was over the second row from the bar, which is tab 1")
        XCTAssertEqual(lifted["overview"], 0)

        // Leaning horizontal: the same two numbers the other way round.
        dragBar(app, by: CGVector(dx: -80, dy: -52))
        let crossed = try state(app)
        XCTAssertEqual(crossed["ws"], 1, "a drag that leans horizontal did not change workspace")
        XCTAssertEqual(crossed["tab"], 0, "the bar lands on the next workspace's first tab")
        XCTAssertEqual(crossed["overview"], 0)
    }

    /// One straight drag from the bar's centre, held before release so it
    /// throws nothing.
    private func dragBar(_ app: XCUIApplication, by offset: CGVector) {
        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
        let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        from.press(
            forDuration: 0.05, thenDragTo: from.withOffset(offset), withVelocity: .slow,
            thenHoldForDuration: 0.5)
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
    /// LAST one — see `ShellGesture.columnRow` — so this fails if the mapping
    /// is inverted as well as if the tap is ignored.
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

    /// **The grid moves DURING the pull, not only at the end of it — and it
    /// comes back if you change your mind.**
    ///
    /// The way into the overview is a continuous, tracked, abandonable lift.
    /// The way out was a `DragGesture(minimumDistance: 20)` whose `onChanged`
    /// recorded one boolean and whose `onEnded` either dismissed or did not:
    /// the path was symmetric and the TRACKING was not, and tracking is what
    /// makes a path readable. WWDC 2018 803: *"avoid methods that are only
    /// detected at the end of the gesture"*, and, on why it matters, *"you
    /// actually wouldn't know the difference between a frozen phone, and phone
    /// that's just at the top of the edge of the screen."*
    ///
    /// **Asserted on the probe and not on pixels**, and that is the whole
    /// design of this test. Pixels change mid-pull either way: the grid is a
    /// scroll view at its top, so a downward drag rubberbands the cards down
    /// whether or not anything else is happening, and a screenshot comparison
    /// would have passed on the bounce alone — the same trap the card and
    /// column tests each had to be rewritten to get out of. `pull=` is the
    /// tracked value itself, in hundredths, and nothing but the reverse reveal
    /// can move it.
    ///
    /// The drag is held for two seconds so the probe can be read while the
    /// finger is still down, and it is released from a standstill on purpose:
    /// 38 points is inside the 40 that commits, so this is the ABANDON, and
    /// the assertion after it is that everything went back.
    func testThePullOutOfTheGridTracksTheFingerAndCanBeAbandoned() throws {
        let app = launch(["-shell-40"])
        liftBar(app, by: 320)
        XCTAssertEqual(try state(app)["overview"], 1, "the lift never reached the overview")
        XCTAssertEqual(try state(app)["pull"], 0, "the overview opened part way out of itself")

        // 58 points: twenty of hysteresis the gesture subtracts before it
        // tracks anything, and thirty-eight of travel after it — which is half
        // of `overRun` and two points inside the throw that commits.
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.30))
        let to = from.withOffset(CGVector(dx: 0, dy: 58))

        var midPull: [String: Int]?
        let read = DispatchSemaphore(value: 0)
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + 1.0) {
            midPull = try? self.state(app)
            read.signal()
        }
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 2.0)
        XCTAssertEqual(read.wait(timeout: .now() + 15), .success, "the probe was never read")

        let during = try XCTUnwrap(midPull, "the shell never answered mid-pull")
        XCTAssertGreaterThan(
            during["pull"] ?? 0, 20,
            "the grid was \(during["pull"] ?? -1)% of the way out with a thumb 58 points down "
                + "it — the pull-down moves nothing until the finger lifts")
        XCTAssertEqual(
            during["overview"], 1,
            "the overview left before the finger did")

        // And the release puts it back, because it was abandoned.
        let after = try state(app)
        XCTAssertEqual(after["overview"], 1, "a pull released short of the threshold dismissed")
        XCTAssertEqual(after["pull"], 0, "the page never went back into its cell")
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

    // MARK: - Touch-down feedback

    /// **The control, and it comes first.**
    ///
    /// The assertions below are of the form "these two renders differ", and
    /// such an assertion passes for free if the screen simply does not render
    /// the same way twice. This is the one that fails in that case: an
    /// untouched card, left alone for exactly as long as the press below
    /// lasts, has to come out byte for byte the same at the end of it. The
    /// same argument `GlanceMarkTests` makes about `ImageRenderer` one package
    /// over, made here about a simulator's glass.
    ///
    /// **Two seconds apart and not back to back**, because back to back is
    /// the interval the press test does NOT use. A pair of screenshots taken
    /// in the same breath can agree for reasons that have nothing to do with
    /// the screen holding still — the same frame served twice would do it —
    /// and the claim the press test needs is about a gap of exactly this
    /// length. So the control waits it out.
    func testACardLeftAloneRendersTheSameBytes() throws {
        let app = launch(["-shell-overview", "-shell-4"])
        let card = try firstCard(app)
        let rect = card.frame
        let settled = try settledFingerprint(in: rect)
        XCTAssertNotEqual(settled, "no such rectangle", "the card's frame is off the screenshot")
        Thread.sleep(forTimeInterval: pressHold)
        XCTAssertEqual(
            settled, fingerprint(XCUIScreen.main.screenshot().image, in: rect),
            "a card nobody touched rendered differently \(pressHold) seconds later")
    }

    /// **A card lights UP under a thumb — it does not fade.** The defect, as
    /// an assertion, and the direction is the whole of it.
    ///
    /// The audit this came from expected to find no pressed treatment at all:
    /// `.buttonStyle(.plain)` has historically rendered custom label content
    /// with nothing. Measured on iOS 26 that is not what happens, and it is
    /// worth knowing before writing the assertion — a first draft of this test
    /// asked only whether the pixels CHANGED, and it passed with the style
    /// removed. A plain card does answer a touch, and it answers it by
    /// DIMMING, fill and text together, from `#494E59` to `#42474F`: mean
    /// brightness over the whole card 84.9 at rest and 76.9 under the thumb.
    ///
    /// Which is the thing this app already has a written rule against, one
    /// file over: *"Pressed is one step UP the hierarchy, not a lower opacity:
    /// a key that fades under a finger looks like a key that did not take the
    /// press"* — `TerminalKeyStyle`. So the assertion is not that something
    /// changed, which `.plain` would satisfy; it is that the card got
    /// BRIGHTER, which only a treatment that adds a fill can. `ShellCardStyle`
    /// takes it to `#626771`, twenty-five levels up.
    ///
    /// **Pixels, and not the tap.** A test that only checked that tapping a
    /// card opens its workspace would have passed on either treatment and on
    /// none at all, which is why nobody noticed: the tap always worked. The
    /// only thing that can go red is a comparison of what is on screen.
    ///
    /// The screenshot is taken from another queue while the press is still
    /// being synthesized on this one, because there is no XCTest call that
    /// puts a finger down and returns. Half way into the hold: far enough past
    /// touch-down that the 0.08-second highlight has finished, and far enough
    /// from the release that nothing is easing back out.
    func testACardLightsUpUnderAThumb() throws {
        let app = launch(["-shell-overview", "-shell-4"])
        let card = try firstCard(app)
        let rect = card.frame
        // Belt and braces on a shared simulator: `waitForExistence` says an
        // element is in the tree, not that the navigation bar has finished
        // arriving over it, and a shot of a screen still settling compared
        // with one taken two seconds later measures the arrival rather than
        // the press.
        _ = try settledFingerprint(in: rect)
        let rest = XCUIScreen.main.screenshot().image

        var shot: XCUIScreenshot?
        let taken = DispatchSemaphore(value: 0)
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + pressHold / 2) {
            shot = XCUIScreen.main.screenshot()
            taken.signal()
        }
        card.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: pressHold)
        XCTAssertEqual(taken.wait(timeout: .now() + 10), .success, "no screenshot was taken")
        let pressed = try XCTUnwrap(shot).image

        XCTAssertNotEqual(
            fingerprint(rest, in: rect), fingerprint(pressed, in: rect),
            "the card renders identically with a thumb on it and at rest — a touch on "
                + "it says nothing until it opens")
        // The MEAN over the whole card rather than one sampled point, so the
        // claim does not depend on finding a spot the fixture's own text never
        // reaches. The fill is most of a card either way.
        let atRest = try meanBrightness(rest, in: rect)
        let underThumb = try meanBrightness(pressed, in: rect)
        XCTAssertGreaterThan(
            underThumb, atRest + 8,
            "the card is \(String(format: "%.1f", atRest)) bright at rest and "
                + "\(String(format: "%.1f", underThumb)) under a thumb — a card that fades "
                + "under a finger looks like a card that did not take the press")
    }

    /// **A pinned column highlights the row under the thumb, not the tab you
    /// are already on.** The second half of the touch-down defect, on the
    /// surface the first half does not reach.
    ///
    /// A column held open by a tap answered the next touch with nothing:
    /// `fingerAbove` was written only once a drag had chosen a vertical axis,
    /// and `columnSelection` short-circuited on `columnPinned && lift == 0`
    /// straight to the current tab. So the shell's primary tab switcher kept
    /// its highlight where you already were for the whole press and moved it
    /// at the instant you let go.
    ///
    /// **Measured against a NEIGHBOURING row rather than against the same row
    /// at rest, and that is what makes this test mean anything.** The bar is
    /// `GlassSurface(interactive: true)`, so the platform lights the whole
    /// surface up at the point of contact — about nine levels of brightness,
    /// which is plenty to carry a naive assertion. A first draft compared the
    /// pressed row with itself at rest and passed on that reaction alone while
    /// the highlight sat on the wrong row. Both rows are inside the same piece
    /// of glass and take the same reaction, so the difference BETWEEN them is
    /// the selection fill and nothing else.
    func testAColumnRowLightsUpUnderAThumb() throws {
        let app = launch()
        let bar = app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
        bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertEqual(try state(app)["pinned"], 1, "a tap did not hold the column open")

        // Workspace 0 has three tabs and the current one is 0, which is the
        // row at the TOP of the column — so the row nearest the bar is tab 2,
        // is unlit at rest, and is the one this puts a thumb on. The row above
        // it is tab 1, which is lit in neither picture and is the reference.
        let frame = bar.frame
        func row(_ fromBottom: Int) -> CGRect {
            CGRect(
                x: frame.minX, y: frame.maxY - CGFloat(rowHeight) * CGFloat(fromBottom + 2),
                width: frame.width, height: CGFloat(rowHeight))
        }
        _ = try settledFingerprint(in: row(0))
        let rest = XCUIScreen.main.screenshot().image

        var shot: XCUIScreenshot?
        let taken = DispatchSemaphore(value: 0)
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + pressHold / 2) {
            shot = XCUIScreen.main.screenshot()
            taken.signal()
        }
        app.coordinate(withNormalizedOffset: .zero)
            .withOffset(CGVector(dx: frame.midX, dy: frame.maxY - CGFloat(rowHeight) * 1.5))
            .press(forDuration: pressHold)
        XCTAssertEqual(taken.wait(timeout: .now() + 10), .success, "no screenshot was taken")
        let pressed = try XCTUnwrap(shot).image

        let apart = try meanBrightness(pressed, in: row(0)) - meanBrightness(pressed, in: row(1))
        let atRest = try meanBrightness(rest, in: row(0)) - meanBrightness(rest, in: row(1))
        XCTAssertGreaterThan(
            apart, 8,
            "the row under the thumb stands \(String(format: "%.2f", apart)) above the row "
                + "over it, against \(String(format: "%.2f", atRest)) with nothing touching "
                + "either — a pinned column keeps its highlight on the tab you are already "
                + "on until the finger lifts")
    }

    /// How long a finger stays down, and how long the control waits. One
    /// number, because the two only mean anything as a pair.
    private let pressHold: TimeInterval = 2.0

    /// The card's pixels once the screen has stopped changing.
    ///
    /// Two screenshots in a row that agree, which is the only definition of
    /// "settled" available from outside the app. In practice the first shot
    /// after `waitForExistence` has always been the settled one on this
    /// simulator; this is what makes that a fact the test checks rather than
    /// one it assumes, and it costs a fifth of a second.
    private func settledFingerprint(in rect: CGRect) throws -> String {
        var last = fingerprint(XCUIScreen.main.screenshot().image, in: rect)
        for _ in 0..<25 {
            Thread.sleep(forTimeInterval: 0.2)
            let next = fingerprint(XCUIScreen.main.screenshot().image, in: rect)
            if next == last { return next }
            last = next
        }
        throw XCTSkip("The card never stopped changing with nothing touching it.")
    }

    /// The first workspace card in the grid, which is the one both assertions
    /// above are made about.
    private func firstCard(_ app: XCUIApplication) throws -> XCUIElement {
        let card = app.descendants(matching: .any).matching(identifier: "shell-card-ws-0")
            .firstMatch
        guard card.waitForExistence(timeout: 30) else {
            print(app.debugDescription)
            throw XCTSkip("The grid never drew a card.")
        }
        XCTAssertTrue(card.isHittable, "the card is on screen but not touchable")
        return card
    }

    /// One rectangle of a screenshot, in the app's own POINTS, as RGBA bytes.
    ///
    /// A screenshot is in pixels and a frame is in points, so the scale is
    /// applied here rather than at each call site — getting that wrong reads a
    /// region a third of the way down the screen from the one being asked
    /// about, which is a different picture and a passing test.
    private func pixels(_ shot: UIImage, in rect: CGRect)
        -> (width: Int, height: Int, bytes: [UInt8])?
    {
        guard let cg = shot.cgImage else { return nil }
        let scale = shot.scale
        let x = Int((rect.minX * scale).rounded())
        let y = Int((rect.minY * scale).rounded())
        let width = Int((rect.width * scale).rounded())
        let height = Int((rect.height * scale).rounded())
        guard x >= 0, y >= 0, width > 0, height > 0,
            x + width <= cg.width, y + height <= cg.height,
            let crop = cg.cropping(to: CGRect(x: x, y: y, width: width, height: height))
        else { return nil }
        var buffer = [UInt8](repeating: 0, count: width * height * 4)
        buffer.withUnsafeMutableBytes { raw in
            let context = CGContext(
                data: raw.baseAddress, width: width, height: height, bitsPerComponent: 8,
                bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            context?.draw(crop, in: CGRect(x: 0, y: 0, width: width, height: height))
        }
        return (width, height, buffer)
    }

    /// The pixels of one rectangle of a screenshot, as a short fingerprint.
    ///
    /// **A digest and not the buffer, because of what a failure PRINTS.** A
    /// 168x132 card at scale 3 is most of a megabyte of decimal integers, and
    /// a failure that arrives as an unreadable wall is a failure nobody reads.
    /// FNV-1a rather than `hashValue`, which is seeded per process and would
    /// make the numbers in that message meaningless between runs.
    private func fingerprint(_ shot: UIImage, in rect: CGRect) -> String {
        guard let read = pixels(shot, in: rect) else { return "no such rectangle" }
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in read.bytes {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01b3
        }
        return String(hash, radix: 16)
    }

    /// How bright that rectangle is on average, 0…255.
    ///
    /// The plain mean of the three colour channels and not a weighted
    /// luminance: this card's fill is a near-neutral slate and both treatments
    /// move all three channels together, so a perceptual weighting would be
    /// arithmetic that changes no answer and one more thing to be wrong.
    private func meanBrightness(_ shot: UIImage, in rect: CGRect) throws -> Double {
        let read = try XCTUnwrap(pixels(shot, in: rect), "the rectangle is off the screenshot")
        var total = 0
        for index in stride(from: 0, to: read.bytes.count, by: 4) {
            total += Int(read.bytes[index]) + Int(read.bytes[index + 1])
                + Int(read.bytes[index + 2])
        }
        return Double(total) / Double(read.width * read.height * 3)
    }

    // MARK: - A grid that spans runners

    /// **The grid lists worktrees across all servers, grouped by runner.**
    ///
    /// The owner's ask. What makes it possible without a second live
    /// connection is that the other runners are CACHED — see
    /// `RunnerDirectory`, and `ShellServerGroup` for why N live connections is
    /// worse than N times the cost — so the assertions here are about a grid
    /// that draws three sections, one of which you are connected to and two of
    /// which are memories.
    ///
    /// `-shell-4` so the whole grid fits on one screen: the cached sections
    /// come after the live one, and a ten-workspace fixture puts them below
    /// the fold where `exists` is still true but nothing has been shown.
    func testTheGridGroupsEveryRunnersWorktreesUnderItsOwnHeading() throws {
        let app = launch(["-shell-servers", "-shell-overview", "-shell-4"])
        XCTAssertEqual(try state(app)["overview"], 1, "the harness did not open on the grid")

        for runner in ["this-mac", "eu-runner-1", "gpu-box-2"] {
            let header = app.descendants(matching: .any)
                .matching(identifier: "shell-section-\(runner)").firstMatch
            XCTAssertTrue(
                header.waitForExistence(timeout: 10),
                "no heading for \(runner): \(app.debugDescription)")
        }

        // A card from another runner, which is a card that cannot be swiped
        // to — it is not in the fleet at all.
        XCTAssertTrue(
            app.buttons["shell-elsewhere-spike/watch-sync"].waitForExistence(timeout: 5),
            "the cached runner's worktree is not in the grid")

        // And one that must not be: hiding is honored on the runner the
        // worktree is on, and this grid is not that runner.
        XCTAssertFalse(
            app.buttons["shell-elsewhere-chore/put-away"].exists,
            "a hidden worktree on another runner was drawn anyway")
    }

    /// The other runners' sections are ordered by how recently this app saw
    /// them, so the runner you were on ten minutes ago is not below the one
    /// you last opened in March.
    ///
    /// Asserted on the frames rather than on the order of the accessibility
    /// tree, which is not the order things are drawn in.
    func testTheMostRecentlySeenRunnerComesFirst() throws {
        let app = launch(["-shell-servers", "-shell-overview", "-shell-4"])
        let recent = app.descendants(matching: .any)
            .matching(identifier: "shell-section-eu-runner-1").firstMatch
        let older = app.descendants(matching: .any)
            .matching(identifier: "shell-section-gpu-box-2").firstMatch
        XCTAssertTrue(recent.waitForExistence(timeout: 10))
        XCTAssertTrue(older.waitForExistence(timeout: 10))
        XCTAssertLessThan(
            recent.frame.minY, older.frame.minY,
            "eu-runner-1 was seen 9 minutes ago and gpu-box-2 two hours ago, so eu-runner-1 "
                + "belongs above it — they are at \(recent.frame.minY) and \(older.frame.minY)")
    }

    /// **A hidden worktree is out of the grid until you ask for it.**
    ///
    /// `Workspace.isHidden` has existed in the model the whole time and iOS
    /// had no consumer for it, so a worktree somebody put away on the Mac came
    /// back as an ordinary card on the phone. A filter alone would be the
    /// other bug — hiding is reversible and the way back must not be a
    /// settings screen — so both halves are asserted here.
    func testHiddenWorktreesLeaveTheGridUntilTheSectionIsOpened() throws {
        let app = launch(["-shell-hidden", "-shell-overview", "-shell-4"])
        XCTAssertEqual(try state(app)["overview"], 1)

        // The fixture hides every fifth workspace from index 3, so `ws-3` is
        // the one out of four that goes.
        let hiddenCard = app.buttons["shell-card-ws-3"]
        let shownCard = app.buttons["shell-card-ws-0"]
        XCTAssertTrue(shownCard.waitForExistence(timeout: 10), "the grid never drew")
        XCTAssertFalse(hiddenCard.exists, "a hidden worktree was drawn as an ordinary card")

        let section = app.descendants(matching: .any)
            .matching(identifier: "shell-hidden-section").firstMatch
        XCTAssertTrue(section.waitForExistence(timeout: 5), "no way back from hiding")
        section.tap()
        XCTAssertTrue(
            hiddenCard.waitForExistence(timeout: 5),
            "opening the hidden section did not reveal the worktree in it")
    }

    /// **Tapping another runner's card says what it costs before it costs it.**
    ///
    /// A cross-runner tap is not navigation inside this shell: `RootView` keys
    /// the whole tree `.id(host)`, so it destroys `FleetView`, the track and
    /// every mounted pane. `ShellPaneTrack`'s whole design is that a pane must
    /// never be rebuilt, and the one moment that cannot be kept is the one
    /// moment it has to be said out loud.
    ///
    /// Cancel and not Switch, because the harness has no runner to switch to —
    /// what is being asserted is that the shell is still exactly where it was
    /// after somebody says no.
    func testCrossingToAnotherRunnerAsksFirstAndCancelChangesNothing() throws {
        let app = launch(["-shell-servers", "-shell-overview", "-shell-4"])
        let before = try state(app)

        let card = app.buttons["shell-elsewhere-spike/watch-sync"]
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        card.tap()

        let alert = app.alerts.firstMatch
        XCTAssertTrue(
            alert.waitForExistence(timeout: 5),
            "a cross-runner tap threw the panes away without asking")
        XCTAssertTrue(
            alert.staticTexts["Switch to eu-runner-1?"].exists,
            "the alert does not name the runner it is about: \(alert.debugDescription)")
        // The cost, in the words the thing is called by. Asserted because the
        // wording IS the feature here: an alert that does not say what goes is
        // an alert people learn to dismiss.
        let body = alert.staticTexts.allElementsBoundByIndex.map { $0.label }.joined(separator: " ")
        XCTAssertTrue(
            body.contains("will close"),
            "the alert never says the open panes close: \(body)")
        XCTAssertTrue(
            body.contains("spike/watch-sync"),
            "the alert never says where it would land: \(body)")

        alert.buttons["Cancel"].tap()
        XCTAssertFalse(alert.exists, "Cancel did not dismiss")
        let after = try state(app)
        XCTAssertEqual(after["ws"], before["ws"], "cancelling moved the shell")
        XCTAssertEqual(after["overview"], 1, "cancelling closed the grid")
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

