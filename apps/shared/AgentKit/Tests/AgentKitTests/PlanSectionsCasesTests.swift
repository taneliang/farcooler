import Foundation
import Testing

@testable import AgentKit

/// The Plan's sections, glance band, themes grid and sidebar lists (ov-503)
/// against `test/fixtures/plan-sections-cases.json`, which Android's
/// `PlanSectionsCasesTest` reads too. The inputs are `plan --json` shapes as
/// the runner sends them, decoded by the parser the apps read them with; the
/// expected outlines are lowercased, since iOS titles in title case and
/// Android in sentence case.
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

        var sections: PlanSections {
            PlanSections(plan: plan, needsYou: needsYou, statuses: statuses)
        }
    }

    /// One line a row, as the fixture writes them.
    static func outline(_ c: Case) -> [String] {
        let s = c.sections
        var out = ["shown: " + s.shown.map { "\($0)" }.joined(separator: " ")]
        func ticket(_ label: String, _ row: PlanSections.TicketRow) {
            var line = "  \(label) \(row.line.key) | \(PlanWords.status(row.line))"
            if let agent = row.agent { line += " | agent: \(agent.who)" }
            if row.waitsOnOwner { line += " | owner" }
            if let landing = row.landedLanding { line += " | names landed \(landing)" }
            out.append(line)
        }
        if s.needsYou > 0 { out.append("needs-you \(s.needsYou)") }
        out.append("landing")
        for card in s.landings {
            out.append("  landing \(card.landing.label) | \(card.landing.subtitle ?? "-") | \(card.phase.sentence)")
            out.append("    ladder: \(PlanLadder.compact(card.phase)) | \(card.phase.explanation)")
            for t in card.tickets {
                out.append("    ticket \(t.key) | \(t.note ?? "-")" + (t.waitsOnOwner ? " | owner" : ""))
            }
        }
        if let empty = s.emptyWords(.landing) { out.append("  empty: \(empty)") }
        for (label, rows) in [("inreview", s.inReview), ("building", s.building)] {
            if rows.isEmpty && label != "building" { continue }
            out.append(label)
            rows.forEach { ticket("ticket", $0) }
            if let empty = s.emptyWords(label == "building" ? .building : .inReview) { out.append("  empty: \(empty)") }
        }
        out.append("waiting")
        s.waiting.next.forEach { ticket("next", $0) }
        s.waiting.notRanked.forEach { ticket("notranked", $0) }
        s.waiting.onYou.forEach { ticket("onyou", $0) }
        if let empty = s.emptyWords(.waiting) { out.append("  empty: \(empty)") }
        if s.shown.contains(.onLandedLanding) {
            out.append("onlandedlanding (folded: \(PlanSections.Kind.onLandedLanding.startsFolded))")
            s.onLandedLanding.forEach { ticket("ticket", $0) }
        }
        if s.shown.contains(.agents) {
            out.append("agents")
            out.append("  header: \(s.fleet.header(locale: en))")
            for a in s.fleet.agents {
                out.append(
                    "  \(a.working ? "working" : "idle") \(a.who) | \(a.worksFor) | \(a.state(now: s.now)) | \(a.tokens(locale: en) ?? "-") | \(a.cost(locale: en) ?? "-")")
            }
        }
        out.append("themes")
        for t in s.themes.tiles {
            out.append("  tile \(t.name) | \(t.progress) | \(t.isMoving ? "moving" : "still") | ask: \(t.ownerAsk ?? "-")")
            t.cards.forEach { out.append("    card \($0.glyph) \($0.key)") }
        }
        if let title = s.themes.finishedTitle, let words = s.themes.finishedWords {
            out.append("  \(title) | \(words): " + s.themes.finished.map(\.name).joined(separator: ", "))
        }
        let glance = PlanGlanceSentence(sections: s)
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
        for g in TicketTree(plan: c.plan, sections: s, locale: en).groups {
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

    @Test("the sections, glance, grid and lists are the fixture's, case by case")
    func cases() throws {
        let all = try Self.cases()
        #expect(all.count >= 10)
        if let path = ProcessInfo.processInfo.environment["PLAN_SECTIONS_DUMP"] {
            var dump: [String: [String]] = [:]
            for c in all { let c = try Case(c); dump[c.name] = Self.outline(c) }
            try JSONSerialization.data(withJSONObject: dump, options: [.prettyPrinted, .sortedKeys]).write(to: URL(fileURLWithPath: path))
            return
        }
        for raw in all {
            let c = try Case(raw)
            #expect(!c.expect.isEmpty, "\(c.name) has an expected outline")
            #expect(Self.outline(c) == c.expect, "\(c.name)")
        }
    }
}
