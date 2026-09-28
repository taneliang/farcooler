import AgentKit
import AppKit
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
}
