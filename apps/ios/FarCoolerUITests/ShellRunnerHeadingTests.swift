import XCTest

/// The overview's runner headings, and a drag inside a runner's section.
///
/// **Why a UI test and not arithmetic.** Which ids a drop sends, to which
/// runner, and what a cached section allows are rules, and they are in
/// `ShellRunnerSectionsTests` where `swift test` runs them. What is left is
/// the part no rule can see: that every heading actually carries its menu,
/// that a press-and-drag on a card reorders it on the OS that has the API and
/// does nothing on the one that does not, and that the drag does not fight the
/// grid's own pull-down — the gesture that closes the overview, which is a
/// downward drag over the same cards.
///
/// No runner and no daemon: `-shell-harness` stands the shell on a canned
/// fleet whose runner keeps an order, and answers a drop in the runner's place
/// (`ShellFleet.reordered`), so this suite cannot skip itself green.
final class ShellRunnerHeadingTests: XCTestCase {
    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-shell-harness", "-shell-overview", "-shell-4"] + extra
        app.launch()
        return app
    }

    /// `ShellGestureTests`'s probe, read the same way.
    private func state(_ app: XCUIApplication) throws -> [String: Int] {
        let probe = app.descendants(matching: .any).matching(identifier: "shell-state").firstMatch
        guard probe.waitForExistence(timeout: 30) else {
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

    /// **Every runner's heading has a menu, and it is that runner's.**
    ///
    /// The runner menu that sat in the toolbar's top-left corner is gone; what
    /// it offered about one runner is on each runner's heading. A connected
    /// runner offers editing it; one this app is not connected to offers
    /// switching to it — the menu's old checkmark row — because a cached
    /// section cannot take anything else.
    func testEveryRunnerHeadingCarriesItsOwnMenu() throws {
        let app = launch(["-shell-servers"])
        XCTAssertEqual(try state(app)["overview"], 1, "the harness did not open on the grid")

        // By runner id, not label: `harness` is labeled this-mac, `runner-eu`
        // eu-runner-1 and `runner-gpu` gpu-box-2. See `ShellHarness.elsewhere`.
        for runner in ["harness", "runner-eu", "runner-gpu"] {
            XCTAssertTrue(
                app.buttons["shell-section-menu-\(runner)"].waitForExistence(timeout: 10),
                "\(runner)'s heading has no menu: \(app.debugDescription)")
        }

        app.buttons["shell-section-menu-harness"].tap()
        XCTAssertTrue(
            app.buttons["Edit Runner…"].waitForExistence(timeout: 5),
            "the connected runner's menu does not offer Edit Runner…")
        XCTAssertFalse(
            app.buttons["Switch to This Runner"].exists,
            "a connected runner was offered a switch to itself")
        // Dismissed by a tap on the large title, not on the grid: a point on
        // the grid is a card, and a card opens. Not on the menu's own button
        // either — on iOS 26 an open menu makes that button unhittable.
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.13)).tap()

        app.buttons["shell-section-menu-runner-eu"].tap()
        XCTAssertTrue(
            app.buttons["Switch to This Runner"].waitForExistence(timeout: 5),
            "a runner this app is not connected to offers no way to switch to it")
    }

    /// **Two runners with one label are two headings, each findable.**
    ///
    /// The identifiers were keyed by label, so two runners that share one gave
    /// two headings the same identifier and a query found whichever it met
    /// first. `-shell-twin-labels` labels `runner-eu` gpu-box-2, the same as
    /// `runner-gpu`, and the assertion is that each id names exactly one
    /// heading and the two are not the same place on screen.
    func testTwoRunnersWithOneLabelHaveTwoHeadings() throws {
        let app = launch(["-shell-servers", "-shell-twin-labels"])
        XCTAssertEqual(try state(app)["overview"], 1, "the harness did not open on the grid")

        var frames: [CGRect] = []
        for runner in ["runner-gpu", "runner-eu"] {
            let headings = app.descendants(matching: .any)
                .matching(identifier: "shell-section-\(runner)")
            XCTAssertTrue(
                headings.firstMatch.waitForExistence(timeout: 10),
                "no heading for \(runner): \(app.debugDescription)")
            XCTAssertEqual(headings.count, 1, "\(runner)'s id names \(headings.count) headings")
            XCTAssertEqual(
                app.buttons.matching(identifier: "shell-section-menu-\(runner)").count, 1,
                "\(runner)'s menu id is not unique")
            frames.append(headings.firstMatch.frame)
        }
        XCTAssertNotEqual(
            frames[0].minY, frames[1].minY,
            "both ids found the same heading, at \(frames[0].minY)")
    }

    /// **A press-and-drag moves a card on iOS 27, and moves nothing on 26.**
    ///
    /// 26 has no `reorderable`, and the rule for it is that nothing on screen
    /// implies a reorder is possible: the same gesture there is the card's
    /// context menu and the card stays exactly where it was.
    ///
    /// Either way the overview is still open afterwards. The drag starts on
    /// the top row with the grid at its top, which is exactly where the
    /// pull-down that closes the overview is armed.
    ///
    /// **Asserted on the ORDER, not on the dragged card's frame.** The first
    /// version compared ws-0's frame before and after and passed for the wrong
    /// reason: the press puts the card's context menu up first, and the lifted
    /// preview reports a frame of its own. Frames captured through the run
    /// showed the menu up and nothing moved. What a reorder changes is which
    /// card is first, so that is what is read — after the menu has had time to
    /// go, from two cards that are both in the grid.
    ///
    /// Coordinates rather than `press(forDuration:thenDragTo:)` on the target
    /// element, for the same reason: once the menu dims the grid the target is
    /// no longer hittable, and the element form did not carry the drag onto it.
    func testDraggingACardMovesItOnlyWhereTheOSCanReorder() throws {
        let app = launch()
        let first = app.buttons["shell-card-ws-0"]
        let second = app.buttons["shell-card-ws-1"]
        let last = app.buttons["shell-card-ws-3"]
        XCTAssertTrue(first.waitForExistence(timeout: 15), "the grid never drew a card")
        XCTAssertTrue(last.waitForExistence(timeout: 5))
        XCTAssertEqual(
            first.frame.minY, second.frame.minY, accuracy: 1,
            "the fixture no longer starts with ws-0 beside ws-1")

        let from = first.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let to = last.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        from.press(
            forDuration: 1.0, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.6)
        // The harness answers a drop after 300 ms, as a runner would after a
        // round trip; the pending order is drawn until then.
        Thread.sleep(forTimeInterval: 1.5)

        if #available(iOS 27, *) {
            XCTAssertGreaterThan(
                first.frame.minY, second.frame.minY + 1,
                "ws-0 was dragged onto ws-3's place and is still in the first row: "
                    + app.debugDescription)
        } else {
            XCTAssertEqual(
                first.frame.minY, second.frame.minY, accuracy: 1,
                "a card moved on an OS with no reorder to keep it")
            XCTAssertLessThan(first.frame.minX, second.frame.minX)
        }
        XCTAssertEqual(
            try state(app)["overview"], 1,
            "a drag on a card in the top row closed the overview")
    }

    /// **A card dragged straight DOWN from the top row does not pull the
    /// overview closed.**
    ///
    /// The pull-down is a simultaneous gesture on the grid, gated on the grid
    /// having been at its top when the drag began — which a drag that lifts a
    /// card from the first row also satisfies. The two must not both happen.
    func testDraggingACardDownDoesNotCloseTheOverview() throws {
        let app = launch()
        let first = app.buttons["shell-card-ws-0"]
        XCTAssertTrue(first.waitForExistence(timeout: 15), "the grid never drew a card")
        let start = first.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let below = start.withOffset(CGVector(dx: 0, dy: 320))
        start.press(
            forDuration: 1.0, thenDragTo: below, withVelocity: .slow,
            thenHoldForDuration: 0.6)
        Thread.sleep(forTimeInterval: 1.5)
        let after = try state(app)
        XCTAssertEqual(after["overview"], 1, "dragging a card down closed the overview")
        XCTAssertEqual(after["pull"] ?? 0, 0, "dragging a card down started the pull-down")
    }
}
