import Foundation
import UIKit
import UserNotifications

/// Telling you when an agent needs you.
///
/// The Mac's `Notifier`, ported — same two states, same rule about which are
/// worth announcing. Only a BLOCKED agent waiting on you and a DONE one are
/// ever announced: working agents are the normal case, and a product that
/// buzzes for the normal case is one people turn off, after which it cannot
/// tell them the thing that mattered.
///
/// The daemon decides those states, so this works identically for an agent on
/// the machine in front of you and one across the world. That is also what
/// makes the push path below a delivery change rather than a rethink.
@MainActor
final class Notifier {
    static let shared = Notifier()

    private var authorised = false
    /// The terminal on screen, so a banner about it can be suppressed.
    ///
    /// Notifications DO show while the app is open — you are usually looking at
    /// one agent while another is the one that got stuck, and iOS's default of
    /// swallowing every foreground banner would hide exactly that. The one case
    /// it is noise is being told about the pane you are already reading.
    var visibleTerminal: String?

    private let presenter = ForegroundPresenter()
    /// What was last announced per terminal, so a state that persists is
    /// announced once. The fleet is polled, so the same `done` arrives over and
    /// over; being told twice that the same agent finished is how people learn
    /// to ignore notifications.
    private var announced: [String: AgentActivity] = [:]

    func requestAuthorisation() {
        UNUserNotificationCenter.current().delegate = presenter
        UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound, .badge]) { [weak self] granted, _ in
                Task { @MainActor in
                    self?.authorised = granted
                    guard granted else { return }
                    // Ask APNs for an address as soon as there is permission to
                    // use one. The token is useless without an account, and
                    // `PushRegistration` simply holds it until there is one —
                    // better than making sign-in trigger a second permission
                    // dance later.
                    UIApplication.shared.registerForRemoteNotifications()
                }
            }
    }

    /// Announce a change, if it is worth announcing.
    func report(terminal: Terminal, workspace: String) {
        let activity = terminal.agent
        defer { announced[terminal.id] = activity }

        guard NotificationSettings.onAttention else { return }
        guard activity.wantsAttention else { return }
        guard activity != announced[terminal.id] else { return }
        if activity == .done && !NotificationSettings.onDone { return }
        guard authorised else { return }

        let content = UNMutableNotificationContent()
        switch activity {
        case .blocked:
            content.title = "\(terminal.label) needs you"
            content.body = "\(workspace) — waiting for your answer"
            // The one state worth breaking through a Focus for: an agent that
            // is blocked has stopped, and will stay stopped until answered.
            content.interruptionLevel = .timeSensitive
        case .done:
            content.title = "\(terminal.label) finished"
            content.body = workspace
        default:
            return
        }
        content.sound = .default
        // Keyed by terminal so a later state replaces the earlier notification
        // for the same one rather than stacking up.
        content.threadIdentifier = terminal.id

        UNUserNotificationCenter.current().add(
            UNNotificationRequest(
                identifier: "\(terminal.id)-\(activity.rawValue)",
                content: content,
                trigger: nil))
    }

    /// Forget a terminal that no longer exists, so a reused id cannot inherit
    /// the announcement history of the terminal it replaced.
    func forget(_ terminalID: String) {
        announced.removeValue(forKey: terminalID)
        UNUserNotificationCenter.current()
            .removeDeliveredNotifications(withIdentifiers: [terminalID])
    }
}


/// Shows banners while the app is open, except for the pane being looked at.
///
/// Without a delegate, iOS decides that an app in the foreground does not need
/// telling — which is right for a messaging app where the message is already on
/// screen, and wrong here: the fleet is many agents and you can only look at
/// one.
private final class ForegroundPresenter: NSObject, UNUserNotificationCenterDelegate {
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        let subject = notification.request.content.threadIdentifier
        let visible = await MainActor.run { Notifier.shared.visibleTerminal }
        return subject == visible ? [] : [.banner, .sound]
    }
}


/// Where APNs delivers this device's address, and where it goes next.
///
/// The local `Notifier` above covers the case where the phone is awake and
/// polling. This covers the case the product exists for: the phone is in a
/// pocket, the app is not running, and an agent on a machine three time zones
/// away is stuck. Nothing local can know that — only the daemon can, and the
/// only way it reaches a sleeping phone is APNs.
///
/// Overnight's own servers never see the notification's contents beyond a title
/// and a terminal id, and never see an APNs key: the key lives in the relay's
/// secrets, and the daemon holds only a token that names an account.
@MainActor
final class PushRegistration: NSObject, ObservableObject {
    static let shared = PushRegistration()

    /// The last token APNs gave us, held until there is an account to file it
    /// under. Registration and sign-in happen in either order, and whichever
    /// finishes second is what completes the pair.
    private var token: String?

    func received(_ deviceToken: Data) {
        token = deviceToken.map { String(format: "%02x", $0) }.joined()
        Task { await sendIfPossible() }
    }

    /// Called after signing in, for the case where the token arrived first.
    func sendIfPossible() async {
        guard let token, Account.shared.isSignedIn else { return }
        await Account.shared.registerDevice(
            pushToken: token, platform: "apns", label: UIDevice.current.name)
    }
}

/// The only reason this app has a delegate.
///
/// SwiftUI has no scene-phase equivalent of "APNs answered" — the token arrives
/// through `UIApplicationDelegate` and nowhere else.
final class PushDelegate: NSObject, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        Task { @MainActor in PushRegistration.shared.received(deviceToken) }
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        // Not surfaced. It fails on the simulator and in builds without a push
        // entitlement, neither of which is a thing to tell a user about, and
        // local notifications keep working regardless.
        print("remote notifications unavailable: \(error.localizedDescription)")
    }
}
