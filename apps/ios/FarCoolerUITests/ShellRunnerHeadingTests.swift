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

        for runner in ["this-mac", "eu-runner-1", "gpu-box-2"] {
            XCTAssertTrue(
                app.buttons["shell-section-menu-\(runner)"].waitForExistence(timeout: 10),
                "\(runner)'s heading has no menu: \(app.debugDescription)")
        }

        app.buttons["shell-section-menu-this-mac"].tap()
        XCTAssertTrue(
            app.buttons["Edit Runner…"].waitForExistence(timeout: 5),
            "the connected runner's menu does not offer Edit Runner…")
        XCTAssertFalse(
            app.buttons["Switch to This Runner"].exists,
            "a connected runner was offered a switch to itself")
        // Dismissed by tapping the menu's own button again rather than a point
        // on the grid: a point on the grid is a card, and a card opens.
        app.buttons["shell-section-menu-this-mac"].tap()

        app.buttons["shell-section-menu-eu-runner-1"].tap()
        XCTAssertTrue(
            app.buttons["Switch to This Runner"].waitForExistence(timeout: 5),
            "a runner this app is not connected to offers no way to switch to it")
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
    func testDraggingACardMovesItOnlyWhereTheOSCanReorder() throws {
        let app = launch()
        let first = app.buttons["shell-card-ws-0"]
        let last = app.buttons["shell-card-ws-3"]
        XCTAssertTrue(first.waitForExistence(timeout: 15), "the grid never drew a card")
        XCTAssertTrue(last.waitForExistence(timeout: 5))
        let before = first.frame

        first.press(
            forDuration: 1.0, thenDragTo: last, withVelocity: .slow,
            thenHoldForDuration: 0.6)
        // The harness answers a drop after 300 ms, as a runner would after a
        // round trip; the pending order is drawn until then.
        Thread.sleep(forTimeInterval: 1.5)

        if #available(iOS 27, *) {
            XCTAssertNotEqual(
                first.frame.origin, before.origin,
                "ws-0 was dragged onto ws-3 and did not move: \(app.debugDescription)")
        } else {
            XCTAssertEqual(
                first.frame.origin, before.origin,
                "a card moved on an OS with no reorder to keep it")
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
