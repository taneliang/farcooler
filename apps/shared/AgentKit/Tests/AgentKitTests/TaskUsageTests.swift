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
    struct Usage: Decodable { var totals: TaskSpend }
    struct Case: Decodable {
        var `case`: String
        var usage: Usage
        var empty: Bool
        var tokens: String?
        var tokenDetail: String?
        var cost: String?
        var time: String?
        var rows: [Row]
    }
    struct Words: Decodable { var nothingYet, needsUpdate, couldntRead, apiEquivalent: String }
    var locale: String
    var words: Words
    var tokens: [Count]
    var dollars: [Money]
    var durations: [Span]
    var cases: [Case]

    static func data(_ name: String) throws -> Data {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try Data(contentsOf: root.appendingPathComponent("test/fixtures/\(name)"))
    }

    static func load() throws -> UsageFixture {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(UsageFixture.self, from: data("task-usage.json"))
    }

    /// `test/fixtures/task-usage-shown.json`: the client core's own answer
    /// for each case, as the Usage section decodes it.
    static func shown() throws -> [String: TaskUsage] {
        let raw = try JSONSerialization.jsonObject(with: data("task-usage-shown.json")) as? [[String: Any]] ?? []
        var out: [String: TaskUsage] = [:]
        for entry in raw {
            let bytes = try JSONSerialization.data(withJSONObject: entry["shown"] ?? [:])
            out[entry["case"] as? String ?? ""] = try TaskUsage.decode(bytes)
        }
        return out
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

    @Test("The client core's answer for each case decodes to the lines the shared fixture says")
    func cases() throws {
        let fixture = try UsageFixture.load()
        let shown = try UsageFixture.shown()
        #expect(fixture.cases.count >= 9)
        for each in fixture.cases {
            let said = try #require(shown[each.case], "\(each.case) is in task-usage-shown.json").said
            #expect((said.nothing != nil) == each.empty, "\(each.case): empty")
            guard said.nothing == nil else { continue }
            #expect(said.tokens == each.tokens, "\(each.case): tokens")
            #expect(said.tokenDetail == each.tokenDetail, "\(each.case): detail")
            #expect(said.cost == each.cost, "\(each.case): cost")
            #expect(said.time == each.time, "\(each.case): time")
            #expect(said.rows.map(\.title) == each.rows.map { $0.title.replacingOccurrences(of: "claude · claude-opus-5", with: "claude · Claude Opus 5").replacingOccurrences(of: "claude · claude-sonnet-5", with: "claude · Claude Sonnet 5") }, "\(each.case): row titles")
            #expect(said.rows.map(\.detail) == each.rows.map(\.detail), "\(each.case): row details")
        }
    }

    @Test("Today's spend in the title panel says the fixture's cost and tokens")
    func totalsLines() throws {
        let fixture = try UsageFixture.load()
        let locale = Locale(identifier: fixture.locale)
        for each in fixture.cases where !each.empty {
            #expect(TaskUsageFormat.tokensLine(each.usage.totals, locale: locale) == each.tokens, "\(each.case): tokens")
            #expect(TaskUsageFormat.cost(each.usage.totals, locale: locale) == each.cost, "\(each.case): cost")
        }
    }

    @Test("The section's sentences are the shared fixture's")
    func sentences() throws {
        let w = try UsageFixture.load().words
        #expect(try UsageFixture.shown()["nothing recorded"]?.said.nothing == w.nothingYet)
        #expect(TaskUsageFormat.needsUpdate == w.needsUpdate)
        #expect(TaskUsageFormat.couldntRead == w.couldntRead)
        #expect(TaskUsageFormat.apiEquivalent == w.apiEquivalent)
        // The button is this platform's own, in Apple's title case.
        #expect(TaskUsageFormat.tryAgain == "Try Again")
    }

    @Test("An older runner needs an update; a read that didn't come back failed")
    func states() throws {
        let usage = try #require(UsageFixture.shown()["nothing recorded"])
        #expect(TaskUsageState.after(read: nil, runnerCan: false) == .needsUpdate)
        #expect(TaskUsageState.after(read: usage, runnerCan: false) == .needsUpdate)
        #expect(TaskUsageState.after(read: nil, runnerCan: true) == .failed)
        #expect(TaskUsageState.after(read: nil, runnerCan: nil) == .failed)
        #expect(TaskUsageState.after(read: usage, runnerCan: nil) == .loaded(usage))
    }

    @Test("Another locale's digits and currency, not en_US's")
    func anotherLocale() {
        let de = Locale(identifier: "de_DE")
        #expect(TaskUsageFormat.tokens(1_155_400, locale: de) == "1,2M")
        #expect(TaskUsageFormat.dollars(3_200_000, locale: de).contains("3,20"))
    }
}
