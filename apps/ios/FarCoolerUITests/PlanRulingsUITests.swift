import XCTest

/// Decided For You on the iPhone (ov-304), over the canned runner, whose plan
/// is a real board's: `test/fixtures/plan-rulings-seeded.json` is `farcooler
/// plan --json` from a scratch daemon given four rulings with `plan ruling add`
/// and `set`, so the bytes the phone decodes are the CLI's.
///
/// Only the gestures a unit test cannot make stay (ov-538): a swipe offers Keep
/// and the runner is told, and Reverse asks first, then reaches the
/// orchestrator. Which rulings are open, the Past Decisions fold, what Keep and
/// Keep All do, who is offered the marks and the words are AgentKit's, pinned in
/// `PlanPhoneRulingsTests` and `PlanRulingActionsTests`.
final class PlanRulingsUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-rulings-seeded.json").path
    }

    /// `.firstMatch`: a row and the label inside it can carry one identifier,
    /// and a query that matches both cannot give a frame at all ("Multiple
    /// matching elements found").
    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id].firstMatch
    }

    private func keep(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// Billing's Plan (its screen with no orchestrator, ov-616), on a runner that keeps a plan and,
    /// unless `rulings` is false, rulings.
    private func openPlan(rulings: Bool = true, extra: [String] = [], large: Bool = true) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan", "-phone-plan-file", Self.fixture]
                + (rulings ? ["-phone-rulings"] : []) + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        // Led, Billing is its conversation, and its Plan is the sheet (ov-616).
        if extra.contains("-phone-billing-led") {
            let card = app.buttons["plan-strip"]
            XCTAssertTrue(card.waitForExistence(timeout: 15), "no Now/Next card")
            card.tap()
            let bar = app.navigationBars["Plan"]
            // To the large detent, dragged by its bar and again if the drag
            // was lost: at half height a revealed row can sit above the
            // sheet's top, where a swipe misses it.
            XCTAssertTrue(bar.waitForExistence(timeout: 10), "no Plan sheet")
            guard large else { return app }
            let top = { bar.frame.minY < app.windows.firstMatch.frame.height * 0.2 }
            for _ in 0..<3 where !top() {
                bar.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
                    .press(forDuration: 0.1, thenDragTo: app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02)))
                Thread.sleep(forTimeInterval: 1)
            }
            XCTAssertTrue(top(), "the Plan sheet didn't reach its large detent")
        }
        // Under the tree, which leads the Plan (ov-616).
        XCTAssertTrue(app.reveal(element(app, "plan-landing")), "the plan didn't draw")
        return app
    }

    /// Whether `target` exists and sits on screen, waiting up to `seconds` for
    /// both. Judged from its frame, not `isHittable`: a row that is moving, or
    /// half under the bar, makes `isHittable` fail the test outright with
    /// "Failed to determine hittability ... Activation point invalid" instead
    /// of answering no, and a loaded runner catches it moving every time. The
    /// frame always answers. On screen is a touchable slice of it, at most 44
    /// points, inside what shows the Plan: the window, or the Plan sheet below
    /// its bar, since a row above a half-height sheet's top is in the window
    /// and still can't be touched.
    private func touchable(_ app: XCUIApplication, _ target: XCUIElement, within seconds: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        repeat {
            if target.exists {
                let frame = target.frame
                let shown = frame.intersection(planArea(app))
                if !frame.isEmpty, !shown.isNull, shown.width > 0, shown.height >= min(frame.height, 44) { return true }
            }
            if seconds > 0 { Thread.sleep(forTimeInterval: 0.2) }
        } while Date() < deadline
        return false
    }

    /// Where the Plan shows: the sheet below its bar, else the window.
    private func planArea(_ app: XCUIApplication) -> CGRect {
        let window = app.windows.firstMatch.frame
        let sheet = element(app, "plan-sheet")
        guard sheet.exists else { return window }
        var area = sheet.frame.intersection(window)
        let bar = app.navigationBars["Plan"]
        if bar.exists, bar.frame.maxY > area.minY {
            area = CGRect(x: area.minX, y: bar.frame.maxY, width: area.width, height: max(0, area.maxY - bar.frame.maxY))
        }
        return area
    }

    /// What to swipe to scroll the Plan: the sheet's list, else the app.
    private func scroller(_ app: XCUIApplication) -> XCUIElement {
        let sheet = element(app, "plan-sheet")
        let list = sheet.collectionViews.firstMatch
        return sheet.exists && list.exists ? list : app
    }

    /// Scrolls to `id`, down the list and then back up it: an element that
    /// was passed, or a list that moved when a ruling left it, is found too.
    /// Ends by waiting for the row to settle where it can be touched.
    private func reveal(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        let target = element(app, id)
        for _ in 0..<8 where !touchable(app, target, within: 0.5) { scroller(app).swipeUp() }
        for _ in 0..<16 where !touchable(app, target, within: 0.5) { scroller(app).swipeDown() }
        _ = touchable(app, target, within: 10)
        return target
    }

    private var sentLines: (XCUIApplication) -> [String] {
        { app in
            (self.element(app, "harness-sent").value as? String).map { $0.split(separator: "\n").map(String.init) } ?? []
        }
    }

    /// A swipe offers Keep; keeping moves the ruling from Decided For You into
    /// Past Decisions, and the runner was told.
    func testKeepMovesARulingIntoPastDecisions() {
        let app = openPlan()
        let row = reveal(app, "plan-ruling-R-2")
        row.swipeLeft()
        let keep = app.buttons["Keep"].firstMatch
        XCTAssertTrue(keep.waitForExistence(timeout: 5), "no Keep on the swipe")
        keep.tap()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: element(app, "plan-ruling-R-2"))
        waitForExpectations(timeout: 10)
        let fold = reveal(app, "plan-rulings-past-header")
        XCTAssertTrue(fold.exists)
        fold.tap()
        XCTAssertTrue(reveal(app, "plan-ruling-R-2").label.hasSuffix("Kept"))
        XCTAssertTrue(element(app, "plan-ruling-R-4").exists, "the others are still open")
    }

    /// Reverse asks first, naming the ruling and the reversal, and sends
    /// nothing until it's confirmed; Cancel sends nothing at all. Confirmed,
    /// the request reaches the orchestrator (typed into its terminal here).
    func testReverseConfirmsThenReachesTheOrchestrator() {
        // This swipe is also the guard for `allowsFullSwipe: false` (ov-538): with a full swipe allowed,
        // it would fire Keep, and the Reverse button below would never be there.
        let app = openPlan(extra: ["-phone-billing-led"])
        let row = reveal(app, "plan-ruling-R-2")
        swipeToReverse(app, row)
        app.buttons["Reverse"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Reverse R-2?"].waitForExistence(timeout: 5), "no confirmation naming R-2")
        XCTAssertFalse(sentLines(app).contains { $0.contains("Please reverse") }, "sent before it was confirmed")
        // Cancel where the dialog draws one; a popover has none and goes away
        // on a tap outside it.
        if app.buttons["Cancel"].waitForExistence(timeout: 2) {
            app.buttons["Cancel"].tap()
        } else {
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: 0.12)).tap()
        }
        XCTAssertFalse(sentLines(app).contains { $0.contains("Please reverse") }, "Cancel sent it")
        let again = reveal(app, "plan-ruling-R-2")
        swipeToReverse(app, again)
        app.buttons["Reverse"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Reverse R-2?"].waitForExistence(timeout: 5))
        app.buttons["Reverse"].firstMatch.tap()
        let sent = NSPredicate { _, _ in self.sentLines(app).contains { $0.contains("Please reverse ruling R-2") } }
        expectation(for: sent, evaluatedWith: nil)
        waitForExpectations(timeout: 10)
    }

    /// Reverse or Discuss from the Plan sheet lands in the orchestrator's
    /// conversation (ov-643): the sheet closes onto it, the draft is in its
    /// composer, and no raw worktree pane covers it (it has a Back).
    func testReverseAndDiscussReturnToTheConversation() {
        for action in ["Reverse", "Discuss"] {
            let app = openPlan(extra: ["-phone-billing-led"])
            let row = reveal(app, "plan-ruling-R-2")
            for _ in 0..<3 {
                row.swipeLeft()
                if app.buttons[action].firstMatch.waitForExistence(timeout: 3) { break }
            }
            app.buttons[action].firstMatch.tap()
            if action == "Reverse" {
                XCTAssertTrue(app.staticTexts["Reverse R-2?"].waitForExistence(timeout: 5))
                app.buttons["Reverse"].firstMatch.tap()
            }
            let drafted = NSPredicate { _, _ in self.sentLines(app).contains { $0.hasPrefix("draft billing") && $0.contains("R-2") } }
            expectation(for: drafted, evaluatedWith: nil)
            waitForExpectations(timeout: 10)
            XCTAssertTrue(
                element(app, "workspace-body").waitForExistence(timeout: 10), "\(action): the conversation isn't showing")
            XCTAssertTrue(app.navigationBars["Plan"].waitForNonExistence(timeout: 10), "\(action): the Plan sheet stayed up")
            XCTAssertFalse(app.buttons["worktree-back"].firstMatch.exists, "\(action): a raw worktree pane covers the conversation")
            Thread.sleep(forTimeInterval: 1)
            keep(app, "ruling-\(action.lowercased())-lands-in-chat")
            app.terminate()
        }
    }

    /// At the sheet's medium detent too, a ruling is found where it can be
    /// touched, and its swipe offers Keep: a row above the half-height sheet's
    /// top isn't taken for one on screen.
    func testARulingSwipesAtTheMediumDetent() {
        let app = openPlan(extra: ["-phone-billing-led"], large: false)
        // Half height until the first swipe: the area a row is judged in
        // starts below the window's middle, which is the case this pins.
        let window = app.windows.firstMatch.frame
        XCTAssertGreaterThan(planArea(app).minY, window.midY, "the sheet wasn't at half height: \(planArea(app))")
        let row = reveal(app, "plan-ruling-R-2")
        XCTAssertTrue(touchable(app, row, within: 5), "the ruling never came into the sheet")
        row.swipeLeft()
        XCTAssertTrue(app.buttons["Keep"].firstMatch.waitForExistence(timeout: 5), "no Keep on the swipe")
    }

    /// A row's swipe, again until its Reverse shows: in the Plan sheet the
    /// sheet's own pan can take the first one (ov-616).
    private func swipeToReverse(_ app: XCUIApplication, _ row: XCUIElement) {
        for _ in 0..<3 {
            row.swipeLeft()
            if app.buttons["Reverse"].firstMatch.waitForExistence(timeout: 3) { return }
        }
    }

    /// Light and dark sheets. Opt-in, as PlanUITests' are:
    /// `TEST_RUNNER_FC_CAPTURES=1`.
    func testCaptures() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["FC_CAPTURES"] == "1",
            "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take the sheets")
        for (name, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            let app = openPlan()
            _ = reveal(app, "plan-ruling-R-2")
            keep(app, "capture-rulings-\(name)")
            app.terminateRetrying()
        }
    }
}
