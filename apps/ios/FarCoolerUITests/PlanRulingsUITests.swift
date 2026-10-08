import XCTest

/// Decided For You on the iPhone (ov-304), over the canned runner, whose plan
/// is a real board's: `test/fixtures/plan-rulings-seeded.json` is `farcooler
/// plan --json` from a scratch daemon given four rulings with `plan ruling add`
/// and `set`, so the bytes the phone decodes are the CLI's.
///
/// Only the open ones, newest first; the settled one folds into Past
/// Decisions (ov-333). Keep and Keep All mark rulings kept through the runner;
/// nothing on a runner without `board_rulings`.
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

    /// Billing's board with Plan chosen, on a runner that keeps a plan and,
    /// unless `rulings` is false, rulings.
    private func openPlan(rulings: Bool = true, extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-board-reads", "-phone-plan", "-phone-plan-file", Self.fixture]
                + (rulings ? ["-phone-rulings"] : []) + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        app.buttons["segment-board"].tap()
        let control = app.segmentedControls["plan-switch"]
        XCTAssertTrue(control.waitForExistence(timeout: 10), "no Tasks | Plan control")
        control.buttons["Plan"].tap()
        XCTAssertTrue(element(app, "plan-now").waitForExistence(timeout: 10), "the plan didn't draw")
        return app
    }

    /// Whether `target` exists and sits on screen, waiting up to `seconds` for
    /// both. Judged from its frame, not `isHittable`: a row that is moving, or
    /// half under the bar, makes `isHittable` fail the test outright with
    /// "Failed to determine hittability ... Activation point invalid" instead
    /// of answering no, and a loaded runner catches it moving every time. The
    /// frame always answers. On screen is a touchable slice of it, at most 44
    /// points, inside the window.
    private func touchable(_ app: XCUIApplication, _ target: XCUIElement, within seconds: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        repeat {
            if target.exists {
                let frame = target.frame, window = app.windows.firstMatch.frame
                let shown = frame.intersection(window)
                if !frame.isEmpty, !shown.isNull, shown.width > 0, shown.height >= min(frame.height, 44) { return true }
            }
            if seconds > 0 { Thread.sleep(forTimeInterval: 0.2) }
        } while Date() < deadline
        return false
    }

    /// Scrolls to `id`, down the list and then back up it: an element that
    /// was passed, or a list that moved when a ruling left it, is found too.
    /// Ends by waiting for the row to settle where it can be touched.
    private func reveal(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        let target = element(app, id)
        for _ in 0..<8 where !touchable(app, target, within: 0.5) { app.swipeUp() }
        for _ in 0..<16 where !touchable(app, target, within: 0.5) { app.swipeDown() }
        _ = touchable(app, target, within: 10)
        return target
    }

    private var sentLines: (XCUIApplication) -> [String] {
        { app in
            (self.element(app, "harness-sent").value as? String).map { $0.split(separator: "\n").map(String.init) } ?? []
        }
    }

    func testOpenRulingsComeFirstNewestFirstAndTheRestFold() {
        let app = openPlan()
        XCTAssertTrue(reveal(app, "plan-rulings").exists, "no Decided For You")
        let newest = reveal(app, "plan-ruling-R-4")
        XCTAssertTrue(newest.exists)
        let next = element(app, "plan-ruling-R-3")
        XCTAssertTrue(next.exists)
        XCTAssertLessThan(newest.frame.minY, next.frame.minY, "newest first")
        XCTAssertTrue(newest.label.contains("Why:"), "the reason is read: \(newest.label)")
        XCTAssertFalse(element(app, "plan-ruling-R-1").exists, "a kept ruling is folded into Past Decisions")
        let fold = reveal(app, "plan-rulings-past-header")
        XCTAssertTrue(fold.exists, "no Past Decisions")
        XCTAssertEqual(fold.value as? String, "Collapsed")
        fold.tap()
        let settled = reveal(app, "plan-ruling-R-1")
        XCTAssertTrue(settled.exists)
        XCTAssertTrue(settled.label.hasSuffix("Kept"), settled.label)
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

    /// Keep All is in the section's header while there's more than one open,
    /// and keeps every one: Decided For You is gone, and no count is left
    /// behind.
    func testKeepAllKeepsEveryOpenRuling() {
        let app = openPlan()
        let all = reveal(app, "plan-rulings-keep-all")
        XCTAssertTrue(all.exists, "no Keep All")
        all.tap()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: element(app, "plan-rulings"))
        waitForExpectations(timeout: 10)
        let fold = reveal(app, "plan-rulings-past-header")
        XCTAssertTrue(fold.exists)
        fold.tap()
        for short in ["R-4", "R-3", "R-2", "R-1"] {
            XCTAssertTrue(reveal(app, "plan-ruling-\(short)").label.hasSuffix("Kept"), short)
        }
    }

    /// A swipe reveals the three actions and performs none: a full swipe never
    /// keeps a ruling by itself.
    func testASwipeRevealsKeepReverseAndDiscussAndDoesNotKeep() {
        let app = openPlan(extra: ["-phone-billing-led"])
        let row = reveal(app, "plan-ruling-R-2")
        row.swipeLeft()
        for name in ["Keep", "Reverse", "Discuss"] {
            XCTAssertTrue(app.buttons[name].firstMatch.waitForExistence(timeout: 5), "no \(name) on the swipe")
        }
        XCTAssertTrue(element(app, "plan-ruling-R-2").exists, "the swipe kept nothing")
        XCTAssertFalse(sentLines(app).contains { $0.hasPrefix("ruling.keep") }, "\(sentLines(app))")
    }

    /// Reverse asks first, naming the ruling and the reversal, and sends
    /// nothing until it's confirmed; Cancel sends nothing at all. Confirmed,
    /// the request reaches the orchestrator (typed into its terminal here).
    func testReverseConfirmsThenReachesTheOrchestrator() {
        let app = openPlan(extra: ["-phone-billing-led"])
        let row = reveal(app, "plan-ruling-R-2")
        row.swipeLeft()
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
        again.swipeLeft()
        app.buttons["Reverse"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Reverse R-2?"].waitForExistence(timeout: 5))
        app.buttons["Reverse"].firstMatch.tap()
        let sent = NSPredicate { _, _ in self.sentLines(app).contains { $0.contains("Please reverse ruling R-2") } }
        expectation(for: sent, evaluatedWith: nil)
        waitForExpectations(timeout: 10)
    }

    /// With the orchestrator a chat pane, a confirmed Reverse is a sent message.
    func testReverseSendsAMessageToAChatOrchestrator() {
        let app = openPlan(extra: ["-phone-billing-led", "-phone-orchestrator-chat"])
        let row = reveal(app, "plan-ruling-R-2")
        row.swipeLeft()
        app.buttons["Reverse"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Reverse R-2?"].waitForExistence(timeout: 5))
        app.buttons["Reverse"].firstMatch.tap()
        let sent = NSPredicate { _, _ in self.sentLines(app).contains { $0.hasPrefix("prompt billing Please reverse ruling R-2") } }
        expectation(for: sent, evaluatedWith: nil)
        waitForExpectations(timeout: 10)
        XCTAssertTrue(sentLines(app).contains { $0.contains("Reversing it:") }, "the recorded reversal travels: \(sentLines(app))")
    }

    /// Discuss reaches the orchestrator as a quote, with no confirmation.
    func testDiscussQuotesTheRulingToTheOrchestrator() {
        let app = openPlan(extra: ["-phone-billing-led"])
        let row = reveal(app, "plan-ruling-R-2")
        row.swipeLeft()
        app.buttons["Discuss"].firstMatch.tap()
        let sent = NSPredicate { _, _ in self.sentLines(app).contains { $0.contains("About ruling R-2") } }
        expectation(for: sent, evaluatedWith: nil)
        waitForExpectations(timeout: 10)
    }

    /// A phone with a Read grant is offered no marks at all: Keep would flip
    /// and silently put itself back.
    func testAReadGrantOffersNoMarks() {
        let app = openPlan(extra: ["-phone-billing-led", "-phone-read-scope"])
        let row = reveal(app, "plan-ruling-R-2")
        row.swipeLeft()
        XCTAssertFalse(app.buttons["Keep"].firstMatch.waitForExistence(timeout: 2), "Keep on a Read grant")
        XCTAssertFalse(element(app, "plan-rulings-keep-all").exists)
        XCTAssertTrue(element(app, "plan-ruling-R-2-copy").exists, "Copy Reference stays")
    }

    func testCopyReferenceIsStillOnAnOpenRuling() {
        let app = openPlan()
        let copy = reveal(app, "plan-ruling-R-3-copy")
        XCTAssertTrue(copy.exists, "no Copy Reference")
        XCTAssertEqual(copy.label, "Copy Reference")
        copy.tap()
        // No sheet, no editor: the plan is still on screen.
        XCTAssertTrue(element(app, "plan-ruling-R-3").exists)
        XCTAssertFalse(app.textFields.firstMatch.exists || app.textViews.firstMatch.exists, "nothing to edit")
    }

    func testARunnerWithoutBoardRulingsShowsNothing() {
        let app = openPlan(rulings: false)
        for _ in 0..<6 { app.swipeUp() }
        XCTAssertFalse(element(app, "plan-rulings").exists)
        XCTAssertFalse(element(app, "plan-ruling-R-4").exists)
        XCTAssertFalse(element(app, "plan-rulings-past-header").exists)
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
