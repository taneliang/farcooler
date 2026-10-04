import XCTest

/// A chat whose agent died mid-conversation says so, and offers Restart
/// (ov-174).
///
/// The failure used to be drawn only over an EMPTY transcript, saying the
/// agent "couldn't start", so an agent that died after its first reply left
/// nothing on screen but a turn that stopped. The harness's `-stopped` stands
/// the canned conversation on a pane whose runner said `adapter-failed`.
final class AgentStoppedTests: XCTestCase {
    func testAStoppedAgentSaysSoUnderItsConversationWithRestart() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain", "-stopped"]
        app.launchDrawn()

        let line = app.staticTexts["agent-stopped"]
        XCTAssertTrue(
            line.waitForExistence(timeout: 30), "a stopped agent said nothing under its conversation")
        XCTAssertEqual(line.label, "The agent stopped")
        XCTAssertTrue(app.buttons["agent-restart"].exists, "and offered no way to start it again")
        XCTAssertFalse(app.staticTexts["The agent couldn’t start"].exists)
    }

    /// No adapter says so beside the composer, without a Restart that would
    /// fail the same way again.
    func testNoAdapterIsSaidWithoutRestart() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness", "-plain", "-no-adapter"]
        app.launchDrawn()

        let line = app.staticTexts["agent-stopped"]
        XCTAssertTrue(line.waitForExistence(timeout: 30), "no adapter said nothing under the conversation")
        XCTAssertEqual(line.label, "No chat adapter for this agent")
        XCTAssertFalse(app.buttons["agent-restart"].exists, "Restart can't fix a missing adapter")
    }
}
