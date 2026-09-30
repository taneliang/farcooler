import AgentKit
import AppKit
import Combine
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// A card in Needs Decision answers its question in place (spec §2.5): the
/// options the question offered, as buttons.
///
/// Held to `TaskCard.offer`, which is all the card's question view draws
/// from, rather than to the drawn buttons: SwiftUI's buttons are not
/// `NSButton`s, and an unshown `NSHostingView` answers no accessibility
/// children, so a search of a hosted card finds no buttons either way.
@MainActor
struct TaskCardTests {
    private static func row(_ status: TaskStatus) -> TaskRow {
        TaskRow(id: "t9", key: "-9", title: "Pick a store", status: status, statusSince: .now)
    }

    private static func question(_ options: [String]) -> TaskQuestion {
        TaskQuestion(id: "q1", body: "Which store?", options: options)
    }

    @Test("A card in Needs Decision shows its question's options as buttons")
    func aCardInNeedsDecisionShowsItsQuestionsOptionsAsButtons() throws {
        let offer = try #require(
            TaskCard.offer(
                row: Self.row(.needsDecision), question: Self.question(["SQLite", "Postgres"]),
                canAnswer: true))
        #expect(offer.buttons == ["SQLite", "Postgres"])
        #expect(offer.more.isEmpty)
        #expect(!offer.typed)

        // Past three, the rest go in the menu.
        let many = try #require(
            TaskCard.offer(
                row: Self.row(.needsDecision), question: Self.question(["a", "b", "c", "d"]),
                canAnswer: true))
        #expect(many.buttons == ["a", "b", "c"])
        #expect(many.more == ["d"])

        // None offered: Answer…
        let open = try #require(
            TaskCard.offer(row: Self.row(.needsDecision), question: Self.question([]), canAnswer: true))
        #expect(open.buttons.isEmpty && open.typed)

        // Moved on since: the question is history, and the card offers nothing.
        #expect(
            TaskCard.offer(
                row: Self.row(.inProgress), question: Self.question(["SQLite"]), canAnswer: true)
                == nil)

        // A read-scoped connection sees the question, with nothing to press.
        let read = try #require(
            TaskCard.offer(
                row: Self.row(.needsDecision), question: Self.question(["SQLite"]), canAnswer: false))
        #expect(read.buttons.isEmpty && read.more.isEmpty && !read.typed)
    }

    /// **A half-typed answer survives another task's write.** The daemon
    /// announces every task write, a progress note on another card included,
    /// and the board re-reads the open card for each. That re-read used to
    /// open the card afresh: its question went nil for the round trip, which
    /// tore down the answer field and the text in it. Now the card is read
    /// again in place, its question stays on screen throughout, and the
    /// draft is the store's, kept by question.
    @Test("A draft survives an unrelated task's write")
    func aDraftSurvivesAnUnrelatedTasksWrite() async throws {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            if args.starts(with: ["task", "list"]) {
                return (
                    Data(
                        #"{"tasks":[{"id":"t9","key":"-9","title":"Pick","status":"needs_decision"},{"id":"t1","key":"-1","title":"Other","status":"in_progress"}]}"#
                            .utf8), nil
                )
            }
            if args.starts(with: ["task", "show"]) {
                return (
                    Data(
                        #"{"task":{},"notes":[{"id":"q1","kind":"question","actor":"manager","at":1,"body":"Which store?","extra":{}}],"blocks":[]}"#
                            .utf8), nil
                )
            }
            return (Data(), nil)
        }
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        await store.readIfNeverRead()
        let row = try #require(store.board.rows.first { $0.id == "t9" })
        await store.open(row)
        let question = try #require(store.question)
        store.setDraft("Postgres, because", for: question)

        var seen: [TaskQuestion?] = []
        let watching = store.$question.dropFirst().sink { seen.append($0) }
        defer { watching.cancel() }
        // A write to the other task, announced for this board.
        client.boardMoved(TaskEvent(repository: "r", actor: "manager"))
        await store.reloadIfMoved()

        #expect(!seen.contains { $0 == nil }, "the question left the card mid-read: \(seen)")
        #expect(store.question == question)
        #expect(store.draft(for: question) == "Postgres, because")
        #expect(store.opened?.id == "t9")
    }

    /// **A column never draws another task's record.** `detail` and `question`
    /// are the store's single slots, and the column of a task just switched to
    /// renders once before `open` blanks them. Asked for through the task's id,
    /// the slots are empty for any task but the one they were read for.
    @Test("The store's record and question are shown only for the task they were read for")
    func theRecordIsShownOnlyForItsOwnTask() async throws {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            if args.starts(with: ["task", "list"]) {
                return (
                    Data(
                        #"{"tasks":[{"id":"t9","key":"-9","title":"Pick","status":"needs_decision"},{"id":"t1","key":"-1","title":"Other","status":"needs_decision"}]}"#
                            .utf8), nil
                )
            }
            if args.starts(with: ["task", "show"]) {
                return (
                    Data(
                        #"{"task":{},"notes":[{"id":"q1","kind":"question","actor":"manager","at":1,"body":"Which store?","extra":{}}],"blocks":[]}"#
                            .utf8), nil
                )
            }
            return (Data(), nil)
        }
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        await store.readIfNeverRead()
        let first = try #require(store.board.rows.first { $0.id == "t9" })
        await store.open(first)
        #expect(!store.detail(for: "t9").notes.isEmpty)
        #expect(store.question(for: "t9") != nil)
        // The other task's first frame, before `open` has run for it.
        #expect(store.detail(for: "t1").notes.isEmpty, "drew t9's notes under t1")
        #expect(store.question(for: "t1") == nil, "offered t9's question under t1")
    }
}
