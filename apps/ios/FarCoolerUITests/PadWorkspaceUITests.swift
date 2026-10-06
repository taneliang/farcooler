import XCTest

/// The iPad's workspace in columns (ov-348), over the canned runner and the
/// plan of `test/fixtures/plan-seeded.json`: the tree, the plan and the
/// orchestrator's chat side by side at regular width, the tree a sidebar on
/// demand where only two fit, and the phone's segments at compact width.
///
/// Run on an iPad simulator (`fc-lanes-ipad`, an iPad Pro 11-inch); on an
/// iPhone every test skips, which `scripts/ios-ui-tests.sh` refuses as a run.
final class PadWorkspaceUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad, "the columns are an iPad's")
        XCUIDevice.shared.orientation = .landscapeLeft
    }

    override func tearDown() {
        XCUIDevice.shared.orientation = .portrait
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-seeded.json").path
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func keep(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// Billing, led and planned, opened from Needs You.
    private func openBilling(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(
            ["-phone-empty-inbox", "-phone-billing-led", "-phone-plan", "-phone-plan-file", Self.fixture] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        XCTAssertTrue(element(app, "pad-layout").waitForExistence(timeout: 10), "no workspace screen")
        return app
    }

    private func layout(_ app: XCUIApplication, is expected: String, _ why: String) {
        let probe = element(app, "pad-layout")
        let shown = NSPredicate(format: "value == %@", expected)
        let met = XCTNSPredicateExpectation(predicate: shown, object: probe)
        XCTAssertEqual(
            XCTWaiter.wait(for: [met], timeout: 10), .completed,
            "\(why): the layout is \(String(describing: probe.value)), not \(expected)")
    }

    /// Whether the chat is on screen: its header in reach, and its pane a
    /// column at least a terminal's width, inside the window.
    private func chatIsShown(_ app: XCUIApplication) -> Bool {
        let header = element(app, "pad-chat-header")
        let pane = element(app, "pad-chat")
        guard header.waitForExistence(timeout: 10), pane.waitForExistence(timeout: 10),
            element(app, "orchestrator-pane").firstMatch.exists
        else { return false }
        let window = app.windows.firstMatch.frame
        let shown = header.isHittable && pane.frame.width >= 340 && window.contains(pane.frame.insetBy(dx: 1, dy: 1))
        if !shown { print("CHAT: header \(header.frame) hittable \(header.isHittable) pane \(pane.frame) window \(window)") }
        return shown
    }

    private func tap(_ app: XCUIApplication, _ id: String, _ why: String) {
        let button = app.buttons[id]
        XCTAssertTrue(button.waitForExistence(timeout: 10), "\(why): no \(id) in \(app.debugDescription)")
        button.tap()
    }

    private func canvasTitle(_ app: XCUIApplication, is title: String, _ why: String) {
        let header = element(app, "pad-canvas-title")
        let met = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", title), object: header)
        XCTAssertEqual(
            XCTWaiter.wait(for: [met], timeout: 10), .completed,
            "\(why): the plan column says \(header.exists ? header.label : "nothing"), not \(title)")
    }

    // MARK: Columns

    /// **Three columns at regular width**: the tree, the plan and the chat,
    /// side by side in that order, and none of the phone's segments.
    func testThreeColumnsAtRegularWidth() {
        let app = openBilling()
        layout(app, is: "threeColumns", "an iPad in landscape")
        let tree = element(app, "pad-tree")
        let canvas = element(app, "pad-canvas")
        let chat = element(app, "pad-chat")
        XCTAssertTrue(tree.waitForExistence(timeout: 10), "no tree: \(app.debugDescription)")
        XCTAssertTrue(canvas.exists, "no plan column")
        XCTAssertTrue(chat.waitForExistence(timeout: 10), "no chat")
        XCTAssertLessThanOrEqual(tree.frame.maxX, canvas.frame.minX + 1, "the tree isn't before the plan")
        XCTAssertLessThanOrEqual(canvas.frame.maxX, chat.frame.minX + 1, "the plan isn't before the chat")
        // The plan's home is the sheet's: what needs you, and Next Up.
        XCTAssertTrue(element(app, "plan-sheet-needs-you").waitForExistence(timeout: 10), "no Needs You in the plan")
        XCTAssertTrue(element(app, "plan-next-up").waitForExistence(timeout: 10), "no Next Up in the plan")
        // The tree is the Mac's outline: its pinned places, and the themes.
        XCTAssertTrue(app.buttons["pad-tree-row-Plan"].exists, "no Plan row")
        XCTAssertTrue(app.buttons["pad-tree-row-Visual language"].exists, "no theme row")
        XCTAssertFalse(app.buttons["segment-tree"].exists, "the phone's segments are up beside the columns")
        XCTAssertFalse(app.buttons["plan-strip"].exists, "the strip is up beside the plan")
        keep(app, "three-columns")
    }

    /// **The chat is always shown, and the same pane in every layout**:
    /// turning the iPad from three columns to two keeps its mount, so its
    /// session isn't built again.
    func testTheChatIsShownAndSurvivesATurn() {
        let app = openBilling()
        layout(app, is: "threeColumns", "landscape")
        XCTAssertTrue(chatIsShown(app), "no chat: \(app.debugDescription)")
        let mount = element(app, "orchestrator-mount")
        XCTAssertTrue(mount.waitForExistence(timeout: 10), "no mount probe")
        let before = mount.value as? String
        XCUIDevice.shared.orientation = .portrait
        layout(app, is: "twoColumns", "an 11-inch iPad in portrait")
        XCTAssertTrue(chatIsShown(app), "the chat went in portrait")
        XCTAssertEqual(mount.value as? String, before, "turning built the chat again")
        XCUIDevice.shared.orientation = .landscapeLeft
        layout(app, is: "threeColumns", "landscape again")
        XCTAssertEqual(mount.value as? String, before, "turning back built the chat again")
    }

    // MARK: Picks

    /// **A pick in the tree shows in the plan column**: a theme's page, then a
    /// task opened under No Theme, with the tree and the chat still beside
    /// it; the header's back goes to the plan.
    func testAPickInTheTreeShowsInThePlanColumn() {
        let app = openBilling()
        layout(app, is: "threeColumns", "landscape")
        canvasTitle(app, is: "Plan", "at first")
        tap(app, "pad-tree-row-Visual language", "the tree")
        canvasTitle(app, is: "Visual language", "a theme picked")
        XCTAssertTrue(app.buttons["pad-tree-row-Visual language"].isSelected, "the theme's row isn't marked")
        // Still a workspace screen: the bar is Billing's, nothing was pushed.
        XCTAssertTrue(app.navigationBars["Billing"].exists, "the pick was pushed")
        XCTAssertTrue(chatIsShown(app), "the chat went")
        keep(app, "theme-picked")

        // Disclosure closes a row in place: the theme's cards go, and No
        // Theme, under it, comes into view.
        XCTAssertTrue(app.buttons["pad-tree-row-ov-220"].exists, "the theme isn't open")
        tap(app, "pad-tree-disclose-Visual language", "the tree")
        XCTAssertFalse(app.buttons["pad-tree-row-ov-220"].waitForExistence(timeout: 2), "the theme didn't close")
        XCTAssertEqual(app.buttons["pad-tree-disclose-Visual language"].value as? String, "Collapsed")
        // bil-9, a card on Billing's board, under No Theme further down.
        let tree = element(app, "pad-tree")
        for _ in 0..<16 where !app.buttons["pad-tree-row-bil-9"].exists {
            if app.buttons["pad-tree-disclose-No Theme"].exists, app.buttons["pad-tree-disclose-No Theme"].value as? String == "Collapsed" {
                app.buttons["pad-tree-disclose-No Theme"].tap()
            } else {
                tree.swipeUp()
            }
        }
        tap(app, "pad-tree-row-bil-9", "No Theme, open")
        let task = element(app, "task-screen")
        XCTAssertTrue(task.waitForExistence(timeout: 10), "the task isn't in the plan column")
        let canvas = element(app, "pad-canvas")
        XCTAssertGreaterThanOrEqual(task.frame.minX, canvas.frame.minX - 1, "the task isn't in the plan column")
        XCTAssertLessThanOrEqual(task.frame.maxX, canvas.frame.maxX + 1, "the task isn't in the plan column")
        XCTAssertTrue(app.navigationBars["Billing"].exists, "the task was pushed")

        tap(app, "pad-canvas-plan", "the plan column")
        canvasTitle(app, is: "Plan", "back to the plan")
    }

    /// **The Board is the toolbar's**, shown in the plan column, and a second
    /// tap brings the plan back.
    func testTheBoardIsTheToolbarsAndShowsInThePlanColumn() {
        let app = openBilling()
        layout(app, is: "threeColumns", "landscape")
        tap(app, "pad-board", "the toolbar")
        canvasTitle(app, is: "Board", "the board chosen")
        tap(app, "pad-board", "the toolbar")
        canvasTitle(app, is: "Plan", "the board put away")
    }

    // MARK: Two columns

    /// **Two columns show the tree on demand**: in portrait the plan and the
    /// chat; the toolbar's Themes shows the tree over the plan, and a pick
    /// both shows itself and puts the tree away.
    func testTwoColumnsShowTheTreeOnDemand() {
        XCUIDevice.shared.orientation = .portrait
        let app = openBilling()
        layout(app, is: "twoColumns", "an 11-inch iPad in portrait")
        XCTAssertTrue(element(app, "pad-canvas").waitForExistence(timeout: 10), "no plan column")
        XCTAssertTrue(chatIsShown(app), "no chat")
        XCTAssertFalse(element(app, "pad-tree").exists, "the tree is up unasked")
        tap(app, "pad-tree-toggle", "the toolbar")
        XCTAssertTrue(element(app, "pad-tree").waitForExistence(timeout: 10), "the tree didn't show")
        keep(app, "two-columns-tree")
        tap(app, "pad-tree-row-Visual language", "the tree")
        canvasTitle(app, is: "Visual language", "a theme picked")
        XCTAssertFalse(element(app, "pad-tree").waitForExistence(timeout: 2), "the tree stayed up after a pick")
        // The scrim puts it away too.
        tap(app, "pad-tree-toggle", "the toolbar")
        XCTAssertTrue(element(app, "pad-tree").waitForExistence(timeout: 10), "the tree didn't show again")
        let scrim = app.buttons["pad-tree-scrim"]
        XCTAssertTrue(scrim.exists, "no scrim")
        scrim.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).tap()
        XCTAssertFalse(element(app, "pad-tree").waitForExistence(timeout: 2), "the scrim left the tree up")
    }

    // MARK: Compact

    /// **Compact width is the phone's screen**: a one-third Split View gets
    /// the segments and the strip, and no columns.
    func testCompactWidthIsThePhonesScreen() {
        let app = openBilling(["-pad-compact"])
        layout(app, is: "phone", "a compact window")
        XCTAssertTrue(app.buttons["segment-tree"].waitForExistence(timeout: 10), "no segments at compact width")
        XCTAssertTrue(app.buttons["plan-strip"].waitForExistence(timeout: 15), "no strip at compact width")
        XCTAssertFalse(element(app, "pad-tree").exists, "a tree column at compact width")
        XCTAssertFalse(element(app, "pad-canvas").exists, "a plan column at compact width")
        keep(app, "compact")
    }

    // MARK: Captures

    /// Three columns in landscape, with a theme picked; two in portrait,
    /// with the tree shown on demand; and the compact fallback. Opt-in, as
    /// `PhoneTreeUITests.testCaptures` is: `TEST_RUNNER_FC_CAPTURES=1`, and
    /// `TEST_RUNNER_FC_CAPTURE_TAG` names the run's appearance. Set it on the
    /// device first (`xcrun simctl ui <device> appearance dark`, and
    /// `increase_contrast enabled`): `XCUIDevice.appearance` left an iPad
    /// simulator on iOS 27 light.
    func testCaptures() throws {
        let environment = ProcessInfo.processInfo.environment
        try XCTSkipUnless(environment["FC_CAPTURES"] == "1", "capture-only; set TEST_RUNNER_FC_CAPTURES=1 to take the sheets")
        let tag = environment["FC_CAPTURE_TAG"].map { "-\($0)" } ?? ""
        XCUIDevice.shared.orientation = .landscapeLeft
        var app = openBilling(["-phone-orchestrator-chat"])
        layout(app, is: "threeColumns", "landscape")
        XCTAssertTrue(element(app, "plan-next-up").waitForExistence(timeout: 10))
        keep(app, "capture-landscape\(tag)")
        tap(app, "pad-tree-row-Visual language", "the tree")
        canvasTitle(app, is: "Visual language", "a theme picked")
        keep(app, "capture-landscape-theme\(tag)")
        XCUIDevice.shared.orientation = .portrait
        layout(app, is: "twoColumns", "portrait")
        keep(app, "capture-portrait\(tag)")
        tap(app, "pad-tree-toggle", "the toolbar")
        XCTAssertTrue(element(app, "pad-tree").waitForExistence(timeout: 10))
        keep(app, "capture-portrait-tree\(tag)")
        app.terminate()
        app = openBilling(["-phone-orchestrator-chat", "-pad-compact"])
        layout(app, is: "phone", "compact")
        XCTAssertTrue(app.buttons["plan-strip"].waitForExistence(timeout: 15))
        keep(app, "capture-compact\(tag)")
        app.terminate()
    }
}
