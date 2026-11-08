import XCTest

/// Opt-in captures for the Review screen and the watch screen on the narrowest iPhone (ov-976, ov-978).
///
/// Written to `FARCOOLER_CAPTURE_DIR` (`TEST_RUNNER_FARCOOLER_CAPTURE_DIR`); skipped without it. The harness
/// feeds the daemon's own question and a recorded pane's real bytes (`HarnessWorkflowFixtures`). The
/// watch is asked for nothing it could type: the only taps here are on Close and the Review's own buttons are
/// never pressed.
final class AgentWatchCaptureTests: XCTestCase {
    private func directory() throws -> URL {
        guard let dir = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_DIR"] else {
            throw XCTSkip("captures are opt-in: set TEST_RUNNER_FARCOOLER_CAPTURE_DIR")
        }
        return URL(fileURLWithPath: dir)
    }

    private func save(_ name: String, in dir: URL) throws {
        let suffix = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_NAME"] ?? "shot"
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try XCUIScreen.main.screenshot().pngRepresentation.write(to: dir.appendingPathComponent("ios-\(name)-\(suffix).png"))
    }

    func testCaptureTheReviewScreen() throws {
        let dir = try directory()
        let app = XCUIApplication.phoneHarness(["-push-task", "bil-7", "-phone-workflow-change"])
        let approve = app.buttons["Approve"]
        let review = app.descendants(matching: .any)["workflow-change-review"]
        XCTAssertTrue(review.waitForExistence(timeout: 30), "no Review view on the task")
        try save("review-top", in: dir)
        // Short drags, so a button is never scrolled past between two looks.
        let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75))
        let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45))
        for _ in 0..<40 where !(approve.exists && approve.isHittable) { from.press(forDuration: 0.05, thenDragTo: to) }
        XCTAssertTrue(approve.exists, "Approve is not at the end of the change")
        try save("review-end", in: dir)
    }

    func testCaptureTheWatchScreen() throws {
        let dir = try directory()
        for state in ["running", "finished", "lost", "not-running"] {
            let app = XCUIApplication.phoneHarness(["-phone-agent-watch", state])
            if state == "not-running" {
                XCTAssertTrue(app.staticTexts["This agent isn’t running right now."].waitForExistence(timeout: 30), "no end-of-run message")
            } else {
                XCTAssertTrue(app.descendants(matching: .any)["agent-watch-surface"].waitForExistence(timeout: 30), "no picture")
            }
            sleep(2)
            try save("watch-\(state)", in: dir)
            if state == "running" {
                let surface = app.descendants(matching: .any)["agent-watch-surface"]
                surface.swipeLeft()
                sleep(1)
                try save("watch-running-scrolled", in: dir)
            }
            app.terminate()
        }
    }
}
