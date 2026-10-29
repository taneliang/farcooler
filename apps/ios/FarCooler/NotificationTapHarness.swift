#if DEBUG
import UIKit
import UserNotifications

/// A real notification for the UI suite to tap (ov-787), under
/// `-notification-tap-harness`.
///
/// A tap crashed the app on a phone: the center's handler for it was called
/// off the main thread, and UIKit asserts that the snapshot it then takes is
/// taken on the main one. Nothing short of a tap goes through that handler, so
/// the suite needs a notification on the Lock Screen, and only the app can put
/// one there. This asks for permission and files one fifteen seconds out; the
/// test closes or leaves the app meanwhile and taps it (`NotificationTapTests`).
///
/// Fifteen, because a banner that fires while the app is still open is shown
/// over the app and gone before the test reaches the Home Screen: the test
/// spends up to five seconds looking for the permission alert, which a device
/// that has answered never shows.
///
/// It carries no destination, so the tap opens the app and goes nowhere: the
/// handler is called the same either way, and the test is about the handler.
enum NotificationTapHarness {
    static var isRequested: Bool { CommandLine.arguments.contains("-notification-tap-harness") }

    /// What the banner says, which is how the test finds it.
    static let title = "A Notification to Tap"

    static func scheduleIfRequested() {
        guard isRequested else { return }
        let center = UNUserNotificationCenter.current()
        center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
            guard granted else { return }
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = "A notification for the UI suite to tap."
            center.add(
                UNNotificationRequest(
                    identifier: "notification-tap-harness", content: content,
                    trigger: UNTimeIntervalNotificationTrigger(timeInterval: 15, repeats: false))
            ) { _ in
                DispatchQueue.main.async { markScheduled(scheduledIdentifier) }
            }
        }
    }

    /// What the test waits for before it closes or leaves the app: the
    /// notification is filed, so fifteen seconds from now it is a banner.
    /// A fresh simulator asks permission first and the answer can take any
    /// time; one that has answered never asks, and the test can't tell which
    /// from the outside. Waiting a fixed 45 s for an alert that never came
    /// let the banner fire over the open app and vanish (red locally).
    static let scheduledIdentifier = "notification-tap-scheduled"

    @MainActor private static func markScheduled(_ id: String) {
        for case let scene as UIWindowScene in UIApplication.shared.connectedScenes {
            for window in scene.windows {
                let mark = UILabel(frame: CGRect(x: 0, y: 0, width: 4, height: 4))
                mark.isAccessibilityElement = true
                mark.accessibilityIdentifier = id
                mark.accessibilityLabel = id
                window.addSubview(mark)
            }
        }
    }
}
#endif
