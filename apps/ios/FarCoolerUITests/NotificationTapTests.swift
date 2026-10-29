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
        // Left running, this app was the one the next class's launch failed
        // to terminate ("Failed to terminate", ov-722): it is closed here,
        // whether the test passed or not.
        addTeardownBlock { if app.state != .notRunning { app.terminate() } }
        app.launchDrawn()
        // The first run on a device asks; every later one has its answer.
        // Either way the app marks its window once the notification is
        // filed, and the banner comes fifteen seconds after that: answer the
        // alert while waiting for the mark, then leave.
        let scheduled = app.descendants(matching: .any)["notification-tap-scheduled"]
        let allow = springboard.alerts.buttons["Allow"]
        let deadline = Date().addingTimeInterval(90)
        while !scheduled.exists, Date() < deadline {
            if allow.exists { allow.tap() }
            _ = scheduled.waitForExistence(timeout: 1)
        }
        guard scheduled.exists else { throw HarnessFailure("the app never filed a notification") }
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
        // Waits on the app ending, and returns the moment it does. Staying
        // open has no event to wait on, so a passing run spends the whole five
        // seconds; the crash came about half a second after the tap.
        _ = app.wait(for: .notRunning, timeout: 5)
        XCTAssertEqual(app.state, .runningForeground, "the app closed after a notification was tapped")
    }
}
