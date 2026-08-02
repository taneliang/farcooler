import AgentKit
import AppKit
import Foundation
import UserNotifications

/// Telling you when an agent needs you.
///
/// Only two things are ever notified about: an agent that is BLOCKED waiting on
/// you, and one that is DONE. Working agents are the normal case, and a product
/// that buzzes for the normal case is one people turn off — after which it
/// cannot tell them the thing that mattered.
///
/// The daemon decides what those states are, so this works identically for an
/// agent on this Mac and one on a machine across the world. That is also what
/// makes a future push notification or Live Activity a delivery change rather
/// than a rethink.
@MainActor
final class Notifier {
    static let shared = Notifier()

    private var authorised = false
    /// What was last announced per terminal, so a state that persists is
    /// announced once. The daemon sends only changes, but a reconnect replays
    /// current state, and being told twice that the same agent finished is how
    /// people learn to ignore notifications.
    private var announced: [String: AgentActivity] = [:]

    func requestAuthorisation() {
        UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound]) { [weak self] granted, _ in
                Task { @MainActor in
                    self?.authorised = granted
                    guard granted else { return }
                    // A Mac gets remote notifications for the same reason a
                    // phone does: it can be closed, asleep, or simply not the
                    // machine the agent is running on.
                    NSApplication.shared.registerForRemoteNotifications()
                }
            }
    }

    /// Announce a change, if it is worth announcing.
    func report(terminal: Terminal, workspace: String) {
        let activity = terminal.agent
        defer { announced[terminal.id] = activity }

        guard Preferences.shared.notifyOnAttention else { return }
        guard activity.wantsAttention else { return }
        guard activity != announced[terminal.id] else { return }
        if activity == .done && !Preferences.shared.notifyOnDone { return }
        guard authorised else { return }

        let content = UNMutableNotificationContent()
        switch activity {
        case .blocked:
            content.title = "\(terminal.title) needs you"
            content.body = "\(workspace) — waiting for your answer"
            content.interruptionLevel = .timeSensitive
        case .done:
            content.title = "\(terminal.title) finished"
            content.body = workspace
        default:
            return
        }
        content.sound = .default
        // Keyed by terminal so a later state replaces the earlier notification
        // for the same one instead of stacking up.
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


/// The only reason this app has a delegate: the APNs token arrives nowhere else.
final class PushDelegate: NSObject, NSApplicationDelegate {
    func application(
        _ application: NSApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        Task { @MainActor in PushRegistration.shared.received(deviceToken) }
    }

    func application(
        _ application: NSApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        Task { @MainActor in PushRegistration.shared.unavailable(error) }
    }
}
