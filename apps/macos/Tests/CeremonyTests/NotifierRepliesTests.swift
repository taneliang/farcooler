import AgentKit
import AppKit
import Foundation
import Testing
import UserNotifications

@testable import Far_Cooler

/// The Mac's notification delegate answers the center on the main thread,
/// after its work (ov-793, as ov-787 did for the phone). Each test calls the
/// delegate through the Objective-C completion-handler selector from a
/// background thread, as the center does; a Swift `await` of an `async`
/// delegate form would never show where the handler was called.
@MainActor
@Suite(.serialized)
struct NotifierRepliesTests {
    @Test("A click is answered on the main thread, after the opener has it")
    func aClickIsAnsweredOnMainAfterItsWork() async {
        let opener = DestinationOpener()
        let notifier = Notifier(opener: opener)
        let window = UUID()
        opener.register(window: window)
        defer { opener.unregister(window: window) }
        let response = Self.response(thread: "agent", action: UNNotificationDefaultActionIdentifier)
        let delegate: UNUserNotificationCenterDelegate = notifier.replies

        let (onMain, queued) = await withCheckedContinuation {
            (done: CheckedContinuation<(Bool, Bool), Never>) in
            DispatchQueue.global().async {
                delegate.userNotificationCenter?(
                    Self.center, didReceive: response,
                    withCompletionHandler: {
                        // On the main thread, so reading main-actor state is sound.
                        let onMain = Thread.isMainThread
                        let queued = onMain && MainActor.assumeIsolated { opener.pending != nil }
                        done.resume(returning: (onMain, queued))
                    })
            }
        }
        #expect(onMain, "the click's handler ran off the main thread")
        #expect(queued, "the handler ran before the click reached the opener")
    }

    @Test("A banner is answered on the main thread with the notifier's choice")
    func aBannerIsAnsweredOnMain() async {
        let notifier = Notifier(opener: DestinationOpener())
        let notification = Self.response(thread: "agent", action: "x").notification
        let delegate: UNUserNotificationCenterDelegate = notifier.replies

        let (onMain, options) = await withCheckedContinuation {
            (done: CheckedContinuation<(Bool, UNNotificationPresentationOptions), Never>) in
            DispatchQueue.global().async {
                delegate.userNotificationCenter?(
                    Self.center, willPresent: notification,
                    withCompletionHandler: { done.resume(returning: (Thread.isMainThread, $0)) })
            }
        }
        #expect(onMain, "the banner's handler ran off the main thread")
        // Nothing is on screen for this pane, so it banners.
        #expect(options.contains(.banner))
    }

    /// A stand-in for the center, which needs an app bundle the test process
    /// is not; the delegate never messages it.
    nonisolated(unsafe) private static let center = unsafeBitCast(NSObject(), to: UNUserNotificationCenter.self)

    /// A response to a notification on `thread`. Neither type has a public
    /// initializer, so both are decoded from a keyed archive of their fields.
    private static func response(thread: String, action: String) -> UNNotificationResponse {
        let terminal = Terminal(id: thread, short: thread, title: "claude", preset: "claude", state: "running", epoch: 0)
        let banner = Notifier.ownBanner(
            terminal: terminal, words: (title: "t", body: "b"), activity: .blocked, host: "", runnerId: nil)
        let notification = decoded(UNNotification.self, ["request": banner, "date": Date()])
        return decoded(UNNotificationResponse.self, ["notification": notification, "actionIdentifier": action])
    }

    private static func decoded<T: NSObject & NSCoding>(_ type: T.Type, _ fields: [String: Any]) -> T {
        let archiver = NSKeyedArchiver(requiringSecureCoding: false)
        for (key, value) in fields { archiver.encode(value, forKey: key) }
        archiver.finishEncoding()
        let unarchiver = try! NSKeyedUnarchiver(forReadingFrom: archiver.encodedData)
        unarchiver.requiresSecureCoding = false
        return T(coder: unarchiver)!
    }
}
