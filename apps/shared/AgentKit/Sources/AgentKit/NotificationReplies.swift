import Foundation
import UserNotifications

/// The notification center's delegate, which hands the center its answer on
/// the main thread (ov-787).
///
/// Tapping a banner crashed the phone app. The center gives its delegate a
/// completion handler for each banner it is about to show and each one that
/// was tapped, and on iOS the handler for a tap goes on to update the app
/// switcher's snapshot, which UIKit asserts happens on the main thread:
/// `-[UIApplication _performBlockAfterCATransactionCommitSynchronizes:]`
/// raised, and the app aborted about half a second after the tap launched it.
///
/// The delegate before this one implemented the two methods as `async`, and an
/// `async` method that Objective-C calls is run in a task by a thunk Swift
/// writes, which calls the handler wherever that task finishes. For a
/// nonisolated method that is a cooperative-pool thread, every time. Marking
/// the class `@MainActor` does not move it: the method runs on the main actor
/// but the thunk still resumes off it before calling the handler (measured
/// with Swift 6.4 in the app's Swift 5 mode, ov-787; Swift 6 mode refuses
/// that conformance outright).
///
/// So the two methods here are the completion-handler forms, and each one
/// calls its handler itself, from a `@MainActor` task, after the work it was
/// given has finished. The work is the app's: what to show while it is open,
/// and where a tap goes. They are closures so that this type, which has to be
/// in AgentKit to be tested (the iOS target has no unit tests), does not need
/// to know about the screens they lead to.
public final class NotificationReplies: NSObject, UNUserNotificationCenterDelegate, Sendable {
    /// How to show a banner that arrives while the app is open.
    public typealias Present = @MainActor @Sendable (UNNotification) -> UNNotificationPresentationOptions
    /// What a tap, or an answer button, does. Awaited in full before the
    /// center is told the response was handled.
    public typealias Respond = @MainActor @Sendable (UNNotificationResponse) async -> Void

    private let present: Present
    private let respond: Respond

    public init(present: @escaping Present, respond: @escaping Respond) {
        self.present = present
        self.respond = respond
    }

    public func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        let notification = Handed(notification)
        let reply = Handed(completionHandler)
        Task { @MainActor in
            reply.value(present(notification.value))
        }
    }

    public func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let response = Handed(response)
        let reply = Handed(completionHandler)
        Task { @MainActor in
            await respond(response.value)
            reply.value()
        }
    }
}

/// A value the center handed over, carried into the main actor's task.
///
/// `UNNotification`, `UNNotificationResponse` and the center's handlers are
/// not marked `Sendable`. Each one is handed over once, used only inside the
/// one task it is moved into, and never touched by the caller again, which is
/// the case the checker cannot see.
private struct Handed<Value>: @unchecked Sendable {
    let value: Value
    init(_ value: Value) { self.value = value }
}
