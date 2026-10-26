import Foundation
import Testing
import UserNotifications

@testable import AgentKit

/// The center's handlers are called on the main thread, after the app's work
/// (ov-787). On a phone a tap's handler off the main thread is a crash.
///
/// Each test calls the delegate the way the center does: through the
/// Objective-C completion-handler selector, from a thread that is not the main
/// one. That is the path the crash took. A Swift `await` of the `async` form
/// would go nowhere near the thunk that called the handler off the main thread.
struct NotificationRepliesTests {
    @Test func aTapIsAnsweredOnTheMainThreadAfterItsWorkFinishes() async {
        let log = Log()
        let replies = NotificationReplies(
            present: { _ in [] },
            respond: { response in
                log.add("respond on main: \(onMainThread())")
                // The work a tap does is awaited (an answer button writes to
                // a runner); the center is told only once it is done.
                try? await Task.sleep(for: .milliseconds(20))
                log.add("respond done: \(response.notification.request.content.threadIdentifier)")
            })
        let delegate: UNUserNotificationCenterDelegate = replies
        let response = Self.response(thread: "t-1")

        await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
            DispatchQueue.global().async {
                delegate.userNotificationCenter?(
                    Self.center, didReceive: response,
                    withCompletionHandler: {
                        log.add("handler on main: \(Thread.isMainThread)")
                        done.resume()
                    })
            }
        }

        #expect(log.lines == ["respond on main: true", "respond done: t-1", "handler on main: true"])
    }

    @Test func aBannerWhileOpenIsAnsweredOnTheMainThreadWithWhatTheAppSaid() async {
        let replies = NotificationReplies(
            present: { notification in
                notification.request.content.threadIdentifier == "looked-at" ? [] : [.banner, .sound]
            },
            respond: { _ in })
        let delegate: UNUserNotificationCenterDelegate = replies

        for (thread, expected) in [("looked-at", UNNotificationPresentationOptions()), ("other", [.banner, .sound])] {
            let notification = Self.response(thread: thread).notification
            let (onMain, options) = await withCheckedContinuation {
                (done: CheckedContinuation<(Bool, UNNotificationPresentationOptions), Never>) in
                DispatchQueue.global().async {
                    delegate.userNotificationCenter?(
                        Self.center, willPresent: notification,
                        withCompletionHandler: { done.resume(returning: (Thread.isMainThread, $0)) })
                }
            }
            #expect(onMain, "the handler for \(thread) ran off the main thread")
            #expect(options == expected)
        }
    }

    /// A stand-in for the center. `UNUserNotificationCenter.current()` needs an
    /// app bundle, which `swift test` is not, and the delegate never messages
    /// the center it is handed.
    nonisolated(unsafe) private static let center = unsafeBitCast(NSObject(), to: UNUserNotificationCenter.self)

    /// A default-action response to a notification on `thread`. Neither type
    /// has a public initializer, so both are decoded from a keyed archive of
    /// the fields they are made of.
    private static func response(thread: String) -> UNNotificationResponse {
        let content = UNMutableNotificationContent()
        content.threadIdentifier = thread
        let request = UNNotificationRequest(identifier: thread, content: content, trigger: nil)
        let notification = decoded(UNNotification.self, ["request": request, "date": Date()])
        return decoded(
            UNNotificationResponse.self,
            ["notification": notification, "actionIdentifier": UNNotificationDefaultActionIdentifier])
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

/// `Thread.isMainThread`, which Swift 6 will not read in an `async` body.
private func onMainThread() -> Bool { Thread.isMainThread }

/// What happened, in order, from whichever thread it happened on.
private final class Log: @unchecked Sendable {
    private let lock = NSLock()
    private var entries: [String] = []

    func add(_ line: String) { lock.withLock { entries.append(line) } }
    var lines: [String] { lock.withLock { entries } }
}
