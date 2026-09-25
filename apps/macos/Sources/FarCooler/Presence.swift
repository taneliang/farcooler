import AppKit
import CoreGraphics

/// Whether somebody is at this Mac, looking at it.
///
/// What `DaemonClient.reportWatching` asks before it tells a runner "these
/// panes are being watched" — a claim that silences the notification for those
/// panes on every device, the watch on the person's wrist included. So a claim
/// made for a person who is not there is a notification they never get:
/// Far Cooler frontmost on a Mac left at the desk, the agent finishes, and
/// nothing buzzes in the kitchen.
///
/// Four things, all of which have to hold:
///
/// - **The app is active.** A window behind another app shows nobody anything.
/// - **The display is awake.** A sleeping screen shows nobody anything, and
///   `NSApp.isActive` stays true through display sleep.
/// - **The session is unlocked and in front.** The lock screen and a fast
///   user switch hide the window without deactivating the app.
/// - **There was input in the last `idleLimit` seconds.** The one signal that
///   catches the person who simply walked away with everything else on.
///
/// Each is a closure so a test can hold any of them false.
@MainActor
struct Presence {
    var appActive: () -> Bool
    var screenAwake: () -> Bool
    var sessionUnlocked: () -> Bool
    var secondsSinceInput: () -> TimeInterval

    /// Sixty seconds without a key or the mouse.
    ///
    /// Long enough to read a screenful of an agent's output without touching
    /// anything — which is what watching an agent mostly is — and short
    /// enough that a person who walks away starts getting notifications again
    /// within about a minute: this plus the runner's ten-second claim
    /// lifetime (`WATCHED_TTL_MS`), at most seventy seconds. When it is wrong
    /// it is wrong toward a buzz about something already on screen, which is
    /// the cheaper mistake than silence about something nobody saw.
    static let idleLimit: TimeInterval = 60

    var isPresent: Bool {
        appActive() && screenAwake() && sessionUnlocked()
            && secondsSinceInput() < Self.idleLimit
    }

    /// This Mac, as it is.
    static var live: Presence { live(ScreenState.shared) }

    /// This Mac, with its display and session read from `screen` — this
    /// Mac's own in `live`, one a test posts to in a test.
    static func live(_ screen: ScreenState) -> Presence {
        Presence(
            appActive: { NSApp.isActive },
            screenAwake: { !screen.displayAsleep },
            sessionUnlocked: { !screen.locked && screen.sessionActive },
            secondsSinceInput: {
                // `kCGAnyInputEventType`: the most recent key, click, scroll
                // or mouse movement of any kind, in this login session.
                CGEventSource.secondsSinceLastEventType(
                    .combinedSessionState, eventType: CGEventType(rawValue: ~0)!)
            })
    }
}

/// The display and session state no AppKit property answers directly, kept
/// from the notifications that announce each change.
///
/// Posts `ScreenState.personLeft` whenever one of them goes away, so a claim
/// can be given back at once rather than aging out on the runner.
@MainActor
final class ScreenState {
    static let shared = ScreenState()
    nonisolated static let personLeft = Notification.Name("FarCoolerPersonLeft")

    private(set) var displayAsleep = false
    /// Assumed unlocked at launch: the app was just opened by somebody.
    private(set) var locked = false
    private(set) var sessionActive = true
    private var tokens: [NSObjectProtocol] = []
    /// Where `personLeft` is posted.
    private let announce: NotificationCenter

    /// The three centers are this Mac's own, except in a test, which hands in
    /// centers of its own to post on — no display has to sleep and no session
    /// has to lock for one to run.
    init(
        workspace: NotificationCenter = NSWorkspace.shared.notificationCenter,
        distributed: NotificationCenter = DistributedNotificationCenter.default(),
        announce: NotificationCenter = .default
    ) {
        self.announce = announce
        observe(workspace, NSWorkspace.screensDidSleepNotification) { $0.displayAsleep = true }
        observe(workspace, NSWorkspace.screensDidWakeNotification) { $0.displayAsleep = false }
        observe(workspace, NSWorkspace.sessionDidResignActiveNotification) { $0.sessionActive = false }
        observe(workspace, NSWorkspace.sessionDidBecomeActiveNotification) { $0.sessionActive = true }
        // Not public API names, but the ones every Mac app that cares uses:
        // the loginwindow posts them on the distributed center.
        observe(distributed, Self.screenIsLocked) { $0.locked = true }
        observe(distributed, Self.screenIsUnlocked) { $0.locked = false }
    }

    nonisolated static let screenIsLocked = Notification.Name("com.apple.screenIsLocked")
    nonisolated static let screenIsUnlocked = Notification.Name("com.apple.screenIsUnlocked")

    private func observe(
        _ center: NotificationCenter, _ name: Notification.Name,
        _ apply: @escaping @MainActor (ScreenState) -> Void
    ) {
        tokens.append(
            center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    apply(self)
                    if self.displayAsleep || self.locked || !self.sessionActive {
                        self.announce.post(name: Self.personLeft, object: nil)
                    }
                }
            })
    }
}
