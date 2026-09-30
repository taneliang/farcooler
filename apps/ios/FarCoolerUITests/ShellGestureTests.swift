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
/// No runner and no daemon — `-shell-harness` stands the shell on one canned
/// worktree, so unlike `TerminalScrollTests` this suite never skips. The shell
/// is the one the phone mounts: over one worktree, with the column its bar
/// opens and nowhere to lift to.
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

    /// The fixture: one worktree with three tabs — Changes, `codex` and
    /// `shell` — which is what every number below is counted against.
    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-shell-harness"] + extra
        app.launch()
        return app
    }

    /// `ws`, `tab`, `worktrees`, `tabs`, `column`, `pinned`.
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
        // An explicit velocity, not `.fast`, and the margin is the point.
        //
        // `.fast` is what a synthesized flick negotiates, not what it delivers:
        // the same `.fast` drag measured 284 points per second on one run of
        // this simulator and 803 on the next, and 508 in another suite where
        // 12000 produced 4453. Against an escape threshold of 136 that looks
        // like room until the whole suite is running and the slow reading is
        // the likely one — this test failed exactly once in a full run and
        // passed four times in isolation, which is what a three-times spread
        // around a threshold looks like.
        //
        // Naming a large velocity widens the margin instead of weakening the
        // assertion: the thing under test is still "a flick escapes and a held
        // lift does not", and the held control two lines up is what keeps this
        // honest — it uses the same 140 points and must still stay.
        from.press(
            forDuration: 0.05, thenDragTo: to,
            withVelocity: XCUIGestureVelocity(rawValue: 12000), thenHoldForDuration: 0)
    }

    /// A swipe on the content walks the worktree's tabs, and the opposite swipe
    /// walks back to exactly where it started.
    ///
    /// The round trip is the assertion that matters. A commit that re-seats on
    /// the wrong index, or one whose silent transaction never runs and leaves
    /// the track parked a page off center, both show up as a return journey
    /// that does not land where it left.
    func testASwipeOnTheContentChangesTabAndComesBack() throws {
        let app = launch()
        let start = try state(app)
        XCTAssertEqual(start["ws"], 0)
        XCTAssertEqual(start["tab"], 0)
        XCTAssertEqual(start["worktrees"], 1)
        XCTAssertEqual(start["tabs"], 3, "the canned worktree has three tabs")

        swipeContent(app, toward: -1)
        let forward = try state(app)
        XCTAssertEqual(forward["ws"], 0, "a step within a worktree does not change worktree")
        XCTAssertEqual(forward["tab"], 1, "the swipe did not commit")

        swipeContent(app, toward: 1)
        let back = try state(app)
        XCTAssertEqual(back["ws"], 0)
        XCTAssertEqual(back["tab"], 0, "the return swipe did not land where it started")
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

    /// **A deliberate lift lands on the row under the finger, and a flick up
    /// from the same place lands on nothing.** The owner's complaint, and the
    /// whole of the momentum projection on one screen.
    ///
    /// The talk's PIP example reproduced as the *before* case — *"the issue
    /// here is that we're only looking at position, we're completely ignoring
    /// the momentum"*. A flick is how anybody who has used a task switcher
    /// asks to leave, so a release with that much throw is not a choice of the
    /// row the thumb happened to be passing. This shell has nowhere to fly to
    /// (the stack under it is the way out), so the flick is put back down: the
    /// tab stays where the last deliberate lift left it.
    ///
    /// **The distances are chosen for what they prove, and they differ.** 60
    /// points pins the ROW, because that is where the two mappings disagree:
    /// the bar's center sits 22 points below its own top edge, so a 60-point
    /// lift puts the fingertip 38 points up the column — inside the row
    /// nearest the bar, which is the LAST tab. The delta mapping this replaced
    /// read the 60 rather than the 38 and answered tab 1, a whole row above
    /// the thumb, and the further down the bar a drag began the worse it got.
    /// 140 pins the ESCAPE, because it leaves the fingertip 118 points up —
    /// squarely on the TOP row, with the column's last 14 points still ahead
    /// of it. A release there is a release from over a menu item by any
    /// reading.
    ///
    /// 140 rather than 60 for the flick because a synthesized flick's velocity
    /// is not repeatable: the same `.fast` drag measured 284 points per second
    /// on one run of this simulator and 803 on the next. From 140 the escape
    /// needs 136, so the slower of those two still clears it twice over; from
    /// 60 it needs 297 and the test would be a coin toss on the machine rather
    /// than a statement about the app.
    func testAFlickUpFromOverAMenuRowChoosesNoRow() throws {
        let app = launch()
        XCTAssertEqual(try state(app)["tabs"], 3, "the canned worktree has three tabs")
        XCTAssertEqual(try state(app)["tab"], 0)

        // The row, off the finger's position rather than off its travel.
        liftBar(app, by: 60)
        let landed = try state(app)
        XCTAssertEqual(
            landed["tab"], 2,
            "the row under the finger is the one nearest the bar, which is the last tab")

        // The escape, and its negative control first: the same 140 points,
        // held still before the finger leaves, projects nowhere.
        liftBar(app, by: 140)
        let held = try state(app)
        XCTAssertEqual(held["tab"], 0, "a lift that stopped before letting go chose the top row, where the finger was")

        // Then the flick from a different tab, so a release that landed on a
        // row would have moved it.
        swipeContent(app, toward: -1)
        XCTAssertEqual(try state(app)["tab"], 1, "could not get to the second tab")
        flickBar(app, by: 140)
        let flicked = try state(app)
        XCTAssertEqual(flicked["tab"], 1, "a flick chose the row it passed over")
        XCTAssertEqual(flicked["column"], 0, "the column stayed open after the flick")
        XCTAssertEqual(flicked["pinned"], 0, "a flick pinned the column")
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
            opened["column"], 3 * rowHeight, "worktree 0's three rows were not showing")

        // The bar element has grown by the column's height, so its center is
        // no longer over the bar. Aim at the bottom of it, which is.
        bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9)).tap()
        let closed = try state(app)
        XCTAssertEqual(closed["pinned"], 0, "the second tap toggled the wrong way")
        XCTAssertEqual(closed["column"], 0)
    }

    /// A lift far past the last row puts the page back, on the same worktree
    /// and the same tab.
    ///
    /// Past the column there is no row left to choose and, over one worktree,
    /// nowhere to fly to: the page follows the finger up and falls back on
    /// release, costing nothing.
    func testALongLiftPutsThePageBack() throws {
        let app = launch()
        swipeContent(app, toward: -1)
        XCTAssertEqual(try state(app)["tab"], 1, "could not get to the second tab")

        liftBar(app, by: 320)
        let after = try state(app)
        XCTAssertEqual(after["ws"], 0, "a straight lift changed worktree")
        XCTAssertEqual(after["tab"], 1, "a lift past the last row chose a tab")
        XCTAssertEqual(after["column"], 0, "the column stayed open after the finger left")
        XCTAssertEqual(after["pinned"], 0, "a long lift pinned the column")
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
    /// **The same two numbers, swapped.** 52 across against 80 up is a tab
    /// chosen off the column; 80 across against 52 up leans the other way, and
    /// over one worktree there is no neighbor for it to change to. Both are
    /// well inside the 19° band `redirect` would hold a gesture through if
    /// either of them ever got there — they are 57° and 33° — and both are
    /// unambiguous outcomes rather than two flavours of nothing: the first
    /// changes tab, the second changes nothing.
    ///
    /// 52 across is also deliberately SHORT of the 70-point commit, so a
    /// gesture that leaned the wrong way would spring back and change no tab,
    /// and 80 up puts the fingertip 58 points above the bar row — inside the
    /// second row from the bar, which on a three-tab column is tab 1.
    func testADiagonalDragOnTheBarResolvesByWhichWayItLeans() throws {
        let app = launch()
        XCTAssertEqual(try state(app)["tabs"], 3, "the canned worktree has three tabs")
        XCTAssertEqual(try state(app)["ws"], 0)
        XCTAssertEqual(try state(app)["tab"], 0)

        // Leaning vertical: 52 across, 80 up.
        dragBar(app, by: CGVector(dx: -52, dy: -80))
        let lifted = try state(app)
        XCTAssertEqual(lifted["ws"], 0, "a drag that leans vertical changed worktree")
        XCTAssertEqual(
            lifted["tab"], 1,
            "the fingertip was over the second row from the bar, which is tab 1")

        // Leaning horizontal: the same two numbers the other way round.
        dragBar(app, by: CGVector(dx: -80, dy: -52))
        let sideways = try state(app)
        XCTAssertEqual(sideways["ws"], 0, "a drag that leans horizontal left the worktree")
        XCTAssertEqual(sideways["tab"], 1, "a drag that leans horizontal chose a tab")
        XCTAssertEqual(sideways["column"], 0, "the column stayed open after the finger left")
    }

    /// One straight drag from the bar's center, held before release so it
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
    /// the middle slot and the already-mounted neighbor along with it. Every
    /// commit rebuilt every pane. With text placeholders that is invisible,
    /// which is exactly why it needs a test rather than a look.
    ///
    /// Both directions of the assertion matter and they fail differently. The
    /// pane swiped ONTO was mounted as a neighbor before the swipe: if it is
    /// rebuilt, the thing you watched slide in is not the thing you landed on,
    /// and a real terminal would renegotiate with tmux at the instant of
    /// arrival. The pane swiped AWAY FROM has the half-typed message.
    func testCommittingASwipeRebuildsNothing() throws {
        let app = launch()
        let here = try pane(app, "ws-0-tab-0")["born"]
        let neighbor = try pane(app, "ws-0-tab-1")["born"]
        XCTAssertNotNil(here)
        XCTAssertNotNil(neighbor)
        XCTAssertNotEqual(here, neighbor, "two panes reported one identity")

        swipeContent(app, toward: -1)
        XCTAssertEqual(try state(app)["tab"], 1, "the swipe did not commit")
        XCTAssertEqual(
            try pane(app, "ws-0-tab-1")["born"], neighbor,
            "the pane swiped ONTO was rebuilt by the commit")
        XCTAssertEqual(
            try pane(app, "ws-0-tab-0")["born"], here,
            "the pane swiped AWAY FROM was rebuilt by the commit")

        swipeContent(app, toward: 1)
        XCTAssertEqual(try state(app)["tab"], 0)
        XCTAssertEqual(try pane(app, "ws-0-tab-0")["born"], here, "the return swipe rebuilt a pane")
        XCTAssertEqual(try pane(app, "ws-0-tab-1")["born"], neighbor)
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
        let center = CGVector(
            dx: frame.midX,
            dy: frame.maxY - CGFloat(rowHeight) * (1.5 + CGFloat(row)))
        app.coordinate(withNormalizedOffset: .zero).withOffset(center).tap()
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
        XCTAssertEqual(after["ws"], 0, "a drag down the content changed worktree")
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

    /// **A scroll that leans sideways is still the pane's, not the shell's.**
    ///
    /// The rule this pins is `ShellGesture.contentAxis`, and the report it
    /// comes from is the owner's: *"when scrolling vertically on a terminal, I
    /// often accidentally trigger the horizontal pan gesture and end up
    /// swiping to a different terminal."* The content decides its axis ONCE,
    /// from the first sample past `ShellMetrics.axisLock`, and that sample is
    /// six to a few dozen points long — which is short enough that the roll a
    /// thumb makes as it lands is most of it. A bare `abs(dx) > abs(dy)` gives
    /// the whole gesture away on that sample the instant the sideways half
    /// wins by one point, and there is no way back: redirection is the bar's
    /// and deliberately not the content's.
    ///
    /// **Forty degrees off horizontal, which is fifty off vertical.** Under
    /// the bare comparison this drag is horizontal — 200 across beats 168 up —
    /// and 200 points is nearly three times the seventy that commits, so it
    /// turned the page. It has to be a `-shell-scroll` pane rather than the
    /// default one because both halves are being asserted: the shell stood
    /// down, AND somebody else took the gesture. A shell that merely refused
    /// everything would pass the first half and be a worse bug.
    ///
    /// The straight-line control is `testADiagonalDragOnTheContentStillTurns`
    /// below, and it is what stops this becoming "the page never turns".
    func testAScrollThatLeansSidewaysStaysWithThePane() throws {
        let app = launch(["-shell-scroll"])
        XCTAssertEqual(try state(app)["tab"], 0)
        let before = try XCTUnwrap(paneOffset(app), "the pane never reported a scroll offset")

        // 200 across, 168 up: `tan 40°`, measured in points rather than in
        // normalized offsets so the angle is the angle whatever the device is.
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.75, dy: 0.72))
        from.press(
            forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -200, dy: -168)),
            withVelocity: .slow, thenHoldForDuration: 0.4)

        let after = try state(app)
        XCTAssertEqual(
            after["tab"], 0,
            """
            a drag 40° off horizontal turned the page. The axis was decided from \
            \(after["lockx"] ?? 0) across against \(after["locky"] ?? 0) up — one sample, \
            and final.
            """)
        XCTAssertGreaterThan(
            try XCTUnwrap(paneOffset(app)), before,
            "the shell stood down but nothing else took the drag: the pane never scrolled")
    }

    /// **And a swipe that means it still turns the page.**
    ///
    /// The other wall, and the cost of the one above stated as a test: the
    /// content is called horizontal only within 35.5° of horizontal, so what
    /// this fixture has to show is that a real sideways swipe clears it with
    /// room. Twenty degrees off horizontal — 240 across against 87 up — which
    /// is a swipe nobody would describe as a scroll.
    ///
    /// Both of these run over `-shell-scroll`, so between them they say the
    /// arbitration moved a boundary rather than picking a side.
    func testADiagonalDragOnTheContentStillTurns() throws {
        let app = launch(["-shell-scroll"])
        let start = try state(app)
        XCTAssertEqual(start["tab"], 0)
        XCTAssertGreaterThan(start["tabs"] ?? 0, 1, "this worktree has nowhere to turn to")

        // 240 across, 87 up: `tan 20°`.
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.80, dy: 0.60))
        from.press(
            forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -240, dy: -87)),
            withVelocity: .slow, thenHoldForDuration: 0.4)

        let after = try state(app)
        XCTAssertEqual(
            after["tab"], 1,
            """
            a swipe 20° off horizontal did not turn the page. The axis was decided from \
            \(after["lockx"] ?? 0) across against \(after["locky"] ?? 0) up.
            """)
    }

    /// Where the visible pane's own scroll view has got to.
    ///
    /// `ShellPaneScrollTests.paneOffset`, which reads the same probe for the
    /// same reason: a scroll offset is a number with nothing on screen that
    /// says it, and "the shell ate the scroll" and "there was nothing to
    /// scroll" are indistinguishable in a screenshot.
    private func paneOffset(_ app: XCUIApplication) -> Int? {
        let probes = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH %@", "shell-pane-"))
        for i in 0..<probes.count {
            let value = (probes.element(boundBy: i).value as? String) ?? ""
            guard value.contains("visible=1") else { continue }
            return value.split(separator: " ")
                .first { $0.hasPrefix("offset=") }
                .flatMap { Int($0.split(separator: "=")[1]) }
        }
        return nil
    }

    // MARK: - Touch-down feedback

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
    /// **Measured against a NEIGHBORING row rather than against the same row
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

        // Worktree 0 has three tabs and the current one is 0, which is the
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
    /// The plain mean of the three color channels and not a weighted
    /// luminance: a row's fill is a near-neutral slate and both treatments
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
}
