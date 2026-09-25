import XCTest

/// The agent composer's Hide Keyboard key.
///
/// The ruling on the bar behind the keyboard keeps the bar where it is, so
/// putting the keyboard away is the way back to it, from an agent pane as much
/// as from a terminal. The terminal's key row had the key; the composer did
/// not, and a person typing to an agent had only the system's own dismissal,
/// which nothing on screen points to.
///
/// Stands on `-agent-layout-harness`: the real `AgentView` over a canned
/// transcript, so this needs no runner and cannot skip itself green.
final class ComposerKeyboardTests: XCTestCase {
    func testTheComposersHideKeyboardKeyPutsTheKeyboardAway() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain"]
        app.launch()

        let field = app.textViews.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 30), "the agent pane drew no composer")
        // Not there before anybody types: with the keyboard down there is
        // nothing for it to do.
        XCTAssertFalse(
            app.buttons[Self.composerHideKeyboard].exists,
            "Hide Keyboard is offered with no keyboard up")

        hideKeyboard(app, Self.composerHideKeyboard, raising: field)

        // Still the composer, still docked: only the keyboard went.
        XCTAssertTrue(field.exists, "hiding the keyboard took the composer with it")
    }
}
