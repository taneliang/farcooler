import XCTest

/// Tapping a notification opens the app, and the app stays open (ov-787).
///
/// It didn't: on a phone the tap launched the app and it aborted about half a
/// second later, because the center's handler for the tap ran off the main
/// thread and UIKit asserts that the snapshot it takes then is taken on the
/// main one. The handler is only ever called for a real tap, so these tap a
/// real banner: `-notification-tap-harness` files one fifteen seconds out, the
/// test closes the app or leaves it, and taps the banner on the Home Screen.
final class NotificationTapTests: XCTestCase {
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    override func setUp() {
        continueAfterFailure = false
    }

    /// The owner's case: the app was not running, and the tap launched it.
    ///
    /// On the iOS 27 simulator the old delegate survived this one, so it is
    /// the next test that goes red for the crash; this one holds that a tap
    /// from closed still opens the app, which is what the phone failed at.
    func testATapThatLaunchesTheAppLeavesItOpen() throws {
        try tapBanner { app in app.terminate() }
    }

    /// The app was in the background, and the tap brought it back.
    ///
    /// Against the old delegate this crashed on the simulator with the
    /// phone's own stack: `@objc closure #1 in
    /// ForegroundPresenter.userNotificationCenter(_:didReceive:)` into
    /// `-[UIApplication _updateSnapshotAndStateRestorationWithAction:windowScene:]`,
    /// on a cooperative-pool thread.
    func testATapThatReopensTheAppLeavesItOpen() throws {
        try tapBanner { _ in XCUIDevice.shared.press(.home) }
    }

    private func tapBanner(leaving leave: (XCUIApplication) -> Void) throws {
        let app = XCUIApplication()
        app.launchArguments = ["-notification-tap-harness"]
        app.launchDrawn()
        // The first run on a device asks; every later one has its answer.
        let allow = springboard.alerts.buttons["Allow"]
        if allow.waitForExistence(timeout: 5) { allow.tap() }
        leave(app)

        // Filed fifteen seconds after launch: found by what it says, wherever
        // on the Home Screen the banner is drawn.
        let banner = springboard.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS %@", "A Notification to Tap"))
            .firstMatch
        guard banner.waitForExistence(timeout: 60) else {
            throw HarnessFailure("no banner arrived to tap")
        }
        banner.tap()

        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 30), "the tap didn't open the app")
        // The crash came about half a second after the tap launched the app;
        // a few seconds is past every handler a tap starts.
        _ = app.wait(for: .notRunning, timeout: 5)
        XCTAssertEqual(app.state, .runningForeground, "the app closed after a notification was tapped")
    }
}
