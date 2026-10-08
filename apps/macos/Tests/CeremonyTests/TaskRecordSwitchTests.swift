import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Switching tickets beside the board (ov-85): the one leaving keeps its own
/// record while it fades out under the next, and no ticket ever draws
/// another's (ov-65 O2).
@MainActor
struct TaskRecordSwitchTests {
    /// A board of t9, t1 and t5, where each task's record says whose it is,
    /// and t1's read can be made to fail, so a switch can be caught before
    /// the new task's read lands.
    private func store(failing: Set<String> = []) -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            if args.starts(with: ["task", "list"]) {
                return (
                    Data(
                        #"{"tasks":[{"id":"t9","key":"-9","title":"Pick","status":"needs_decision"},{"id":"t1","key":"-1","title":"Other","status":"todo"},{"id":"t5","key":"-5","title":"Third","status":"todo"}]}"#
                            .utf8), nil
                )
            }
            if args.starts(with: ["task", "show"]) {
                let key = args.count > 2 ? args[2] : "?"
                if failing.contains(key) { return (nil, "unreachable") }
                return (
                    Data(
                        #"{"task":{},"notes":[{"id":"n\#(key)","kind":"question","actor":"manager","at":1,"body":"record of \#(key)","extra":{}}],"blocks":[]}"#
                            .utf8), nil
                )
            }
            return (Data(), nil)
        }
        return TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
    }

    private func bodies(_ detail: TaskDetailModel) -> [String] { detail.notes.map(\.body) }

    @Test("The ticket leaving keeps its own record while the next one loads")
    func theLeavingTicketKeepsItsRecord() async throws {
        let store = store(failing: ["-1"])
        await store.readIfNeverRead()
        let t9 = try #require(store.board.rows.first { $0.id == "t9" })
        let t1 = try #require(store.board.rows.first { $0.id == "t1" })
        await store.open(t9)
        #expect(bodies(store.detail(for: "t9")) == ["record of -9"])
        #expect(store.canAnswer("t9"), "a card read for itself can't answer")
        // Switched to t1, whose read hasn't landed (here, never does).
        await store.open(t1)
        #expect(bodies(store.detail(for: "t9")) == ["record of -9"], "the leaving ticket went blank")
        #expect(store.question(for: "t9") != nil, "the leaving ticket lost its question")
        // Kept, it's drawn read-only: no Answer buttons on a question that
        // may have been answered since.
        #expect(!store.canAnswer("t9"), "the leaving ticket's question is still live")
        let offer = TaskCard.offer(row: t9, question: store.question(for: "t9"), canAnswer: store.canAnswer("t9"))
        #expect(offer.map { $0.options.isEmpty && !$0.typed } ?? true, "the leaving ticket offers answers")
        // O2: t1 never shows t9's record or question.
        #expect(store.detail(for: "t1").notes.isEmpty, "drew t9's record under t1")
        #expect(store.question(for: "t1") == nil, "offered t9's question under t1")
    }

    @Test("Each ticket draws only its own record, through several switches")
    func eachTicketDrawsItsOwn() async throws {
        let store = store()
        await store.readIfNeverRead()
        for id in ["t9", "t1", "t5", "t9"] {
            await store.open(try #require(store.board.rows.first { $0.id == id }))
        }
        #expect(bodies(store.detail(for: "t9")) == ["record of -9"])
        #expect(bodies(store.detail(for: "t1")) == ["record of -1"])
        #expect(bodies(store.detail(for: "t5")) == ["record of -5"])
        #expect(store.detail(for: "t404").notes.isEmpty)
        #expect(store.question(for: "t404") == nil)
    }

    /// A held arrow's repeats come faster than the window redraws: each
    /// steps on from the last step, not from the selection the window still
    /// shows, across the navigator's sections, and the walk stops at the end
    /// (live, ov-85: before this, twelve repeats moved the list one row).
    @Test("Repeats step on from the last step, ahead of the window, across sections")
    func repeatsStepOnFromTheLastStep() async throws {
        let store = store()
        await store.readIfNeverRead()
        var stepped: [NavigatorItem] = []
        let heard = TaskBoardView.Heard()
        heard.store = store
        heard.onStep = { stepped.append($0) }
        heard.items = Navigator.items(orchestrator: true, tasks: ["t9", "t1", "t5"], worktrees: ["w1"])
        heard.selected = .orchestrator
        for _ in 0..<6 { _ = heard.step(1) }
        #expect(stepped == [.task("t9"), .task("t1"), .task("t5"), .worktree("w1")], "\(stepped)")
        // The window catches up, and ↑ goes back from there.
        heard.selected = .worktree("w1")
        heard.stepped = nil
        _ = heard.step(-1)
        #expect(stepped.last == .task("t5"))
        // Without the window, a task is opened on the board itself.
        var glanced: [String] = []
        store.onGlance = { glanced.append($0.id) }
        heard.onStep = nil
        heard.selected = .task("t9")
        heard.stepped = nil
        _ = heard.step(1)
        #expect(glanced == ["t1"])
    }
}
