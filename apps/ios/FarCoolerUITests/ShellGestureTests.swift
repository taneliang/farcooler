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
    /// Workspace 0 has three tabs, so the column runs out at 102 and the
    /// overview arrives 76 further up. 320 is well past both, and past is the
    /// only thing being asserted — the exact arrival point is
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
        XCTAssertEqual(opened["column"], 102, "three rows of 34 were not showing")

        // The bar element has grown by the column's height, so its centre is
        // no longer over the bar. Aim at the bottom of it, which is.
        bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9)).tap()
        let closed = try state(app)
        XCTAssertEqual(closed["pinned"], 0, "the second tap toggled the wrong way")
        XCTAssertEqual(closed["column"], 0)
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
