import XCTest

/// The iPad's chat composer across 700 points, and ⌘↩ in it (ov-357).
///
/// The column's composer (regular width) and the keyboard's (compact) are two
/// views, so crossing the width builds the other. They share one model above
/// the switch (`ComposerModel`), so the draft, the picked photo, the caret and
/// the keyboard cross with it. The width is crossed through
/// `PadCompactWindow`'s notice: the window is stood in, the composers in it
/// are the shipping ones, in the same place in the tree.
///
/// Run on an iPad simulator (`fc-lanes-ipad`); on an iPhone every test skips,
/// which `scripts/ios-ui-tests.sh` refuses as a run.
///
/// Each test was seen red with its production line broken; the commit that
/// last changed them names the mutations.
final class ComposerWidthUITests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad, "the column composer is an iPad's")
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

    /// Billing's chat, in columns, in a window that a notice makes compact.
    private func openChat(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness([
            "-phone-empty-inbox", "-phone-billing-led", "-phone-plan", "-phone-plan-file", Self.fixture,
            "-phone-orchestrator-chat", "-pad-compact-wide", "-phone-accept-prompts",
        ] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        layout(app, is: "threeColumns", "landscape")
        return app
    }

    private func layout(_ app: XCUIApplication, is expected: String, _ why: String) {
        let probe = element(app, "pad-layout")
        XCTAssertTrue(probe.waitForExistence(timeout: 10), "no layout probe")
        let shown = NSPredicate(format: "value BEGINSWITH %@", expected + " ")
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: shown, object: probe)], timeout: 10),
            .completed, "\(why): the layout is \(String(describing: probe.value)), not \(expected)")
    }

    /// The screen's, not the app's (the iPad simulator's `app.screenshot()` came
    /// back turned in landscape; see `PadWorkspaceUITests.keep`).
    private func keep(_ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func post(_ name: String) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName(name as CFString), nil, nil, true)
    }

    private var field: (XCUIApplication) -> XCUIElement { { $0.textViews.firstMatch } }

    /// Hide Keyboard is drawn while the model says the field has the keyboard,
    /// so it's there when the focus truly crossed and the old field's resign
    /// on its way out wasn't taken for the reader putting the keyboard away.
    private func assertHideKeyboardShown(_ app: XCUIApplication, _ why: String) {
        let hide = app.buttons.matching(NSPredicate(format: "label == 'Hide Keyboard'")).firstMatch
        let ok = hide.waitForExistence(timeout: 10)
        XCTAssertTrue(ok, "\(why): the composer has the keyboard but not Hide Keyboard")
    }

    private func sendButton(_ app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label == 'Send'")).firstMatch
    }

    private func hasKeyboardFocus(_ element: XCUIElement) -> Bool {
        (element.value(forKey: "hasKeyboardFocus") as? Bool) ?? false
    }

    private func waitFor(_ element: XCUIElement, _ format: String, _ arg: Any, _ why: String) {
        let predicate = NSPredicate(format: format, argumentArray: [arg])
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: predicate, object: element)], timeout: 10),
            .completed, "\(why): the field says \(element.value ?? "nothing")")
    }

    /// What the runner was sent, as the harness records it.
    private func sent(_ app: XCUIApplication) -> String {
        (element(app, "harness-sent").value as? String) ?? ""
    }

    private func waitForSent(_ app: XCUIApplication, _ message: String, _ why: String) {
        let predicate = NSPredicate(format: "value CONTAINS %@", message)
        XCTAssertEqual(
            XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: predicate, object: element(app, "harness-sent"))], timeout: 10),
            .completed, "\(why): the runner was sent \(sent(app).debugDescription)")
    }

    /// ⌘↩ as a hardware key. The one spelling that reaches the app is
    /// `typeKey("\n", .command)` (measured on fc-lanes-ipad with a probe on
    /// `pressesBegan`): `typeKey(.return, ...)` delivers the modifier and never
    /// the Return, so it types nothing at all, and `.enter` is sent as ⌃C.
    /// `typeText("\n")` is a new line.
    private func commandReturn(_ app: XCUIApplication) {
        app.typeKey("\n", modifierFlags: .command)
    }

    /// **⌘↩ sends from the column's composer**: the words, trimmed, and the
    /// picked photo, as the runner received them. A Return before it only
    /// added a line.
    func testCommandReturnSendsTheMessageAndItsPhoto() {
        let app = openChat()
        let field = field(app)
        XCTAssertTrue(field.waitForExistence(timeout: 15), "no composer")
        field.tap()
        app.typeText("ship it")
        waitFor(field, "value == %@", "ship it", "typed")
        app.typeText("\n")
        waitFor(field, "value == %@", "ship it\n", "Return alone")
        app.typeText("now")
        post("com.farcooler.harness.composer-photo")
        XCTAssertTrue(app.buttons.matching(identifier: "composer-photo-remove").firstMatch.waitForExistence(timeout: 10), "no photo")
        XCTAssertEqual(sent(app).contains("prompt billing"), false, "sent before ⌘↩")
        commandReturn(app)
        waitForSent(app, "prompt billing ship it\nnow images=1", "⌘↩ sent the wrong thing")
        waitFor(field, "value == %@", "", "the field kept the draft after the send")
    }

    /// **The Send button's own ⌘↩, alone**: with the keyboard put away the
    /// field isn't in the responder chain, so only the button's shortcut can
    /// answer. Removing `.keyboardShortcut` turns this red.
    func testTheSendButtonsShortcutSendsWithTheKeyboardAway() {
        let app = openChat()
        let field = field(app)
        XCTAssertTrue(field.waitForExistence(timeout: 15), "no composer")
        field.tap()
        app.typeText("from the button")
        let hide = app.buttons.matching(NSPredicate(format: "label == 'Hide Keyboard'")).firstMatch
        XCTAssertTrue(hide.waitForExistence(timeout: 10), "no Hide Keyboard")
        hide.tap()
        waitFor(field, "hasKeyboardFocus == false", false, "the keyboard didn't go")
        commandReturn(app)
        waitForSent(app, "prompt billing from the button images=0", "the button's ⌘↩ didn't send")
    }

    /// **The field's key command, alone**: `-composer-no-send-shortcut` leaves
    /// the Send button without ⌘↩, so only the field's key command answers.
    /// Breaking the key command turns this red.
    func testTheFieldsKeyCommandSendsWithoutTheButtonsShortcut() {
        let app = openChat(["-composer-no-send-shortcut"])
        let field = field(app)
        XCTAssertTrue(field.waitForExistence(timeout: 15), "no composer")
        field.tap()
        app.typeText("from the field")
        commandReturn(app)
        waitForSent(app, "prompt billing from the field images=0", "the field's key command didn't send")
    }

    /// **⌘↩ on a blank draft sends nothing.**
    func testCommandReturnOnABlankDraftSendsNothing() {
        let app = openChat()
        let field = field(app)
        XCTAssertTrue(field.waitForExistence(timeout: 15), "no composer")
        field.tap()
        app.typeText("   ")
        waitFor(field, "value == %@", "   ", "typed")
        commandReturn(app)
        Thread.sleep(forTimeInterval: 2)
        XCTAssertFalse(sent(app).contains("prompt billing"), "a blank draft was sent: \(sent(app).debugDescription)")
        XCTAssertEqual(field.value as? String, "   ", "the blank draft was cleared")
    }

    /// **A pane covered at a fixed width doesn't raise the keyboard when it
    /// comes back.** A shell opened from the tree covers the workspace, and
    /// the chat's composer leaves its window with the keyboard in it. Nothing
    /// replaces it, so it isn't a hand-off to the other width's composer, and
    /// the keyboard the reader had is gone with it.
    func testACoveredPaneComesBackWithoutTheKeyboard() {
        let app = openChat()
        let field = field(app)
        XCTAssertTrue(field.waitForExistence(timeout: 15), "no composer")
        field.tap()
        app.typeText("kept")
        XCTAssertTrue(hasKeyboardFocus(field), "no keyboard to lose")
        // The main checkout, which opens its shell over the columns.
        let tree = element(app, "pad-tree")
        let row = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'pad-tree-row-' AND label CONTAINS[c] 'Main Checkout'")).firstMatch
        for _ in 0..<16 where !row.exists { tree.swipeUp() }
        XCTAssertTrue(row.exists, "no main checkout row: \(app.buttons.allElementsBoundByIndex.map(\.identifier))")
        row.tap()
        let back = app.buttons["BackButton"]
        XCTAssertTrue(back.waitForExistence(timeout: 15), "nothing covered the chat")
        back.tap()
        layout(app, is: "threeColumns", "back at the columns")
        waitFor(app.textViews.firstMatch, "value == %@", "kept", "the draft")
        Thread.sleep(forTimeInterval: 2)
        XCTAssertFalse(hasKeyboardFocus(app.textViews.firstMatch), "the composer raised a keyboard nobody asked for")
        XCTAssertFalse(
            app.buttons.matching(NSPredicate(format: "label == 'Hide Keyboard'")).firstMatch.exists,
            "Hide Keyboard is shown with no keyboard")
    }

    /// **The composer crosses 700 points whole, both ways**: text, one photo,
    /// the caret in the middle of the text, and the keyboard's focus. After
    /// each crossing a character typed with no tap lands at the caret (so the
    /// caret and the focus both crossed), and the photo's strip is there.
    func testTheComposerKeepsItsStateAcrossCompactAndBack() {
        let app = openChat()
        let field = field(app)
        XCTAssertTrue(field.waitForExistence(timeout: 15), "no composer")
        field.tap()
        app.typeText("hello world")
        for _ in 0..<5 { app.typeKey(.leftArrow, modifierFlags: []) }
        post("com.farcooler.harness.composer-photo")
        let photo = app.buttons.matching(identifier: "composer-photo-remove")
        XCTAssertTrue(photo.firstMatch.waitForExistence(timeout: 10), "no photo in the strip: \(app.debugDescription)")
        XCTAssertTrue(hasKeyboardFocus(field), "the column's composer lost the keyboard to the photo")

        assertHideKeyboardShown(app, "column first")
        keep("1-column-before")
        post("com.farcooler.harness.pad-compact")
        layout(app, is: "phone", "compact")
        let docked = app.textViews.firstMatch
        waitFor(docked, "value == %@", "hello world", "the docked composer's draft")
        XCTAssertTrue(photo.firstMatch.waitForExistence(timeout: 10), "the photo didn't cross to the docked composer")
        XCTAssertEqual(photo.count, 1, "the photo was doubled")
        XCTAssertTrue(hasKeyboardFocus(docked), "the docked composer didn't take the keyboard")
        assertHideKeyboardShown(app, "docked")
        keep("2-docked-after")
        app.typeText("X")
        waitFor(docked, "value == %@", "hello Xworld", "the caret didn't cross to the docked composer")

        post("com.farcooler.harness.pad-compact")
        layout(app, is: "threeColumns", "regular again")
        let column = app.textViews.firstMatch
        waitFor(column, "value == %@", "hello Xworld", "the column composer's draft")
        XCTAssertTrue(photo.firstMatch.waitForExistence(timeout: 10), "the photo didn't cross back")
        XCTAssertEqual(photo.count, 1, "the photo was doubled coming back")
        XCTAssertTrue(hasKeyboardFocus(column), "the column composer didn't take the keyboard back")
        assertHideKeyboardShown(app, "column again")
        keep("3-column-again")
        app.typeText("Y")
        waitFor(column, "value == %@", "hello XYworld", "the caret didn't cross back")
    }

    /// **A keyboard put away stays put**: the focus that crosses is the one
    /// the reader left, not a field taken down. With Hide Keyboard pressed,
    /// the other width's composer doesn't raise it.
    func testAPutAwayKeyboardStaysAwayAcrossTheWidth() {
        let app = openChat()
        let field = field(app)
        XCTAssertTrue(field.waitForExistence(timeout: 15), "no composer")
        field.tap()
        app.typeText("draft")
        let hide = app.buttons.matching(NSPredicate(format: "label == 'Hide Keyboard'")).firstMatch
        XCTAssertTrue(hide.waitForExistence(timeout: 10), "no Hide Keyboard: \(app.buttons.allElementsBoundByIndex.map { $0.identifier + "/" + $0.label })")
        hide.tap()
        waitFor(app.textViews.firstMatch, "hasKeyboardFocus == false", false, "the keyboard didn't go")
        post("com.farcooler.harness.pad-compact")
        layout(app, is: "phone", "compact")
        waitFor(app.textViews.firstMatch, "value == %@", "draft", "the docked composer's draft")
        XCTAssertFalse(hasKeyboardFocus(app.textViews.firstMatch), "the docked composer raised a keyboard nobody asked for")
    }
}
