import XCTest

/// Accessibility-size Dynamic Type, which broke three controls: the workspace
/// segment bar ("Orches-/trator"), the queued message's actions ("Queu/ed",
/// "Re-/move") and the composer's chips ("Ma… S… Hi…").
///
/// The size comes in as a launch argument, which the app reads as its own
/// preferred content size. Each test asserts a SHAPE rather than a pixel: a
/// label that cannot wrap is as tall as the label beside it that has one word,
/// and a chip that is not truncated is as wide as its own text.
final class DynamicTypeTests: XCTestCase {
    private static let size = [
        "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL",
    ]

    /// **The segment bar keeps every title on one line.** "Orchestrator" is the
    /// longest word and "Board" the shortest, so a bar that lets the long one
    /// wrap makes its button taller than the other's.
    func testTheSegmentBarNeverWrapsATitle() throws {
        let app = XCUIApplication.phoneHarness(["-phone-empty-inbox"] + Self.size)
        let row = app.buttons["workspace-row-Main"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no Main row")
        row.tap()
        let board = app.buttons["segment-board"]
        XCTAssertTrue(board.waitForExistence(timeout: 10), "the workspace did not open")
        for id in ["segment-orchestrator", "segment-worktrees"] {
            let segment = app.buttons[id]
            XCTAssertTrue(segment.exists, "no \(id)")
            XCTAssertEqual(
                segment.frame.height, board.frame.height, accuracy: 2,
                "\(id) is taller than a one-word title: it wrapped")
        }
    }

    /// **The queue's actions and the composer's chips hold up.** Each action is
    /// one line tall, and each chip is as wide as its own text needs, which a
    /// truncated "Ma…" is not.
    func testTheQueueAndTheChipsAreNotBroken() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-agent-layout-harness"] + Self.size
        app.launch()
        let send = app.buttons["Send Now"]
        XCTAssertTrue(send.waitForExistence(timeout: 30), "the queued message was not drawn")
        let edit = app.buttons["Edit"]
        XCTAssertEqual(
            send.frame.height, edit.frame.height, accuracy: 2, "Send Now wrapped")
        XCTAssertEqual(
            app.buttons["Remove"].frame.height, edit.frame.height, accuracy: 2, "Remove wrapped")
        for name in ["Manual", "Sonnet", "High"] {
            let chip = app.buttons[name]
            XCTAssertTrue(chip.waitForExistence(timeout: 5), "no \(name) chip")
            XCTAssertGreaterThan(
                chip.frame.width, CGFloat(name.count) * 14,
                "\(name) was squeezed to \(chip.frame.width) points")
        }
        XCTAssertFalse(
            app.descendants(matching: .any)["adapter-badge"].exists,
            "the ACP badge still takes room at this size")
    }
}
