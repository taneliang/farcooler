import XCTest

/// An image can be put into the orchestrator's message again (ov-444).
///
/// The orchestrator as a terminal (the projector off, or its conversation put
/// away) had no way to take an image: the shell pane's image menu never
/// reached its own screen. Its photo door is a toolbar menu with the library's
/// picker and the pasteboard's image, and what it sends shows as a chip over
/// the pane. The system's photo picker can't be driven, so the harness posts
/// the photo a picker would hand over (`OrchestratorImageDoor`); it takes the
/// door's own path from there. The conversation's and a chat pane's composers
/// have their own photo buttons, tested where they are (`NativeComposerTests`,
/// `ComposerWidthUITests`).
final class OrchestratorImageDoorTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id]
    }

    private func post(_ name: String) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName(name as CFString), nil, nil, true)
    }

    private func openOrchestrator(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox", "-phone-billing-led"] + extra)
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        XCTAssertTrue(element(app, "orchestrator-pane").waitForExistence(timeout: 30), "not on the orchestrator")
        return app
    }

    /// **The terminal orchestrator offers a photo and a paste**: the menu is
    /// in the bar, and Choose Photo is in it.
    func testTheOrchestratorTerminalOffersAPhoto() {
        let app = openOrchestrator()
        let menu = element(app, "orchestrator-image-menu")
        XCTAssertTrue(menu.waitForExistence(timeout: 30), "no way to add an image to the orchestrator: \(app.debugDescription)")
        menu.tap()
        XCTAssertTrue(element(app, "orchestrator-choose-photo").waitForExistence(timeout: 10), "no Choose Photo in the menu")
    }

    /// **A photo becomes a chip over the pane**, on its way to the orchestrator.
    func testAPhotoIsAChipOverTheOrchestrator() {
        let app = openOrchestrator()
        XCTAssertTrue(element(app, "orchestrator-image-menu").waitForExistence(timeout: 30), "no image menu")
        XCTAssertFalse(element(app, "image-paste-chip").exists, "a chip before any photo")
        post("com.farcooler.harness.orchestrator-photo")
        XCTAssertTrue(element(app, "image-paste-chip").waitForExistence(timeout: 30), "the photo never became a chip")
    }

    /// **A chat orchestrator has its composer's button, not a second door.**
    func testAChatOrchestratorHasNoSecondDoor() {
        let app = openOrchestrator(["-phone-orchestrator-chat"])
        XCTAssertTrue(element(app, "agent-send").waitForExistence(timeout: 30), "no chat composer")
        XCTAssertFalse(element(app, "orchestrator-image-menu").exists, "two doors to one action")
    }
}
