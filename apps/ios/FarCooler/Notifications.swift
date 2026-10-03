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

    private var authorized = false
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
    ///
    /// **Written here and nowhere else**, through `claim` and `release`
    /// (ov-66). Three screens decide what's being read — the shell's pane at
    /// rest (`ShellScreen.markVisible`), a pane's own mount
    /// (`TerminalView`), and the workspace's orchestrator — and each used to
    /// assign this directly, so one clearing on its way out wiped a claim
    /// another had made since. A release names what it gives back, and
    /// gives back only that.
    private(set) var visibleTerminal: String?

    /// `id` is the pane being read now, or nil for none: the Changes tab,
    /// or the grid up.
    func claim(_ id: String?) {
        visibleTerminal = id
    }

    /// `id` isn't being read any more. A no-op when another pane has
    /// claimed since, which is the whole difference from `claim(nil)`.
    func release(_ id: String) {
        if visibleTerminal == id { visibleTerminal = nil }
    }

    private let presenter = ForegroundPresenter()
    /// What was last announced per terminal, so a state that persists is
    /// announced once. The fleet is polled, so the same `done` arrives over and
    /// over; being told twice that the same agent finished is how people learn
    /// to ignore notifications.
    private var announced: [String: AgentActivity] = [:]

    /// Be the notification center's delegate: banners while the app is
    /// open, and where a tapped one goes. At launch, from `PushDelegate`,
    /// since a tap that launched the app is delivered to whatever delegate
    /// is set by the time launching ends, and to nothing otherwise.
    func listen() {
        UNUserNotificationCenter.current().delegate = presenter
    }

    func requestAuthorization() {
        listen()
        UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound, .badge]) { [weak self] granted, _ in
                Task { @MainActor in
                    self?.authorized = granted
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
    ///
    /// `report.leftToTask` is `Fleet.agentReports`'s answer for this pane:
    /// an agent working on a task is told about through the task's push,
    /// worded by task and filed under the task's own thread, so posting its
    /// own banner too would say the same thing twice (ov-107). Still
    /// recorded as announced, so a fold that ends doesn't replay old news.
    func report(_ report: AgentReport) {
        let terminal = report.terminal, worktree = report.worktree
        let activity = terminal.agent
        defer { announced[terminal.id] = activity }

        guard NotificationSettings.onAttention else { return }
        guard !report.leftToTask else { return }
        guard activity.wantsAttention else { return }
        guard activity != announced[terminal.id] else { return }
        if activity == .done && !NotificationSettings.onDone { return }
        guard authorized else { return }

        let content = UNMutableNotificationContent()
        switch activity {
        case .blocked:
            content.title = "\(terminal.label) needs you"
            // Capitalized to match the daemon's own `watch::notification`, which
            // writes this same sentence into the push it sends when this app is
            // closed. One person gets whichever of the two is delivered, about
            // one pane, and two casings of one sentence is two notifications.
            content.body = "\(worktree) — Waiting for your answer"
            // The one state worth breaking through a Focus for: an agent that
            // is blocked has stopped, and will stay stopped until answered.
            content.interruptionLevel = .timeSensitive
        case .done:
            // Whether it finished or DIED. The daemon reads that out of the
            // agent's own log; `activity` says only that the turn ended, and a
            // card claiming an agent finished when its turn came back an error
            // is the lie the row's green checkmark used to tell.
            if terminal.turnDidFail {
                content.title = "\(terminal.label) failed"
                content.body = "\(worktree) — Its last turn didn’t finish"
            } else {
                content.title = "\(terminal.label) finished"
                // What it finished, where there is an answer to that.
                //
                // The body was the worktree name alone, which the title's
                // label had very nearly already said — so the whole
                // notification was "claude finished / add auth", a sentence
                // about Far Cooler rather than about the work. The agent's own
                // last words are already on the row by the time this fires,
                // redacted and cut to a window by the daemon, and they are the
                // difference between knowing something ended and knowing
                // whether to go and look.
                //
                // The worktree stays in front of it: several panes finish in a
                // day and which worktree this was is what tells them apart.
                // Falls back to the worktree alone when the turn was all tool
                // calls and no prose, which is a real case.
                //
                // `lastSaid`, not `recentSteps.last`, which is what this used
                // to read. A step is a wrapped ROW, so the last of them is the
                // last forty characters of the window — the END of the message
                // — and a notification reports something already over, where
                // what a sentence opens with is what it is about. A turn that
                // ended "More shit. An industrial quantity of shit, shipped in
                // carefully authorized batches to avoid N+1 shits." arrived as
                // "batches to avoid N+1 shits." until it did not. The whole
                // message is on the wire now, cut from its start by the daemon
                // — see `Terminal.lastSaid`.
                if let said = terminal.lastSaid, !said.isEmpty {
                    content.body = "\(worktree) — \(said)"
                } else {
                    content.body = worktree
                }
            }
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

    /// A tapped notification: its agent's pane, or a decision's task, over
    /// the screens that lead to it (ruling 3). Handed to `FleetView` through
    /// `NotificationTaps`, which routes it as it routes a card's link.
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let content = response.notification.request.content
        // A task decision's answer button (ov-94): written as the person,
        // through the runner the task is on, without opening the app.
        if let notice = TaskNotice(userInfo: content.userInfo),
            let answer = TaskDecisionActions.answer(
                action: response.actionIdentifier, options: notice.options,
                typed: (response as? UNTextInputNotificationResponse)?.userText)
        {
            await WatchLinkHost.shared.answerDecision(notice, with: answer)
            return
        }
        guard response.actionIdentifier == UNNotificationDefaultActionIdentifier,
            let tap = PushTap(userInfo: content.userInfo, thread: content.threadIdentifier)
        else { return }
        await MainActor.run { NotificationTaps.shared.tap = tap }
    }
}

/// The last notification tapped, until `FleetView` takes it.
@MainActor
final class NotificationTaps: ObservableObject {
    static let shared = NotificationTaps()
    @Published var tap: PushTap?
}


/// Why this app has a delegate.
///
/// SwiftUI has no scene-phase equivalent of "APNs answered" — the token arrives
/// through `UIApplicationDelegate` and nowhere else. The watch link is here for
/// the mirror-image reason: it has to be listening before any scene exists.
final class PushDelegate: NSObject, UIApplicationDelegate {
    /// Activate the watch link at launch, including a launch nobody asked for,
    /// and let a lock screen card's buttons find their way here too.
    ///
    /// iOS starts this app in the background to deliver a `sendMessage` from the
    /// watch, and a background launch may never build a `WindowGroup` at all —
    /// so a SwiftUI `.task`, where `LiveActivities.start` lives, is a hook that
    /// does not run in exactly the case the watch exists for. This one does:
    /// `didFinishLaunchingWithOptions` runs on every launch, foreground or not.
    /// Without it the watch's Allow button reaches a phone with no delegate
    /// listening, waits out WatchConnectivity's timeout, and reports that the
    /// phone did not answer.
    ///
    /// `acceptAnswersFromGlances` is here for the identical reason and it is
    /// the same sentence one surface further out: iOS launches this app into
    /// the background to perform an `AppIntent`, so the handler that carries a
    /// card's tap to a runner has to be installed before any scene exists. A
    /// button whose intent runs against an uninstalled handler reports that it
    /// could not send — honest, and avoidable by one line at the right moment.
    ///
    /// `assumeIsolated` rather than a `Task`, and it is an assertion rather
    /// than a hope: UIKit calls every `UIApplicationDelegate` lifecycle method
    /// on the main thread, so the claim is true and the check is free. A `Task`
    /// would defer activation past the end of launch, which is precisely the
    /// window a launch-to-deliver-a-message has.
    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        MainActor.assumeIsolated {
            WatchLinkHost.shared.start()
            WatchLinkHost.shared.acceptAnswersFromGlances()
            Notifier.shared.listen()
        }
        return true
    }

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
