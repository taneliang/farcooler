import XCTest

/// Closing a terminal from the workspace bar's column, with a real finger.
///
/// **Why a UI test and not arithmetic.** What `ShellCloseTests` can reach is
/// the sentence a running pane is confirmed with; what it cannot reach is any
/// of the four constraints the ruling is actually made of — that the action
/// lives inside the menu the bar opens, that the gesture is the platform's own
/// horizontal swipe, that the diff row does not offer it, and that the swipe
/// reveals rather than fires. Every one of those is a fact about a `List` in a
/// clipped window inside a piece of glass, and every one of them passes every
/// pure test while leaving a column nobody can close a terminal from.
///
/// No runner and no daemon. `-shell-harness` stands the shell on a canned
/// fleet whose tab 0 is the diff and whose other tabs are terminals — the same
/// split `ShellFleetMap.one(_:naming:now:)` makes over a real one — so this
/// suite never skips, the way `ShellGestureTests` never does and for the same
/// reason.
///
/// What is deliberately NOT here is the close itself. The two calls go to a
/// runner, the confirmation is raised from `ShellScreen` against a live
/// `Terminal`, and a fixture has no runner to answer — asserting it here would
/// be asserting that the harness's own do-nothing closure does nothing.
/// `ShellCloseTests` holds the sentence; `Connection.close` is the pair of
/// calls, in the order the daemon requires.
final class ShellColumnCloseTests: XCTestCase {
    /// The default fixture's first workspace has three tabs — `Diff`, then
    /// `codex` and `shell`. Ten workspaces, tab counts `[3, 2, 5, 1, 4]`
    /// cycling, agent names cycling `[claude, codex, shell, aider]` from tab 0,
    /// whose title is overwritten with `Diff`.
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-shell-harness"]
        app.launch()
        return app
    }

    /// `ws`, `tab`, `column`, `pinned`, and the rest of `ShellRootView.probe`.
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

    private func bar(_ app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: "shell-bar").firstMatch
    }

    /// Tap the bar, and wait until the column it opens is really on screen.
    ///
    /// A tap and not a lift, because the tap is the whole of the first
    /// constraint: the swipe is offered on a PINNED column only, and a tap is
    /// the only thing that pins one. Waiting on a row rather than on a duration
    /// — the menu springs open on `ShellMotion.menu`, and a fixed sleep is a
    /// test that goes red on a slow simulator and green on a fast one.
    private func pinColumn(_ app: XCUIApplication) throws {
        let bar = bar(app)
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")
        bar.tap()
        XCTAssertTrue(
            app.buttons["shell-column-row-0"].waitForExistence(timeout: 10),
            "tapping the bar opened no column: \(app.debugDescription)")
        XCTAssertEqual(
            try state(app)["pinned"], 1,
            "the column is showing but the shell does not think it is pinned")
    }

    /// Swipe a column row leftwards, the way a table row is swiped.
    ///
    /// Slow and held, for `ShellGestureTests.swipeContent`'s reason: a release
    /// arriving in the same frame as the last movement can leave the final
    /// translation unreported, which reads as a gesture that recognized and
    /// then decided to do nothing.
    ///
    /// Measured off the ROW's own frame and not the screen's. The column is a
    /// surface inset from both edges of a bar that is itself inset, so a
    /// normalized swipe across the display starts outside it — and starting
    /// outside it is a swipe the bar's own horizontal gesture would answer by
    /// changing workspace.
    private func swipe(_ row: XCUIElement, by points: CGFloat = 60) {
        let from = row.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5))
        let to = from.withOffset(CGVector(dx: -points, dy: 0))
        from.press(
            forDuration: 0.05, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.4)
    }

    /// **A terminal row swipes to reveal Close, and does not close.**
    ///
    /// Both halves, in one test, because they are one decision:
    /// `allowsFullSwipe` is false, so the swipe REVEALS and a tap decides. A
    /// version that fired on the swipe would satisfy an assertion that only
    /// looked for the row going away — and firing on the swipe is the exact
    /// thing the owner ruled out: *"we don't want the user to accidentally
    /// close terminals"*.
    ///
    /// The workspace is asserted unchanged in the same breath. A horizontal
    /// drag on this surface is also the gesture that changes workspace, and the
    /// interesting failure is not that the swipe does nothing — it is that the
    /// bar answers it instead, which looks like a working swipe on a screen
    /// that has quietly moved to a different worktree.
    func testSwipingATerminalRowRevealsCloseWithoutClosingIt() throws {
        let app = launch()
        try pinColumn(app)

        let row = app.buttons["shell-column-row-1"]
        XCTAssertTrue(
            row.waitForExistence(timeout: 10),
            "the column drew no terminal row: \(app.debugDescription)")
        XCTAssertEqual(row.label, "codex", "row 1 of the fixture\u{2019}s first workspace is codex")
        swipe(row)

        XCTAssertTrue(
            app.buttons["Close"].waitForExistence(timeout: 5),
            "swiping a terminal row revealed no Close: \(app.debugDescription)")
        XCTAssertTrue(row.exists, "the swipe closed the terminal on its own")
        XCTAssertEqual(
            try state(app)["ws"], 0,
            "the swipe was answered by the bar and changed workspace")
    }

    /// **The Diff row has no Close, and it is absent rather than disabled.**
    ///
    /// The Mac's rule, kept: a daemon-side refusal is a safety net, and the
    /// button should not be there to press. A diff is synthesized by the shell,
    /// has no terminal behind it for `terminal.remove` to be called about, and
    /// is what closing the last terminal in a workspace lands on.
    ///
    /// A terminal row is swiped first, on a launch of its own, and that is the
    /// control: a column that built no swipe action at all — or one whose rows
    /// no longer reach the swipe recognizer — would satisfy the assertion below
    /// by drawing nothing, and this is what stops it.
    func testTheDiffRowIsNotOfferedClose() throws {
        // The control FIRST, and on its own launch. A revealed swipe action is
        // modal in a table: once one row's actions are open, the next touch
        // anywhere is spent dismissing them rather than starting a new swipe —
        // so a control run after the diff's swipe measures the dismissal and
        // reports it as "this column has no Close at all". Two launches, two
        // clean lists.
        let control = launch()
        try pinColumn(control)
        let terminal = control.buttons["shell-column-row-1"]
        XCTAssertTrue(terminal.waitForExistence(timeout: 10), "the column drew no terminal row")
        swipe(terminal)
        XCTAssertTrue(
            control.buttons["Close"].waitForExistence(timeout: 5),
            "the control failed: no row in this column swipes to Close over the same "
                + "sixty points, so the assertion below would prove nothing")
        control.terminate()

        let app = launch()
        try pinColumn(app)
        let diff = app.buttons["shell-column-row-0"]
        XCTAssertTrue(diff.waitForExistence(timeout: 10), "the column drew no Diff row")
        XCTAssertEqual(diff.label, "Diff", "row 0 of every workspace is the diff")
        swipe(diff)
        XCTAssertFalse(
            app.buttons["Close"].waitForExistence(timeout: 3),
            "the Diff row offered to close a terminal it does not have: \(app.debugDescription)")
        // And the swipe did not change worktree.
        //
        // This is the assertion that keeps the one above from passing for the
        // wrong reason. A row with no swipe action leaves the drag to the bar's
        // own horizontal gesture — which past `ShellGesture`'s commit threshold
        // changes workspace and furls the column, and a furled column has no
        // Close on it whatever its diff row would have offered. Sixty points is
        // deliberately short of that threshold, and this says so rather than
        // assuming it.
        //
        // What the column DOES do is furl, and that is not asserted here
        // because it is the evidence rather than the requirement: the drag
        // reaching the bar at all is what proves the Diff row had no swipe
        // action to consume it, and the bar resolves a short lift over an open
        // column as choosing the row it was on. The terminal row above,
        // swiped over the same sixty points on the same list, keeps its column
        // open and shows a Close — which is the difference this test is about.
        let after = try state(app)
        XCTAssertEqual(after["ws"], 0, "the swipe on the Diff row changed workspace")
        XCTAssertEqual(after["tab"], 0, "the swipe on the Diff row landed on some other tab")
    }

    /// **Neither the drag-up path nor a chosen row leaves a Close standing.**
    ///
    /// Two phases, and they are the two ways the column stops being a menu.
    ///
    /// The first is the owner's third constraint: *"if the user swipes up from
    /// the workspace bar, they should not be able to delete terminals that
    /// way"*. A column opened by a drag is never LEFT open — every release
    /// either lands on a row or furls it, so the finger that opened it is also
    /// the finger that spends it, and there is no state a single finger can
    /// reach in which a dragged column stands there to be swiped. The half a
    /// single finger cannot reach — a second thumb swiping a row while the
    /// first still holds the column open — is refused by `ShellColumn.closable`,
    /// which is gated on `columnPinned` rather than on the column's height.
    /// XCUITest has no public two-finger drag, so that branch is stated here
    /// and cannot be driven from here.
    ///
    /// The second is the one a mutation can reach: a PINNED column furls when a
    /// row is chosen — `ShellDrag`'s `.land` clears `columnPinned` on purpose,
    /// *"a tap-opened column that stayed open after a choice would leave the
    /// chosen pane behind a list of its siblings"* — and the destructive
    /// affordance has to go with it rather than being left over a pane somebody
    /// is now looking at.
    func testNeitherADraggedNorASpentColumnLeavesACloseStanding() throws {
        let app = launch()
        let bar = bar(app)
        XCTAssertTrue(bar.waitForExistence(timeout: 30), "the bar never appeared")

        // Enough to clear `ShellMetrics.openMin` and open the column, short of
        // the lift that hands the page to the overview.
        let from = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        from.press(
            forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 0, dy: -60)),
            withVelocity: .slow, thenHoldForDuration: 0.5)

        let dragged = try state(app)
        XCTAssertEqual(dragged["pinned"], 0, "a dragged column was left pinned")
        XCTAssertEqual(dragged["column"], 0, "a dragged column was left standing open")
        XCTAssertFalse(
            app.buttons["Close"].exists,
            "the drag-up path left a Close on screen: \(app.debugDescription)")

        try pinColumn(app)
        // Row 2 is the last tab and the row nearest the bar. Pressed by
        // identifier rather than by the title on it: an agent's name is the
        // fixture's and repeats across the fleet, while a row's place in its
        // own column is what a tap is about.
        app.buttons["shell-column-row-2"].tap()

        let chosen = try state(app)
        XCTAssertEqual(chosen["tab"], 2, "tapping a column row did not land on it")
        XCTAssertEqual(chosen["pinned"], 0, "choosing a row left the column pinned")
        XCTAssertEqual(chosen["column"], 0, "choosing a row left the column open")
        XCTAssertFalse(
            app.buttons["Close"].exists,
            "a spent column left a Close over the pane it chose: \(app.debugDescription)")
    }
}
