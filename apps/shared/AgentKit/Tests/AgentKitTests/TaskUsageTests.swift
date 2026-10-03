import Foundation
import Testing

@testable import AgentKit

/// `test/fixtures/task-usage.json`, which Android's `TaskUsageTest` and the
/// CLI's `usage_words` read too (ov-195): what a task's Usage section says.
private struct UsageFixture: Decodable {
    struct Count: Decodable { var n: UInt64; var text: String }
    struct Money: Decodable { var micros: Int64; var text: String }
    struct Span: Decodable { var ms: Int64; var text: String }
    struct Row: Decodable { var title: String; var detail: String }
    struct Case: Decodable {
        var `case`: String
        var usage: TaskUsage
        var empty: Bool
        var tokens: String?
        var tokenDetail: String?
        var cost: String?
        var time: String?
        var rows: [Row]
    }
    var locale: String
    var tokens: [Count]
    var dollars: [Money]
    var durations: [Span]
    var cases: [Case]

    static func load() throws -> UsageFixture {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/task-usage.json"))
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(UsageFixture.self, from: data)
    }
}

struct TaskUsageTests {
    @Test("Token counts, dollars and agent time read as the shared fixture says")
    func numbers() throws {
        let fixture = try UsageFixture.load()
        let locale = Locale(identifier: fixture.locale)
        for c in fixture.tokens {
            #expect(TaskUsageFormat.tokens(c.n, locale: locale) == c.text, "\(c.n)")
        }
        for c in fixture.dollars {
            #expect(TaskUsageFormat.dollars(c.micros, locale: locale) == c.text, "\(c.micros)")
        }
        for c in fixture.durations {
            #expect(TaskUsageFormat.duration(ms: c.ms) == c.text, "\(c.ms)")
        }
    }

    @Test("Each case's lines, provenance and breakdown read as the shared fixture says")
    func cases() throws {
        let fixture = try UsageFixture.load()
        let locale = Locale(identifier: fixture.locale)
        #expect(fixture.cases.count >= 8)
        for each in fixture.cases {
            let t = each.usage.totals
            #expect(t.isEmpty == each.empty, "\(each.case): empty")
            guard !t.isEmpty else {
                #expect(each.usage.rows.isEmpty, "\(each.case)")
                continue
            }
            #expect(TaskUsageFormat.tokensLine(t, locale: locale) == each.tokens, "\(each.case): tokens")
            #expect(TaskUsageFormat.tokenDetail(t, locale: locale) == each.tokenDetail, "\(each.case): detail")
            #expect(TaskUsageFormat.cost(t, locale: locale) == each.cost, "\(each.case): cost")
            #expect(TaskUsageFormat.time(t) == each.time, "\(each.case): time")
            let rows = each.usage.rows.map {
                UsageFixture.Row(
                    title: TaskUsageFormat.title($0), detail: TaskUsageFormat.detail($0, locale: locale))
            }
            #expect(rows.map(\.title) == each.rows.map(\.title), "\(each.case): row titles")
            #expect(rows.map(\.detail) == each.rows.map(\.detail), "\(each.case): row details")
        }
    }

    @Test("The runner's JSON decodes, and the empty state has its sentence")
    func decodesTheRunnersJSON() throws {
        let json = """
            {"task":"t","price_table":"2026-09-25","totals":{"turns":0,"active_ms":0},"by_harness_model":[]}
            """
        let usage = try TaskUsage.decode(Data(json.utf8))
        #expect(usage.totals.isEmpty)
        #expect(TaskUsageFormat.nothingYet == "No agent usage recorded yet.")
    }

    @Test("Another locale's digits and currency, not en_US's")
    func anotherLocale() {
        let de = Locale(identifier: "de_DE")
        #expect(TaskUsageFormat.tokens(1_155_400, locale: de) == "1,2M")
        #expect(TaskUsageFormat.dollars(3_200_000, locale: de).contains("3,20"))
    }
}
