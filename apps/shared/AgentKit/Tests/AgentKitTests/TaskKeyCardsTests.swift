import Foundation
import SwiftUI
import Testing

@testable import AgentKit

/// A task key's card (ov-299): what it says, which keys have one, and that it
/// is built once per read, not once per hover.
@MainActor
struct TaskKeyCardsTests {
    static func board(_ rows: [TaskRow]) -> TaskBoardModel {
        TaskBoardModel.board(from: []).withRows(rows)
    }

    static func row(_ id: String, _ key: String, _ title: String, _ status: TaskStatus = .inProgress, intent: String = "")
        -> TaskRow
    {
        TaskRow(id: id, key: key, title: title, status: status, statusSince: .now, intent: intent)
    }

    static func note(_ kind: TaskNoteKind, _ body: String, at seconds: TimeInterval) -> TaskNoteRow {
        TaskNoteRow(id: UUID().uuidString, kind: kind, actor: "manager", at: Date(timeIntervalSince1970: seconds), body: body)
    }

    @Test("A card is the row's title and status, with the plan's theme and live lane, and its newest written note")
    func cardFromReads() throws {
        let plan = try PlanFixture.seeded()
        let theme = try #require(plan.themes.first { $0.name == "Mac navigation" })
        let task = try #require(theme.cards.first { $0.key == "ov-248" })
        let board = Self.board([Self.row(task.task, "ov-248", "Mac: the jump bar opens anything", intent: "Intent line\nmore")])
        let notes = [
            Self.note(.progress, "Built the jump bar.\nDetails follow.", at: 100),
            Self.note(.statusChange, "moved to in_review", at: 300),
            Self.note(.finding, "  ", at: 400),
        ]
        let cards = TaskKeyCards(runner: "r1", boards: ["w": board], plans: ["w": plan], notes: [task.task: notes])
        let card = try #require(cards.card(for: "ov-248"))
        #expect(card.title == "Mac: the jump bar opens anything")
        #expect(card.status == .inProgress)
        #expect(card.theme == "Mac navigation")
        #expect(card.lane == "integ-8", "the live lane that names it")
        #expect(card.excerpt == "Built the jump bar.", "the newest note a person or agent wrote, first line")
        #expect(card.details == "In Progress · Mac navigation · integ-8")
        #expect(card.accessibilityLabel == "ov-248, Mac: the jump bar opens anything")
    }

    @Test("The CLI's own board and plan, through the app's parsers, give each key its card")
    func fromTheCLI() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let dir = root.appendingPathComponent("test/fixtures/task-key-cards")
        let board = try TaskBoardModel.decode(Data(contentsOf: dir.appendingPathComponent("tasks.json")))
        let plan = try PlanModel.decode(Data(contentsOf: dir.appendingPathComponent("plan.json")))
        let cards = TaskKeyCards(runner: "", boards: ["w": board], plans: ["w": plan])
        #expect(cards.card(for: "bil-1")?.details == "In Progress · Invoices · invoice-totals")
        #expect(cards.card(for: "bil-1")?.excerpt == "Invoices show the sum of their lines.")
        #expect(cards.card(for: "bil-2")?.details == "Needs Decision · Invoices · rounding")
        #expect(cards.card(for: "bil-3")?.details == "Done · Invoices")
        #expect(cards.card(for: "bil-4") == nil)
    }

    @Test("Without a note or a plan, the card says the intent's first line and the status alone")
    func intentExcerpt() throws {
        let board = Self.board([Self.row("t1", "ov-1", "Fix it", .todo, intent: "\n## Why it matters\nSecond line")])
        let card = try #require(TaskKeyCards(runner: "r1", boards: ["w": board]).card(for: "ov-1"))
        #expect(card.excerpt == "Why it matters")
        #expect(card.details == "To Do")
    }

    @Test("A long excerpt is cut at a word with an ellipsis")
    func longExcerpt() {
        let long = String(repeating: "word ", count: 60)
        let cut = TaskKeyCard.excerpt(long)
        #expect(cut.count <= TaskKeyCard.excerptLimit)
        #expect(cut.hasSuffix("word…"))
    }

    @Test("Unknown keys and other runners' links show nothing")
    func unknown() throws {
        let board = Self.board([Self.row("t1", "ov-1", "One")])
        let cards = TaskKeyCards(runner: "r1", boards: ["w": board])
        #expect(cards.card(for: "ov-2") == nil)
        #expect(cards.card(for: try #require(TaskKeyLinks.url(runner: "r2", key: "ov-1"))) == nil)
        #expect(cards.card(for: try #require(TaskKeyLinks.url(runner: "r1", key: "ov-1")))?.title == "One")
        #expect(cards.card(for: try #require(URL(string: "https://example.com/ov-1"))) == nil)

        // A linker shows only the cards of keys it links, on its own runner.
        let index = TaskKeyIndex(runner: "r1", workspaces: [], boards: ["w": board])
        #expect(TaskKeyLinker(index: index, cards: cards, open: { _ in }).card(forKey: "ov-1")?.title == "One")
        let elsewhere = TaskKeyCards(runner: "r2", boards: ["w": board])
        #expect(TaskKeyLinker(index: index, cards: elsewhere, open: { _ in }).card(forKey: "ov-1") == nil)
        #expect(TaskKeyLinker(index: .empty, cards: cards, open: { _ in }).card(forKey: "ov-1") == nil)
    }

    @Test("The cache builds once per read, and again only when a read changes")
    func cache() {
        let cache = TaskKeyCardCache()
        var board = Self.board([Self.row("t1", "ov-1", "One")])
        for _ in 0..<50 { _ = cache.cards(runner: "r1", boards: ["w": board]) }
        #expect(cache.builds == 1, "fifty hovers, one build")
        board = Self.board([Self.row("t1", "ov-1", "One, renamed")])
        #expect(cache.cards(runner: "r1", boards: ["w": board]).card(for: "ov-1")?.title == "One, renamed")
        #expect(cache.builds == 2)
        _ = cache.cards(runner: "r1", boards: ["w": board], notes: ["t1": [Self.note(.comment, "Hi", at: 1)]])
        #expect(cache.builds == 3, "a record read is a new read")
        _ = cache.cards(runner: "r2", boards: ["w": board], notes: ["t1": [Self.note(.comment, "Hi", at: 1)]])
        #expect(cache.builds == 4, "another runner")
    }

    @Test("An accessibility action to open a key names its title")
    func openLabel() {
        let card = TaskKeyCard(key: "ov-1", title: "Fix it", status: .todo)
        #expect(TaskKeyLinks.openLabel("ov-1", card: card) == "Open ov-1, Fix it")
        #expect(TaskKeyLinks.openLabel("ov-1", card: nil) == "Open ov-1")
    }
}

enum PlanFixture {
    /// The seeded board's plan, as the CLI printed it.
    static func seeded() throws -> PlanModel {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-seeded.json"))
        let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: try #require(object["plan"])))
    }
}

extension TaskBoardModel {
    /// The board with these rows in their status's columns.
    fileprivate func withRows(_ rows: [TaskRow]) -> TaskBoardModel {
        TaskBoardModel(columns: Self.order.map { status in
            TaskBoardColumn(status: status, rows: rows.filter { $0.status == status })
        })
    }
}
