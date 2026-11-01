import XCTest

/// The plan strip, the Plan sheet and the One tree on an iPhone (ov-300),
/// over the canned runner, whose plan is a real board's:
/// `test/fixtures/plan-seeded.json` is the CLI's own output. Billing is led
/// by an orchestrator, so its screen opens on the strip.
///
/// The strip (the Now/Next card) says what needs you and what's moving; a tap
/// peeks the Plan as a sheet at the medium detent (ov-616). The Plan is the
/// plan column (ov-874, `PhonePlanColumnUITests`); under it the loose
/// worktrees are the tree's levels, pushed inside the sheet.
final class PhoneTreeUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-seeded.json").path
    }

    /// The first match: the probe is on the stack and on the sheet both.
    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    private func keep(_ app: XCUIApplication, _ name: String, expecting state: CaptureState) {
        guard requireState(state, for: name) else { return }
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// Billing, led and planned, on its orchestrator.
    private func openBilling(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-billing-led", "-phone-plan", "-phone-plan-file", Self.fixture] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        XCTAssertTrue(app.buttons["plan-strip"].waitForExistence(timeout: 15), "no Now/Next card")
        return app
    }

    /// The Plan sheet, dragged to its full height: the tree's root.
    private func openTree(_ app: XCUIApplication) {
        app.buttons["plan-strip"].tap()
        let bar = app.navigationBars["Plan"]
        XCTAssertTrue(bar.waitForExistence(timeout: 10), "no Plan sheet: \(app.debugDescription)")
        bar.swipeUp()
    }

    /// Tap `id`, scrolled to first: the tree sits under the Plan's header.
    private func tap(_ app: XCUIApplication, _ id: String, _ why: String) {
        let button = app.buttons[id]
        if !button.waitForExistence(timeout: 5) { app.reveal(button) }
        XCTAssertTrue(button.waitForExistence(timeout: 10), "\(why): no \(id) in \(app.debugDescription)")
        button.tap()
    }

    // MARK: The strip and the sheet

    /// **The strip says what needs you, what's moving and what's next**, over
    /// the orchestrator: the plan's two themes with an ask, its first two
    /// tickets in Now and how many more, and its next ticket.
    func testTheStripSaysWhatNeedsYouAndWhatsMoving() {
        let app = openBilling()
        XCTAssertTrue(element(app, "orchestrator-pane").waitForExistence(timeout: 10), "not on the orchestrator")
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 15), "no strip: \(app.debugDescription)")
        // The glance band's two lines (ov-503): Now by landing and section, Next by rank.
        XCTAssertTrue(strip.label.hasPrefix("Orchestrator, idle. Landing "), strip.label)
        XCTAssertTrue(strip.label.contains(" building") && strip.label.contains(" in review"), strip.label)
        XCTAssertTrue(strip.label.contains("Next: Plan view on iOS"), strip.label)
        XCTAssertTrue(strip.label.hasSuffix(" 2 need you."), strip.label)
        // Above the pane, not over it: the pane's top (its mount probe sits
        // at its top edge) starts under the strip.
        let mount = element(app, "orchestrator-mount")
        XCTAssertLessThanOrEqual(strip.frame.maxY, mount.frame.minY + 1, "the strip covers the pane")
        keep(app, "strip", expecting: .shows(strip, "the plan strip above the pane"))
    }

    /// **A tap peeks the Plan at the medium detent, over the pane**: the
    /// orchestrator's state and the plan's sections, and Done puts it away.
    func testTheStripPeeksThePlanAsASheet() {
        let app = openBilling()
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 15), "no strip")
        strip.tap()
        let header = element(app, "plan-facts")
        XCTAssertTrue(header.waitForExistence(timeout: 10), "no sheet: \(app.debugDescription)")
        XCTAssertTrue(element(app, "plan-fact-needsYou").exists, "the sheet doesn't say what needs you")
        XCTAssertTrue(element(app, "plan-needs-you").waitForExistence(timeout: 10), "no Needs You in the sheet")
        // A peek: the medium detent leaves the top of the screen uncovered.
        let window = app.windows.firstMatch.frame
        XCTAssertGreaterThan(app.navigationBars["Plan"].frame.minY, window.height * 0.3, "the sheet opened full height")
        keep(app, "peek", expecting: .when("the Plan sheet at its medium detent") { app.navigationBars["Plan"].frame.minY > window.height * 0.3 })
        app.buttons["plan-sheet-done"].tap()
        XCTAssertFalse(header.waitForExistence(timeout: 3) && header.isHittable, "Done left the sheet up")
        XCTAssertTrue(strip.isHittable, "the strip is gone after the peek")
    }

    /// **A ticket chosen in the sheet opens its task inside it** (ov-616, ov-874), and Back
    /// comes back to the Plan, still in the sheet.
    func testATicketChosenInTheSheetIsPushedInIt() {
        let app = openBilling()
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 15), "no strip")
        strip.tap()
        let ticket = element(app, "plan-ticket-ov-259")
        XCTAssertTrue(app.reveal(ticket), "no landing ticket: \(app.debugDescription)")
        ticket.tap()
        XCTAssertTrue(element(app, "task-screen").waitForExistence(timeout: 10), "the ticket wasn't pushed")
        XCTAssertTrue(element(app, "plan-sheet").exists, "the ticket left the sheet")
        app.tapBack()
        XCTAssertTrue(app.navigationBars["Plan"].waitForExistence(timeout: 10), "Back didn't come back to the Plan")
    }

    // MARK: The tree

    /// **A worktree no card reaches, put away, is under Loose Worktrees ›
    /// Hidden**, and Unhide brings it out of Hidden (review 6).
    func testAHiddenLooseWorktreeIsUnderLooseHidden() {
        let app = openBilling(["-phone-loose-hidden"])
        openTree(app)
        tap(app, "tree-row-Loose Worktrees", "the root")
        tap(app, "tree-row-Hidden", "Loose Worktrees")
        let row = app.buttons["worktree-row-spike"]
        XCTAssertTrue(row.waitForExistence(timeout: 10), "spike isn't under Hidden: \(app.debugDescription)")
        let from = row.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5))
        from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: -120, dy: 0)), withVelocity: .slow, thenHoldForDuration: 0.3)
        let unhide = app.buttons["unhide-spike"]
        XCTAssertTrue(unhide.waitForExistence(timeout: 5), "no Unhide on a hidden loose worktree")
        unhide.tap()
        let sent = element(app, "harness-sent")
        let unhid = NSPredicate { _, _ in (sent.value as? String ?? "").contains("worktree.unhide spike") }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: unhid, object: nil)], timeout: 10), .completed)
        // Out of Hidden: back at Loose Worktrees, it's a row of its own.
        app.tapBack()
        XCTAssertTrue(app.buttons["worktree-row-spike"].waitForExistence(timeout: 10), "spike didn't leave Hidden")
        XCTAssertFalse(app.buttons["tree-row-Hidden"].exists, "Hidden stayed with nothing in it")
    }

    /// **A level the board moved under shows the move once it's on top
    /// again** (ov-791): covered by a pushed screen it keeps the tree it
    /// drew rather than building it on every board move, and Back builds
    /// it again, with nothing more from the runner.
    func testALevelShowsWhatMovedWhileItWasCovered() {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox"])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        for id in ["tree-row-No Theme", "tree-row-bil-9"] {
            XCTAssertTrue(app.reveal(app.buttons[id]) || app.buttons[id].waitForExistence(timeout: 10), "no \(id)")
            app.buttons[id].tap()
        }
        XCTAssertTrue(app.buttons["tree-own"].waitForExistence(timeout: 10), "bil-9's level didn't open")
        XCTAssertFalse(app.buttons["tree-row-bil-77"].exists, "bil-77 is on the board before it was added")
        // The board grows under No Theme while bil-9's level covers it.
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName("com.farcooler.harness.board-grows" as CFString),
            nil, nil, true)
        Thread.sleep(forTimeInterval: 3)
        app.tapBack()
        let grown = app.buttons["tree-row-bil-77"]
        XCTAssertTrue(
            app.reveal(grown) || grown.waitForExistence(timeout: 10),
            "Back showed the tree held while covered, without bil-77: \(app.debugDescription)")
    }

    // MARK: Captures

    /// The strip, the peek and the full sheet, light and dark. Opt-in, as `PlanUITests.testCaptures` is: it flips the
    /// device's appearance and asserts nothing the tests above don't.
    /// `TEST_RUNNER_FC_CAPTURES=1`; `TEST_RUNNER_FC_CAPTURE_TAG` names a run
    /// taken with Increase Contrast or Reduce Transparency set on the device.
    func testCaptures() throws {
        let environment = ProcessInfo.processInfo.environment
        try XCTSkipUnless(environment["FC_CAPTURES"] == "1", "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take the sheets")
        let tag = environment["FC_CAPTURE_TAG"].map { "-\($0)" } ?? ""
        for (name, appearance) in [("light", XCUIDevice.Appearance.light), ("dark", .dark)] {
            XCUIDevice.shared.appearance = appearance
            let app = openBilling()
            let strip = app.buttons["plan-strip"]
            XCTAssertTrue(strip.waitForExistence(timeout: 15))
            keep(app, "capture-strip-\(name)\(tag)", expecting: .shows(strip))
            strip.tap()
            XCTAssertTrue(element(app, "plan-facts").waitForExistence(timeout: 10))
            keep(app, "capture-peek-\(name)\(tag)", expecting: .shows(element(app, "plan-facts")))
            app.navigationBars["Plan"].swipeUp()
            keep(app, "capture-sheet-\(name)\(tag)", expecting: .when("the Plan sheet at its large detent") {
                app.navigationBars["Plan"].frame.minY < app.windows.firstMatch.frame.height * 0.3
            })
            app.terminate()
        }
    }
}
