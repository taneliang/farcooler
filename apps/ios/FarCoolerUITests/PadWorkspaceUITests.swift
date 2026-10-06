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
        // The probe says the layout, then how often an iPad drew a phone's.
        let shown = NSPredicate(format: "value BEGINSWITH %@", expected + " ")
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
        // Not even for the first frame, before the width was known (review 5).
        XCTAssertEqual(element(app, "pad-layout").value as? String, "threeColumns phoneOnPad=0")
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

    /// **The chat survives the phone's layout and back**, crossing 700 points
    /// at run time as a Split View's divider does: columns, then compact (a
    /// window 800 points wide, so only its size class makes it a phone), then
    /// columns. The pane keeps its mount, and the plan column's pick, kept
    /// while it wasn't drawn, is back with the columns.
    func testTheChatAndThePickSurviveCompactWidth() {
        let app = openBilling(["-pad-compact-wide"])
        layout(app, is: "threeColumns", "landscape")
        let mount = element(app, "orchestrator-mount")
        XCTAssertTrue(mount.waitForExistence(timeout: 10), "no mount probe")
        let before = mount.value as? String
        tap(app, "pad-tree-row-Visual language", "the tree")
        canvasTitle(app, is: "Visual language", "a theme picked")
        resize(app)
        layout(app, is: "phone", "compact at 800 points")
        XCTAssertTrue(app.buttons["segment-tree"].waitForExistence(timeout: 10), "no segments at compact width")
        XCTAssertEqual(mount.value as? String, before, "compact width built the chat again")
        resize(app)
        layout(app, is: "threeColumns", "regular again")
        XCTAssertEqual(mount.value as? String, before, "the columns built the chat again")
        canvasTitle(app, is: "Visual language", "the pick, back with the columns")
    }

    /// The stand-in window to compact width, or back (`PadCompactWindow`).
    private func resize(_ app: XCUIApplication) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName("com.farcooler.harness.pad-compact" as CFString),
            nil, nil, true)
    }

    /// **A chat's composer is the chat column's own**, in three columns and
    /// in two: pinned to the column's foot, never the keyboard's accessory,
    /// which is as wide as the window and lay across the tree and the plan.
    /// A draft typed in it survives a turn, and with the keyboard up the
    /// composer rests above it, still inside the column.
    func testTheComposerIsTheChatColumns() {
        let app = openBilling(["-phone-orchestrator-chat"])
        layout(app, is: "threeColumns", "landscape")
        let chat = element(app, "pad-chat")
        // The composer by its own two ends: its field and its Send, found
        // anywhere in the app, so one drawn outside the column is found too.
        // By label: inside the pane, the pane's identifier is laid over
        // theirs (`orchestrator-pane`).
        let send = app.buttons.matching(NSPredicate(format: "label == 'Send'")).firstMatch
        let field = app.textViews.firstMatch
        XCTAssertTrue(send.waitForExistence(timeout: 15), "no composer in the chat: \(app.debugDescription)")
        XCTAssertTrue(field.waitForExistence(timeout: 10), "no field in the composer")
        func within(_ why: String) {
            let column = chat.frame
            let bar = field.frame.union(send.frame)
            XCTAssertGreaterThanOrEqual(bar.minX, column.minX - 1, "\(why): the composer starts left of the chat (\(bar) in \(column))")
            XCTAssertLessThanOrEqual(bar.maxX, column.maxX + 1, "\(why): the composer runs past the chat (\(bar) in \(column))")
            XCTAssertLessThanOrEqual(bar.maxY, column.maxY + 1, "\(why): the composer is below the chat (\(bar) in \(column))")
            let keyboard = app.keyboards.firstMatch
            if keyboard.exists, keyboard.frame.height > 100 {
                // Up: resting on the keyboard, not under it.
                XCTAssertLessThanOrEqual(bar.maxY, keyboard.frame.minY + 1, "\(why): the keyboard covers the composer")
                XCTAssertGreaterThan(bar.maxY, keyboard.frame.minY - 80, "\(why): the composer isn't on the keyboard (\(bar), keyboard \(keyboard.frame))")
            } else {
                XCTAssertGreaterThan(bar.minY, column.midY, "\(why): the composer isn't at the chat's foot (\(bar) in \(column))")
            }
        }
        within("three columns")
        field.tap()
        field.typeText("a draft kept")
        within("typing in three columns")
        XCUIDevice.shared.orientation = .portrait
        layout(app, is: "twoColumns", "portrait")
        XCTAssertTrue(send.waitForExistence(timeout: 10), "no composer in two columns")
        within("two columns")
        XCTAssertEqual(field.value as? String, "a draft kept", "the draft didn't survive the turn")
        keep(app, "composer-two-columns")
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

    /// **Needs You picked in the tree shows in the plan column** (R-25), as
    /// the Mac's canvas shows it, rather than leaving the workspace.
    func testNeedsYouShowsInThePlanColumn() {
        let app = openBilling()
        layout(app, is: "threeColumns", "landscape")
        tap(app, "pad-tree-row-Needs You", "the tree")
        canvasTitle(app, is: "Needs You", "Needs You picked")
        XCTAssertTrue(element(app, "pad-needs-you").waitForExistence(timeout: 10), "no Needs You list")
        XCTAssertTrue(app.navigationBars["Billing"].exists, "Needs You left the workspace")
        XCTAssertTrue(app.buttons["pad-tree-row-Needs You"].isSelected, "its row isn't marked")
    }

    /// **The Board's own Plan view keeps its picks in the column** (review 8):
    /// a lane there shows beside the tree, not over the columns.
    func testTheBoardsPlanViewKeepsItsPicksInTheColumn() {
        let app = openBilling()
        layout(app, is: "threeColumns", "landscape")
        tap(app, "pad-board", "the toolbar")
        canvasTitle(app, is: "Board", "the board chosen")
        let control = app.segmentedControls["plan-switch"]
        XCTAssertTrue(control.waitForExistence(timeout: 15), "no Tasks | Plan: \(app.debugDescription)")
        control.buttons["Plan"].tap()
        let lane = element(app, "plan-lane-plan-phones")
        XCTAssertTrue(lane.waitForExistence(timeout: 10), "no Next Up lane in the board's plan")
        lane.tap()
        canvasTitle(app, is: "plan-phones", "a lane picked in the board's plan")
        XCTAssertTrue(app.navigationBars["Billing"].exists, "the lane was pushed over the columns")
    }

    /// **The keys**: ⇧⌘B shows the board and again the plan, ⌥⌘P goes back to
    /// the plan, and ⌃⌘S, the system's sidebar key, puts the tree's column
    /// away and back in three columns and shows and hides it in two.
    func testTheKeysShowTheBoardThePlanAndTheTree() {
        let app = openBilling()
        layout(app, is: "threeColumns", "landscape")
        XCTAssertTrue(element(app, "pad-tree").waitForExistence(timeout: 10))
        app.typeKey("b", modifierFlags: [.command, .shift])
        canvasTitle(app, is: "Board", "⇧⌘B")
        app.typeKey("p", modifierFlags: [.command, .option])
        canvasTitle(app, is: "Plan", "⌥⌘P")
        app.typeKey("s", modifierFlags: [.command, .control])
        XCTAssertFalse(element(app, "pad-tree").waitForExistence(timeout: 2), "⌃⌘S didn't put the tree's column away")
        app.typeKey("s", modifierFlags: [.command, .control])
        XCTAssertTrue(element(app, "pad-tree").waitForExistence(timeout: 10), "⌃⌘S didn't bring the tree's column back")
        XCUIDevice.shared.orientation = .portrait
        layout(app, is: "twoColumns", "portrait")
        XCTAssertTrue(app.buttons["pad-tree-toggle"].waitForExistence(timeout: 10), "no tree button")
        XCTAssertFalse(element(app, "pad-tree").waitForExistence(timeout: 2), "the tree is up unasked in two columns")
        app.typeKey("s", modifierFlags: [.command, .control])
        XCTAssertTrue(element(app, "pad-tree").waitForExistence(timeout: 10), "⌃⌘S didn't show the tree")
        app.typeKey("s", modifierFlags: [.command, .control])
        XCTAssertFalse(element(app, "pad-tree").waitForExistence(timeout: 2), "⌃⌘S didn't hide the tree")
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

    /// **Cancel puts the sidebar away**, as the scrim does: ⌘., the iPad's
    /// cancel key. (Escape is bound to the same `.cancelAction`, but the
    /// simulator never delivered XCUITest's Escape to the app, so ⌘. is what
    /// a test can press.) The scrim names what it closes.
    func testCancelPutsTheSidebarAway() {
        XCUIDevice.shared.orientation = .portrait
        let app = openBilling()
        layout(app, is: "twoColumns", "portrait")
        tap(app, "pad-tree-toggle", "the toolbar")
        XCTAssertTrue(element(app, "pad-tree").waitForExistence(timeout: 10), "the tree didn't show")
        XCTAssertEqual(app.buttons["pad-tree-scrim"].label, "Close Themes")
        app.typeKey(".", modifierFlags: [.command])
        XCTAssertFalse(element(app, "pad-tree").waitForExistence(timeout: 2), "⌘. left the tree up")
    }

    /// **A new layout puts the sidebar away**: turned to landscape the tree
    /// is a column, and turned back it isn't shown again unasked.
    func testANewLayoutPutsTheSidebarAway() {
        XCUIDevice.shared.orientation = .portrait
        let app = openBilling()
        layout(app, is: "twoColumns", "portrait")
        tap(app, "pad-tree-toggle", "the toolbar")
        XCTAssertTrue(app.buttons["pad-tree-scrim"].waitForExistence(timeout: 10), "the tree didn't show")
        XCUIDevice.shared.orientation = .landscapeLeft
        layout(app, is: "threeColumns", "landscape")
        XCTAssertFalse(app.buttons["pad-tree-scrim"].exists, "the tree is still over the plan in three columns")
        XCUIDevice.shared.orientation = .portrait
        layout(app, is: "twoColumns", "portrait again")
        XCTAssertFalse(element(app, "pad-tree").waitForExistence(timeout: 2), "the tree came back unasked")
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
