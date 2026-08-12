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

    private var authorized = false
    /// What was last announced per terminal, so a state that persists is
    /// announced once. The daemon sends only changes, but a reconnect replays
    /// current state, and being told twice that the same agent finished is how
    /// people learn to ignore notifications.
    private var announced: [String: AgentActivity] = [:]

    /// Whether this build can talk to the notification centre at all.
    ///
    /// `UNUserNotificationCenter.current()` does not fail politely for an
    /// executable with no bundle identifier: it raises
    /// `NSInternalInconsistencyException`, which is an Objective-C exception,
    /// which Swift cannot catch. The process aborts.
    ///
    /// A `swift build` product is exactly such an executable, and running one
    /// directly is what this project's own README documents — so the
    /// documented way to run the app crashed it on launch, twice on the
    /// developer's machine before anybody read the stack. Asked once and
    /// cached, because the answer cannot change while the process lives.
    ///
    /// This is not a repair. An unbundled build genuinely cannot receive
    /// notifications; what changes is that it now runs without them instead of
    /// not running.
    private static let canNotify: Bool = {
        guard Bundle.main.bundleIdentifier != nil else {
            NSLog("Far Cooler: launched without a bundle identifier, so notifications are off.")
            return false
        }
        return true
    }()

    func requestAuthorization() {
        guard Self.canNotify else { return }
        UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound]) { [weak self] granted, _ in
                Task { @MainActor in
                    self?.authorized = granted
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
        // `authorized` can only have been set true by a request that ran, which
        // `canNotify` already gates — but this is the other call into the
        // notification centre, and gating it on its own means neither depends
        // on the order they happen to be reached in.
        guard Self.canNotify, authorized else { return }

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
        guard Self.canNotify else { return }
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
