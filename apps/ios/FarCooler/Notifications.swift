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
    /// The terminal on screen.
    ///
    /// Two readers, and deliberately one register rather than two. A banner
    /// about this pane is suppressed: notifications DO show while the app is
    /// open — you are usually looking at one agent while another is the one that
    /// got stuck, and iOS's default of swallowing every foreground banner would
    /// hide exactly that — and the one case it is noise is being told about the
    /// pane you are already reading.
    ///
    /// The same fact is what ends `done`, which is finished-and-unseen; see
    /// `Connection.markVisibleSeen`. Suppressing a banner and marking something
    /// read are the same judgement — "you are looking at this" — and answering
    /// it in two places is how they come to disagree.
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
        Task { @MainActor in PushRegistration.shared.unavailable(error) }
    }
}
