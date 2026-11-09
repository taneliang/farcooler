import Foundation
import Testing

@testable import AgentKit

/// The Plan's glance band and sidebar lists (ov-503) against
/// `test/fixtures/plan-sections-cases.json`, which Android's
/// `PlanSectionsCasesTest` reads too. The inputs are `plan --json` shapes as
/// the runner sends them, decoded by the parser the apps read them with; the
/// expected outlines are lowercased, since iOS titles in title case and
/// Android in sentence case. The lines before "glance now" (the sections, the
/// grid) are the Rust core's, and its test checks them
/// (`crates/client/src/plan_view/tests.rs`, ov-1016); the band and the trees
/// still read its view here, so these check from there on.
struct PlanSectionsCasesTests {
    static let en = Locale(identifier: "en_US")

    static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root
    }

    static func cases() throws -> [[String: Any]] {
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-sections-cases.json"))
        let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        return try #require(object["cases"] as? [[String: Any]])
    }

    struct Case {
        var name: String
        var plan: PlanModel
        var needsYou: Int
        var statuses: [String: TaskStatus]
        var expect: [String]

        init(_ c: [String: Any]) throws {
            name = c["name"] as? String ?? ""
            plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: c["plan"] as Any))
            needsYou = c["needs_you"] as? Int ?? 0
            statuses = (c["statuses"] as? [String: String] ?? [:]).compactMapValues { TaskStatus(rawValue: $0) }
            expect = c["expect"] as? [String] ?? []
        }

        /// The Rust core's view of the case, through the bridge the apps call.
        var view: PlanView? {
            PlanView.build(plan: plan, needsYou: needsYou, statuses: statuses)
        }
    }

    /// One line a row, as the fixture writes them, from the glance on.
    static func outline(_ c: Case) throws -> [String] {
        let view = try #require(c.view, "\(c.name): the bridge built no view")
        var out: [String] = []
        let glance = PlanGlanceSentence(view: view)
        out.append("glance now: \(glance.nowLine)")
        out.append("glance next: \(glance.nextLine)")
        out.append("glance text: \(glance.text)")

        for filter in OneTreeFilter.allCases {
            out.append("themetree \(filter)")
            for theme in ThemeTree(plan: c.plan, filter: filter, statuses: c.statuses).themes {
                out.append("  theme \(theme.name) | \(theme.progress)")
                for card in theme.cards {
                    out.append("    card \(card.glyph) \(card.key)" + (card.stage.map { " | \($0)" } ?? ""))
                }
            }
        }
        out.append("tickettree")
        for g in TicketTree(plan: c.plan, view: view, locale: en).groups {
            out.append("  group \(g.title) | \(g.detail ?? "-") | \(g.folded ? "folded" : "open")")
            func ticketLines(_ tickets: [TicketTree.TicketNode], indent: String) {
                for t in tickets {
                    out.append("\(indent)ticket \(t.key) | \(t.state.isEmpty ? "-" : t.state)")
                    for a in t.agents { out.append("\(indent)  agent \(a.who) | \(a.working ? "working" : "done") | \(a.tokens ?? "-")") }
                }
            }
            for l in g.landings {
                out.append("    landing \(l.label) | \(l.subtitle ?? "-") | \(l.phase)")
                ticketLines(l.tickets, indent: "      ")
            }
            ticketLines(g.tickets, indent: "    ")
        }
        // Lowercased, and without trailing spaces, which a row with an empty last column leaves.
        return out.map { line in
            var line = line.lowercased()
            while line.hasSuffix(" ") { line.removeLast() }
            return line
        }
    }

    /// The fixture's lines from the glance on: the ones before it are Rust's.
    static func native(_ expect: [String]) -> [String] {
        Array(expect.drop { !$0.hasPrefix("glance now:") })
    }

    @Test("the glance and the lists are the fixture's, case by case, read from the Rust core's view")
    func cases() throws {
        let all = try Self.cases()
        #expect(all.count >= 10)
        if let path = ProcessInfo.processInfo.environment["PLAN_SECTIONS_DUMP"] {
            var dump: [String: [String]] = [:]
            for c in all { let c = try Case(c); dump[c.name] = try Self.outline(c) }
            try JSONSerialization.data(withJSONObject: dump, options: [.prettyPrinted, .sortedKeys]).write(to: URL(fileURLWithPath: path))
            return
        }
        for raw in all {
            let c = try Case(raw)
            #expect(!Self.native(c.expect).isEmpty, "\(c.name) has an expected outline from the glance on")
            #expect(try Self.outline(c) == Self.native(c.expect), "\(c.name)")
        }
    }
}
