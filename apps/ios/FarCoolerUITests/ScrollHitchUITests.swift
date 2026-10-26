import XCTest

/// A fling over a long conversation and a long card, timed frame by frame
/// (ov-788, ov-791). The app flings itself (`FrameWatch`, `-frame-watch`):
/// XCUITest's own snapshots between gestures would be the hitches measured.
/// The report is printed as a `FRAME-WATCH` line and attached to the result.
///
/// A measurement, not a gate: a simulator's frame times follow the Mac's
/// load. The gate on the cost found is `FoldedMarkdownPrefixTests` in
/// AgentKit. `FRAME_WATCH_DELAY` (seconds, via `TEST_RUNNER_FRAME_WATCH_DELAY`)
/// waits before the fling, so a profiler can be attached to the app first.
final class ScrollHitchUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func fling(_ app: XCUIApplication, _ name: String) {
        let delay = Double(ProcessInfo.processInfo.environment["FRAME_WATCH_DELAY"] ?? "") ?? 3
        print("FRAME-WATCH-READY \(name)")
        Thread.sleep(forTimeInterval: delay)
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName("com.farcooler.harness.fling" as CFString), nil,
            nil, true)
        // Untouched until it's done: a query mid-fling is main-thread work.
        Thread.sleep(forTimeInterval: 22)
        let watch = app.descendants(matching: .any)["frame-watch"]
        let deadline = Date().addingTimeInterval(60)
        var said = ""
        while Date() < deadline {
            said = watch.value as? String ?? ""
            if !said.isEmpty { break }
            Thread.sleep(forTimeInterval: 1)
        }
        print("FRAME-WATCH \(name) \(said)")
        let attachment = XCTAttachment(string: said)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        XCTAssertTrue(said.hasPrefix("frames="), "no fling measured: \(said)")
    }

    /// The conversation: four hundred uneven rows, lane reports of about
    /// 11 KB folded among them, older pages loading and lines streaming.
    func testFlingTheConversation() {
        let app = XCUIApplication()
        app.launchArguments = [
            "-native-agent-harness", "-native-live", "-native-live-real", "-native-long-reports", "-frame-watch",
        ]
        app.launchDrawn()
        XCTAssertTrue(
            app.descendants(matching: .any)["native-composer"].waitForExistence(timeout: 60), "no conversation")
        fling(app, "conversation")
    }

    /// The Task screen: bil-7 with a real long card's notes, on a board of
    /// eight hundred tasks.
    func testFlingTheTaskScreen() {
        let app = XCUIApplication.phoneHarness([
            "-push-task", "bil-7", "-phone-long-record", "-phone-big-board", "-frame-watch",
        ])
        XCTAssertTrue(app.descendants(matching: .any)["task-heading"].waitForExistence(timeout: 30), "no task")
        // The record is read after the screen shows.
        Thread.sleep(forTimeInterval: 3)
        fling(app, "task")
    }
}
