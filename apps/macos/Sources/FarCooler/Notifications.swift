import AgentKit
import AppKit
import Foundation
import UserNotifications

/// Telling you when something needs you.
///
/// An agent that is BLOCKED waiting on you, one that is DONE, and a plain
/// command that ran and came back badly. Working agents and a clean exit are
/// both the normal case, and a product that buzzes for the normal case is one
/// people turn off — after which it cannot tell them the thing that mattered.
///
/// The daemon decides what those states are, so this works identically for an
/// agent on this Mac and one on a runner across the world. That is also what
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
    /// Terminals a failed exit has already been announced for.
    ///
    /// Kept apart from `announced`: a failed command has no agent to be
    /// blocked or done, so `Status.failedRun` needs its own dedup rather than
    /// borrowing a dictionary keyed on the wrong enum for this case.
    private var announcedFailure: Set<String> = []

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
                    // runner the agent is running on.
                    NSApplication.shared.registerForRemoteNotifications()
                }
            }
    }

    /// Announce a change, if it is worth announcing.
    func report(terminal: Terminal, workspace: String) {
        reportFailedExit(terminal: terminal, workspace: workspace)

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
            // How the turn ENDED, which `activity` alone cannot say — the
            // daemon reads it out of the agent's own log and sends it beside
            // this. Telling someone an agent finished when its turn died is
            // the same lie the green dot used to tell, in the surface they are
            // most likely to be looking at.
            if terminal.status == .failedTurn {
                content.title = "\(terminal.title) failed"
                content.body = "\(workspace) — its last turn didn't finish"
            } else {
                content.title = "\(terminal.title) finished"
                content.body = workspace
            }
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

    /// Announce a command that ran and came back badly, if that hasn't
    /// already happened for this terminal.
    ///
    /// This is the notification `wants_attention` used to leave out: a `cargo
    /// build` that exited 101 at 3am reached the sidebar dot and nothing
    /// else. Split out from `report` rather than folded into its switch
    /// because `.failedRun` is a `Status`, not an `AgentActivity` — a failed
    /// command has no agent to be blocked or done — so it needs its own guard
    /// and its own dedup, not a case squeezed into an enum it doesn't belong
    /// to.
    private func reportFailedExit(terminal: Terminal, workspace: String) {
        guard terminal.status == .failedRun else {
            // Cleared rather than left set, so a terminal that is rerun after
            // a failure — same pane, same id, `exit` and the command run
            // again — can fail a second time and be announced a second time,
            // instead of staying silenced because it once failed.
            announcedFailure.remove(terminal.id)
            return
        }
        guard Preferences.shared.notifyOnAttention else { return }
        guard !announcedFailure.contains(terminal.id) else { return }
        announcedFailure.insert(terminal.id)
        guard Self.canNotify, authorized else { return }

        let content = UNMutableNotificationContent()
        content.title = "\(terminal.title) failed"
        // The code or the signal, whichever the command actually left behind
        // — never both, since a signal means there is no exit code to show.
        if let signal = terminal.exitSignal {
            content.body = "\(workspace) — stopped by signal \(signal)"
        } else if let code = terminal.exitCode {
            content.body = "\(workspace) — exit code \(code)"
        } else {
            content.body = workspace
        }
        content.sound = .default
        content.interruptionLevel = .timeSensitive
        content.threadIdentifier = terminal.id

        UNUserNotificationCenter.current().add(
            UNNotificationRequest(
                identifier: "\(terminal.id)-failedRun",
                content: content,
                trigger: nil))
    }

    /// Forget a terminal that no longer exists, so a reused id cannot inherit
    /// the announcement history of the terminal it replaced.
    func forget(_ terminalID: String) {
        announced.removeValue(forKey: terminalID)
        announcedFailure.remove(terminalID)
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
