import AgentKit
import Foundation
import Testing
import UserNotifications

@testable import Far_Cooler

/// Notifications about tasks on the Mac (ov-94): the runner's `notice` line,
/// posted under the notice's own id so a newer one replaces the older, with a
/// decision's answers as buttons, and nothing posted locally that the relay
/// will deliver anyway.
@MainActor
struct TaskNoticeTests {
    static let line = #"""
        {"kind":"notice","notice_id":"t:r-1:ov-90","event":"decision","level":"time-sensitive","title":"ov-90 Wake the agent on an answer","body":"Needs your decision · Which?","task":"ov-90","runner":"r-1","workspace":"Main","options":["pdfkit","pdf.js"]}
        """#

    static func notice(_ event: String = "review", body: String = "Moved to In Review", options: [String] = [])
        -> NoticeEvent
    {
        NoticeEvent(
            noticeId: "t:r-1:ov-90", event: event, level: event == "decision" ? "time-sensitive" : "active",
            title: "ov-90 Wake the agent on an answer", body: body, task: "ov-90", runner: "r-1",
            options: options)
    }

    @Test("The runner's notice line reaches the notifier, whole")
    func theLineIsDispatched() {
        var heard: [NoticeEvent] = []
        EventStream.dispatch(Data(Self.line.utf8), decoder: JSONDecoder(), onNotice: { heard.append($0) })
        #expect(heard == [Self.notice("decision", body: "Needs your decision · Which?", options: ["pdfkit", "pdf.js"])])
    }

    @Test("A newer notice about a task replaces the older: one identifier, one thread")
    func aNewerNoticeReplacesTheOlder() {
        let review = Notifier.request(for: Self.notice(), target: "")
        let done = Notifier.request(for: Self.notice("done", body: "Done"), target: "")
        #expect(review.identifier == "t:r-1:ov-90")
        #expect(done.identifier == review.identifier)
        #expect(review.content.threadIdentifier == "t:r-1:ov-90")
        #expect(review.content.title == "ov-90 Wake the agent on an answer")
        #expect(review.content.body == "Moved to In Review")
        #expect(review.content.interruptionLevel == .active)
        #expect(done.content.body == "Done")
        // A tap or an answer on it is read by the push's own code.
        #expect(TaskNotice(userInfo: review.content.userInfo)?.key == "ov-90")
        #expect(review.content.userInfo["target"] as? String == "")
        #expect(review.content.categoryIdentifier.isEmpty, "only a decision has buttons")
    }

    @Test("A decision files under its options' category, and a button sends that option")
    func aDecisionHasItsAnswers() {
        let options = ["pdfkit", "pdf.js"]
        let request = Notifier.request(for: Self.notice("decision", options: options), target: "studio")
        #expect(request.content.categoryIdentifier == TaskDecisionActions.category(for: options))
        #expect(request.content.interruptionLevel == .timeSensitive)
        #expect(request.content.userInfo["target"] as? String == "studio")
        let notice = TaskNotice(userInfo: request.content.userInfo)
        #expect(notice?.options == options)
        #expect(TaskDecisionActions.answer(action: "answer.0", options: notice?.options ?? [], typed: nil) == "pdfkit")
    }

    @Test("Nothing is posted here that the relay will deliver: a paired runner and a registered Mac")
    func postsOnlyWhatNoPushWillBring() {
        #expect(Notifier.postsLocally(pushPaired: false, registered: false))
        #expect(Notifier.postsLocally(pushPaired: false, registered: true))
        #expect(Notifier.postsLocally(pushPaired: true, registered: false))
        #expect(!Notifier.postsLocally(pushPaired: true, registered: true))
    }

    @Test("A task's thread is never read as a terminal on screen")
    func aTaskThreadIsNoTerminal() {
        #expect(Notifier.terminalID(userInfo: [:], thread: "t:r-1:ov-90") == nil)
        #expect(Notifier.terminalID(userInfo: ["kind": "task", "task": "ov-90"], thread: "t:r-1:ov-90") == nil)
        #expect(Notifier.terminalID(userInfo: ["terminal": "term-1"], thread: "x") == "term-1")
        #expect(Notifier.terminalID(userInfo: [:], thread: "term-2") == "term-2")
    }

    @Test("An agent on a task is told about through its task, once the runner sends task notices")
    func anAgentOnATaskFoldsIntoIt() throws {
        func pane(_ id: String) -> Terminal {
            Terminal(id: id, short: id, title: "claude", preset: "claude", state: "running", epoch: 0)
        }
        func lane(openTasks: Int, checkout: Bool = false) throws -> Worktree {
            let tasks = (0..<openTasks).map { #"{"id":"t-\#($0)","key":"ov-\#($0)","title":"T","status":"in_progress"}"# }
            let json = #"{"id":"w-1","short":"w1","task":"lane","branch":"b","worktree":"/tmp/w","state":"active","is_main_checkout":\#(checkout),"open_tasks":[\#(tasks.joined(separator: ","))],"terminals":[]}"#
            return try JSONDecoder().decode(Worktree.self, from: Data(json.utf8))
        }
        var agent = pane("a1")
        agent.taskId = "task-1"
        var orchestrator = pane("o1")
        orchestrator.taskId = "task-1"
        orchestrator.role = "orchestrator"
        let loose = pane("l1")
        let bare = try lane(openTasks: 0)
        #expect(Notifier.foldsIntoTask(agent, in: bare, runnerSendsNotices: true))
        #expect(!Notifier.foldsIntoTask(agent, in: bare, runnerSendsNotices: false), "an older runner sends none")
        #expect(!Notifier.foldsIntoTask(orchestrator, in: bare, runnerSendsNotices: true))
        #expect(!Notifier.foldsIntoTask(loose, in: bare, runnerSendsNotices: true))
        // Opened by hand in a lane with one open task: the runner folds it,
        // so its banner is the task's too (ov-107). Not in the main checkout.
        #expect(Notifier.foldsIntoTask(loose, in: try lane(openTasks: 1), runnerSendsNotices: true))
        #expect(!Notifier.foldsIntoTask(loose, in: try lane(openTasks: 2), runnerSendsNotices: true))
        #expect(!Notifier.foldsIntoTask(loose, in: try lane(openTasks: 1, checkout: true), runnerSendsNotices: true))
    }
}
