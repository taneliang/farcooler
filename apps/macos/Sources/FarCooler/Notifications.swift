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
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Notifier()

    private override init() { super.init() }

    /// The terminals each main window has on screen while the app is active,
    /// as of that window's last report, keyed by a per-window identity.
    ///
    /// One set for the whole app was written by whichever window reported
    /// last, so a banner was suppressed for a terminal that only the OTHER
    /// window showed, or raised for one that was on screen there. Whether
    /// anybody is there to see them is asked live.
    private var watchingByWindow: [UUID: Set<String>] = [:]

    /// What every window shows, together.
    var watching: Set<String> { watchingByWindow.values.reduce(into: []) { $0.formUnion($1) } }

    /// Record what one window has on screen, for `willPresent` and for the
    /// claim each runner is told.
    func setWatching(_ terminalIDs: [String], window: UUID) {
        watchingByWindow[window] = Set(terminalIDs)
    }

    /// A window went away: what it showed is no longer shown by anybody on its
    /// account.
    func closeWindow(_ window: UUID) {
        watchingByWindow[window] = nil
    }

    /// How to present a notification that arrives while the app is frontmost.
    ///
    /// macOS shows nothing for the frontmost app's own notification unless
    /// its delegate asks. Asked for here when the terminal it is about is not
    /// on screen to somebody there: the app is in front but the pane that
    /// finished is in another workspace or a collapsed column, or the person
    /// has been idle a minute (`Presence`), and a silent post is how that gets
    /// missed. For a pane on screen the person is looking at it, and a banner
    /// would tell them what they just watched happen; it still goes in the
    /// notification list. Decided from `presence` when the notification
    /// arrives, not from when the window last reported: `watching` is only as
    /// fresh as the last fleet event, and a person can walk away between.
    /// `terminalID` is the notification's thread identifier, which `report`
    /// sets to the terminal's id.
    static func presentation(
        terminalID: String, watching: Set<String>, presence: Presence
    ) -> UNNotificationPresentationOptions {
        presence.isPresent && watching.contains(terminalID) ? [.list] : [.banner, .list, .sound]
    }

    /// The terminal a notification is about, for `presentation`: the push's
    /// or the local post's `terminal`, else its thread, which `report` files
    /// under the terminal's id. Never a task notice's thread (`t:…`) or an
    /// agent notice id (`a:…`), which name no terminal (ov-94).
    nonisolated static func terminalID(userInfo: [AnyHashable: Any], thread: String) -> String? {
        if let named = userInfo["terminal"] as? String, !named.isEmpty { return named }
        if userInfo["kind"] as? String == "task" { return nil }
        if thread.isEmpty || thread.hasPrefix("t:") || thread.hasPrefix("a:") { return nil }
        return thread
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        let content = notification.request.content
        // A task notice has no pane to be on screen: it banners, and the
        // runner's coalescing is what keeps that to one per burst.
        guard let terminalID = Self.terminalID(userInfo: content.userInfo, thread: content.threadIdentifier) else {
            return content.interruptionLevel == .passive ? [.banner, .list] : [.banner, .list, .sound]
        }
        return await MainActor.run {
            Self.presentation(terminalID: terminalID, watching: watching, presence: .live)
        }
    }

    /// A notification's button, or a click on it.
    ///
    /// A task decision's answer buttons write the answer as an ANSWER note
    /// through the runner that posted it, the same note the Needs You rows
    /// write; the runner then tells the agent waiting on it (ov-90). Any
    /// other click just brings the app forward, as it always has.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse
    ) async {
        let info = response.notification.request.content.userInfo
        let action = response.actionIdentifier
        let typed = (response as? UNTextInputNotificationResponse)?.userText
        let target = info["target"] as? String
        guard let notice = TaskNotice(userInfo: info),
            let answer = TaskDecisionActions.answer(action: action, options: notice.options, typed: typed)
        else { return }
        await send(answer, to: notice, target: target)
    }

    /// Send `answer` to `notice`'s task, or say it wasn't sent.
    private func send(_ answer: String, to notice: TaskNotice, target: String?) async {
        let client = target.flatMap { answerers[$0]?.client }
        let refusal: String? =
            if let client {
                await client.answerTask(key: notice.key, body: answer)
            } else {
                "No runner to send it through."
            }
        guard let refusal else { return }
        NSLog("Far Cooler: an answer to %@ wasn't sent: %@", notice.key, refusal)
        guard Self.canNotify else { return }
        let content = UNMutableNotificationContent()
        content.title = "Couldn’t Send Your Answer"
        content.body = "Open Far Cooler to answer \(notice.key) again."
        content.userInfo = notice.userInfo
        // Under the task's own id, so it replaces the decision whose buttons
        // didn't work rather than sitting beside it.
        if let id = notice.noticeId { content.threadIdentifier = id }
        try? await UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: notice.noticeId ?? UUID().uuidString, content: content, trigger: nil))
    }

    /// The runner each task notice was posted from, by `DaemonClient.target`,
    /// so its answer goes back the same way. Weak: a client that has gone
    /// has no runner to answer through.
    private var answerers: [String: WeakClient] = [:]

    private struct WeakClient {
        weak var client: DaemonClient?
    }

    /// Whether a task notice is posted here rather than left to the relay.
    ///
    /// Left to the relay only when both ends are set up for it: the runner is
    /// paired (`Host.push_paired`) AND this Mac has registered for pushes. The
    /// push and a local post share an identifier, so one would replace the
    /// other anyway, but a replacement alerts again: posting both would be two
    /// sounds for one notice.
    static func postsLocally(pushPaired: Bool, registered: Bool) -> Bool {
        !(pushPaired && registered)
    }

    /// Whether `terminal`'s own banner is left to its task's notice (ov-94).
    ///
    /// An agent working on a task is told about through the task, worded in
    /// terms of the task, once its runner sends task notices at all
    /// (`task_notices`): this Mac posts that notice itself. Which task an
    /// agent works on is `TaskLink.noticeTask`, the rule the phones use too,
    /// so an agent opened by hand in a task's lane folds as the runner folds
    /// it (ov-107). An orchestrator is never a task's agent, and an agent with
    /// no task notifies as it always has.
    static func foldsIntoTask(_ terminal: Terminal, in worktree: Worktree, runnerSendsNotices: Bool) -> Bool {
        TaskLink.leavesBannerToTask(terminal, in: worktree, noticeReachesHere: runnerSendsNotices)
    }

    /// The notification for `notice`, posted from the runner `target` names.
    ///
    /// Under the notice's own id, as identifier and thread both, so a newer
    /// notice about the same task replaces this one, and so does the relay's
    /// push about it. A decision gets its options' category, whose buttons
    /// are its answers (`TaskDecisionActions`).
    static func request(for notice: NoticeEvent, target: String) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.title = notice.title
        content.body = notice.body
        content.threadIdentifier = notice.noticeId
        let task = TaskNotice(
            key: notice.task, runner: notice.runner, event: TaskNoticeEvent(rawValue: notice.event),
            noticeId: notice.noticeId, options: notice.event == "decision" ? notice.options : [])
        var info = task.userInfo
        info["target"] = target
        content.userInfo = info
        switch notice.level {
        case "time-sensitive": content.interruptionLevel = .timeSensitive
        case "passive": content.interruptionLevel = .passive
        default: content.interruptionLevel = .active
        }
        if notice.level != "passive" { content.sound = .default }
        if !task.options.isEmpty {
            content.categoryIdentifier = TaskDecisionActions.category(for: task.options)
        }
        return UNNotificationRequest(identifier: notice.noticeId, content: content, trigger: nil)
    }

    /// Post a task notice the runner behind `client` composed, if this Mac
    /// wants its class and the relay won't deliver it anyway.
    func post(notice: NoticeEvent, from client: DaemonClient) {
        answerers[client.target] = WeakClient(client: client)
        guard
            TaskNotifications.wants(notice.event, master: Preferences.shared.notifyOnAttention),
            Self.postsLocally(pushPaired: client.pushPaired, registered: PushRegistration.shared.registered),
            Self.canNotify, authorized
        else { return }
        let request = Self.request(for: notice, target: client.target)
        let options = TaskNotice(userInfo: request.content.userInfo)?.options ?? []
        Task {
            if !options.isEmpty { _ = await TaskDecisionActions.register(options: options) }
            try? await UNUserNotificationCenter.current().add(request)
        }
    }

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
        // The task classes this Mac keeps on, filed with the device so the
        // relay honors them while the app is closed (ov-94). Set here rather
        // than beside `notifyOnDone` so the notifier owns its own settings.
        PushRegistration.shared.notifyEvents = {
            TaskNotifications.events(master: Preferences.shared.notifyOnAttention)
        }
        guard Self.canNotify else { return }
        UNUserNotificationCenter.current().delegate = self
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
    ///
    /// `place` is where the pane is, workspace first — see `place(of:in:workspaces:)`.
    /// `foldsIntoTask` is `Notifier.foldsIntoTask`'s answer for this pane:
    /// its blocked and done banners are its task's notice's to give.
    func report(terminal: Terminal, place: String, foldsIntoTask: Bool = false) {
        reportFailedExit(terminal: terminal, place: place)

        let activity = terminal.agent
        defer { announced[terminal.id] = activity }

        guard Preferences.shared.notifyOnAttention else { return }
        guard !foldsIntoTask else { return }
        guard activity.wantsAttention else { return }
        guard activity != announced[terminal.id] else { return }
        if activity == .done && !Preferences.shared.notifyOnDone { return }
        // `authorized` can only have been set true by a request that ran, which
        // `canNotify` already gates — but this is the other call into the
        // notification centre, and gating it on its own means neither depends
        // on the order they happen to be reached in.
        guard Self.canNotify, authorized else { return }

        // The words are `words(for:place:)`'s, in `WorkspaceActions.swift`,
        // where they can be tested without a notification centre.
        guard let words = Self.words(for: terminal, place: place) else { return }
        let content = UNMutableNotificationContent()
        content.title = words.title
        content.body = words.body
        if activity == .blocked { content.interruptionLevel = .timeSensitive }
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
    private func reportFailedExit(terminal: Terminal, place: String) {
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
        content.title = "\(Self.speaker(terminal)) failed"
        // The code or the signal, whichever the command actually left behind
        // — never both, since a signal means there is no exit code to show.
        if let signal = terminal.exitSignal {
            content.body = "\(place) — Stopped by signal \(signal)"
        } else if let code = terminal.exitCode {
            content.body = "\(place) — Exit code \(code)"
        } else {
            content.body = place
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

/// Whether a main window's panes are in sight: the window isn't in the Dock
/// and some of it is visible, not wholly behind other windows. A window out
/// of sight shows nothing, so it neither silences a banner (`Notifier`) nor
/// tells a runner it's watching (`DaemonClient.reportWatching`), whatever
/// its selection says. Asked with the app active; an inactive app shows
/// nothing already.
@MainActor
enum WindowSight {
    static func inSight(miniaturized: Bool, occlusion: NSWindow.OcclusionState) -> Bool {
        !miniaturized && occlusion.contains(.visible)
    }

    /// `window`'s answer. A window this view hasn't been placed in yet
    /// answers as every window did before: in sight.
    static func inSight(_ window: NSWindow?) -> Bool {
        guard let window else { return true }
        return inSight(miniaturized: window.isMiniaturized, occlusion: window.occlusionState)
    }

    /// `window` is out of sight: what it showed is shown no longer, and each
    /// of its runners is told what the other windows still show.
    static func leave(window: UUID, clients: [DaemonClient]) {
        Notifier.shared.setWatching([], window: window)
        for client in clients { client.reportWatching([]) }
    }
}
