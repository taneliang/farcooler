import XCTest

/// The key rows' Hide Keyboard key, driven the one way that is known to work.
///
/// Shared because it was written five times as `if dismiss.exists {
/// dismiss.tap() }`, and every copy had the same two holes:
///
/// - **The tap can land nowhere.** The row is in the tree before the keyboard
///   has finished sliding in, and a tap then is synthesized at {-1, -1}. That
///   was measured, not guessed: the keyboard stayed up and the test went on as
///   if it had gone.
/// - **`exists` can be false because the keyboard has not come up YET.** The
///   check passes, nothing is tapped, and the keyboard arrives a moment later.
///
/// So these wait for the key to be TAPPABLE, and fail rather than skip when it
/// never is. A test that needs the keyboard down and quietly skips when it
/// could not put it down is a test that can never go red on that path.
extension XCTestCase {
    /// The terminal's key row's key. See `TerminalKeyRow`.
    static let terminalHideKeyboard = "terminal-hide-keyboard"
    /// The agent composer's, at the end of its control row. See
    /// `AgentComposer` in `AgentView.swift`.
    static let composerHideKeyboard = "composer-hide-keyboard"

    /// Wait for a key row's Hide Keyboard key to be on screen and tappable,
    /// and hand it back. Fails, rather than skipping, when it never is.
    ///
    /// `raising`, when given, is tapped once if the key has not appeared: a
    /// pane raises the keyboard when it appears, and this is the fallback for
    /// a run where it has not yet.
    @discardableResult
    func waitForHideKeyboardKey(
        _ app: XCUIApplication, _ identifier: String = XCTestCase.terminalHideKeyboard,
        raising: XCUIElement? = nil,
        file: StaticString = #filePath, line: UInt = #line
    ) -> XCUIElement {
        let hide = app.buttons[identifier]
        if !hide.waitForExistence(timeout: 10), let raising { raising.tap() }
        XCTAssertTrue(
            hide.waitForExistence(timeout: 10),
            "\(identifier) never appeared, so there was no keyboard to hide",
            file: file, line: line)
        let tappable = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in hide.isHittable }, object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [tappable], timeout: 10), .completed,
            "\(identifier) is in the tree but never became tappable",
            file: file, line: line)
        XCTAssertEqual(hide.label, "Hide Keyboard", file: file, line: line)
        return hide
    }

    /// Put the keyboard away with the key row's own key, and fail unless both
    /// the key row and the keyboard are gone afterwards.
    ///
    /// Asserted on the row as well as on `app.keyboards`: with a hardware
    /// keyboard attached no software keyboard is ever up, so a count of zero
    /// holds before the tap as well as after it. The row is the input
    /// accessory whichever keyboard is attached, so its going is what proves
    /// the tap resigned first responder.
    func hideKeyboard(
        _ app: XCUIApplication, _ identifier: String = XCTestCase.terminalHideKeyboard,
        raising: XCUIElement? = nil,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        let hide = waitForHideKeyboardKey(
            app, identifier, raising: raising, file: file, line: line)
        let softwareKeyboardWasUp = app.keyboards.count > 0
        hide.tap()
        let gone = XCTNSPredicateExpectation(
            predicate: NSPredicate { _, _ in !hide.exists && app.keyboards.count == 0 },
            object: nil)
        XCTAssertEqual(
            XCTWaiter.wait(for: [gone], timeout: 10), .completed,
            "after Hide Keyboard: key row \(hide.exists ? "still up" : "gone"), "
                + "\(app.keyboards.count) keyboard(s) "
                + "(a software keyboard was \(softwareKeyboardWasUp ? "" : "not ")up before)",
            file: file, line: line)
    }
}
