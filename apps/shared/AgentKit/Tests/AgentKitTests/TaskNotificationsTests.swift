import Foundation
import Testing

@testable import AgentKit

/// Notifications about tasks (ov-94): the five classes and their defaults,
/// what a push about a task carries, and the decision's answer buttons. Shared
/// by the Mac and the phone, so the two can't disagree about which classes are
/// on or which button sends which answer.
struct TaskNotificationsTests {
    /// A scratch defaults domain, so no test reads another's switches.
    static func scratch() -> UserDefaults {
        let name = "TaskNotificationsTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    @Test("Needs a Decision, Ready for Review and Blocked are on until turned off; Done and New Task aren't")
    func theDefaults() {
        let defaults = Self.scratch()
        #expect(TaskNoticeEvent.allCases.map(\.rawValue) == ["decision", "review", "blocked", "done", "new"])
        #expect(TaskNoticeEvent.allCases.map(\.title)
            == ["Needs a Decision", "Ready for Review", "Blocked", "Done", "New Task"])
        #expect(TaskNotifications.events(master: true, in: defaults) == ["decision", "review", "blocked"])
        defaults.set(false, forKey: TaskNoticeEvent.review.defaultsKey)
        defaults.set(true, forKey: TaskNoticeEvent.done.defaultsKey)
        #expect(TaskNotifications.events(master: true, in: defaults) == ["decision", "blocked", "done"])
        // The master switch is "all notifications from this app", and the
        // relay hears it as no classes at all.
        #expect(TaskNotifications.events(master: false, in: defaults) == [])
        #expect(TaskNotifications.wants("done", master: true, in: defaults))
        #expect(!TaskNotifications.wants("review", master: true, in: defaults))
        #expect(!TaskNotifications.wants("done", master: false, in: defaults))
        #expect(!TaskNotifications.wants("invented", master: true, in: defaults))
    }

    @Test("A task push is read for its task, runner, class, id and options")
    func aTaskPushIsRead() {
        let info: [AnyHashable: Any] = [
            "kind": "task", "task": "ov-90", "runner": "r-1", "event": "decision",
            "noticeId": "t:r-1:ov-90", "options": ["pdfkit", "pdf.js"],
        ]
        let notice = TaskNotice(userInfo: info)
        #expect(notice == TaskNotice(
            key: "ov-90", runner: "r-1", event: .decision, noticeId: "t:r-1:ov-90", options: ["pdfkit", "pdf.js"]))
        // Round-trips through what a local post files.
        #expect(notice.flatMap { TaskNotice(userInfo: $0.userInfo) } == notice)
        // Android's spelling: the options as a JSON string.
        #expect(TaskNotice(userInfo: ["kind": "task", "task": "ov-1", "options": "[\"A\",\"B\"]"])?.options == ["A", "B"])
        // Not a task notice.
        #expect(TaskNotice(userInfo: ["kind": "decision", "task": "ov-1"]) == nil)
        #expect(TaskNotice(userInfo: ["kind": "task", "task": ""]) == nil)
    }

    @Test("A tap on a task notice opens its task, never a terminal named after its thread")
    func aTapOnATaskNotice() {
        #expect(
            PushTap(userInfo: ["kind": "task", "task": "ov-90", "runner": "r-1", "event": "review"], thread: "t:r-1:ov-90")
                == .task(DecisionPush(key: "ov-90", runner: "r-1")))
        // A task's thread is not a terminal id, whatever arrives with it.
        #expect(PushTap(userInfo: [:], thread: "t:r-1:ov-90") == nil)
        #expect(PushTap(userInfo: ["kind": "task"], thread: "t:r-1:ov-90") == nil)
        #expect(PushTap(userInfo: [:], thread: "a:term-1") == nil)
    }

    @Test("A decision's buttons are its options and Answer…, under a category named for them")
    func theDecisionButtons() {
        let options = ["pdfkit", "pdf.js"]
        let category = TaskDecisionActions.category(for: options)
        #expect(category.hasPrefix("decision."))
        #expect(category == TaskDecisionActions.category(for: options), "stable")
        #expect(category != TaskDecisionActions.category(for: ["pdf.js", "pdfkit"]), "the order is part of it")
        #expect(category != TaskDecisionActions.category(for: ["pdfkit pdf.js"]))
        #expect(TaskDecisionActions.actions(for: options).map(\.id) == ["answer.0", "answer.1", "answer.text"])
        #expect(TaskDecisionActions.actions(for: options).map(\.title) == ["pdfkit", "pdf.js", "Answer…"])
        // At most three option buttons, as the Needs You rows draw.
        #expect(TaskDecisionActions.actions(for: ["a", "b", "c", "d"]).count == 4)

        #expect(TaskDecisionActions.answer(action: "answer.1", options: options, typed: nil) == "pdf.js")
        #expect(TaskDecisionActions.answer(action: "answer.text", options: options, typed: "  qpdf \n") == "qpdf")
        #expect(TaskDecisionActions.answer(action: "answer.text", options: options, typed: "   ") == nil)
        #expect(TaskDecisionActions.answer(action: "answer.7", options: options, typed: nil) == nil)
        #expect(TaskDecisionActions.answer(action: "com.apple.UNNotificationDefaultActionIdentifier", options: options, typed: nil) == nil)
    }
}

/// What a device registers with: its task classes beside the done switch, and
/// an empty list sent as one rather than left out.
struct TaskNotificationsRegistrationTests {
    static func payload(_ events: [String]?) -> [String: Any] {
        Account.registration(
            pushToken: "t", platform: "apns", label: "Phone", environment: "production",
            liveActivityStartToken: nil, notifyOnDone: true, notifyEvents: events, pulseToken: nil)
    }

    @Test("Registration sends the classes that are on, and an empty list when every one is off")
    func registrationSendsTheClasses() {
        let defaults = TaskNotificationsTests.scratch()
        defaults.set(false, forKey: TaskNoticeEvent.blocked.defaultsKey)
        #expect(Self.payload(TaskNotifications.events(master: true, in: defaults))["notifyEvents"] as? [String]
            == ["decision", "review"])
        #expect(Self.payload(TaskNotifications.events(master: false, in: defaults))["notifyEvents"] as? [String] == [])
        // An app that never set the closure says nothing, and the relay keeps
        // what it has.
        #expect(Self.payload(nil)["notifyEvents"] == nil)
    }
}
