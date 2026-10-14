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
        var agentsSupported: Bool
        var statuses: [String: TaskStatus]
        var expect: [String]

        init(_ c: [String: Any]) throws {
            name = c["name"] as? String ?? ""
            plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: c["plan"] as Any))
            needsYou = c["needs_you"] as? Int ?? 0
            agentsSupported = c["agents_supported"] as? Bool ?? true
            statuses = (c["statuses"] as? [String: String] ?? [:]).compactMapValues { TaskStatus(rawValue: $0) }
            expect = c["expect"] as? [String] ?? []
        }

        var sections: PlanSections {
            PlanSections(plan: plan, needsYou: needsYou, statuses: statuses, agentsSupported: agentsSupported)
        }
    }

    /// One line a row, as the fixture writes them.
    static func outline(_ c: Case) -> [String] {
        let s = c.sections
        var out = ["shown: " + s.shown.map { "\($0)" }.joined(separator: " ")]
        func lane(_ label: String, _ row: PlanSections.LaneRow) {
            var line = "  \(label) \(row.lane.name) | \(PlanWords.status(row.lane))"
            if let agent = row.agent { line += " | agent: \(agent.who)" }
            if row.waitsOnOwner { line += " | owner" }
            if let train = row.landedTrain { line += " | names landed \(train)" }
            out.append(line)
        }
        if s.needsYou > 0 { out.append("needs-you \(s.needsYou)") }
        out.append("landing")
        for card in s.trains {
            out.append("  train \(card.train.label) | \(card.train.subtitle ?? "-") | \(card.phase.sentence) | \(card.held ?? "-")")
            out.append("    ladder: \(PlanLadder.compact(card.phase)) | \(card.phase.explanation)")
            for l in card.lanes {
                out.append("    lane \(l.name) | \(l.note ?? "-")" + (l.holding ? " | holding" : "") + (l.waitsOnOwner ? " | owner" : ""))
            }
        }
        if let empty = s.emptyWords(.landing) { out.append("  empty: \(empty)") }
        for (label, rows) in [("inreview", s.inReview), ("building", s.building)] {
            if rows.isEmpty && label != "building" { continue }
            out.append(label)
            rows.forEach { lane("lane", $0) }
            if let empty = s.emptyWords(label == "building" ? .building : .inReview) { out.append("  empty: \(empty)") }
        }
        out.append("waiting")
        s.waiting.next.forEach { lane("next", $0) }
        s.waiting.notRanked.forEach { lane("notranked", $0) }
        s.waiting.onYou.forEach { lane("onyou", $0) }
        if let empty = s.emptyWords(.waiting) { out.append("  empty: \(empty)") }
        if s.shown.contains(.onLandedTrain) {
            out.append("onlandedtrain (folded: \(PlanSections.Kind.onLandedTrain.startsFolded))")
            s.onLandedTrain.forEach { lane("lane", $0) }
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
                    out.append("    card \(card.glyph) \(card.key)" + (card.lane.map { " | \($0)" } ?? ""))
                }
            }
        }
        out.append("lanetree")
        for g in LaneTree(plan: c.plan, sections: s, locale: en).groups {
            out.append("  group \(g.title) | \(g.detail ?? "-") | \(g.folded ? "folded" : "open")")
            for t in g.trains {
                out.append("    train \(t.label) | \(t.subtitle ?? "-") | \(t.phase)")
                for l in t.lanes {
                    out.append("      lane \(l.name) | \(l.state.isEmpty ? "-" : l.state) | \(l.cards.map(\.key).joined(separator: ","))")
                    for a in l.agents { out.append("        agent \(a.who) | \(a.working ? "working" : "done") | \(a.tokens ?? "-")") }
                }
            }
            for l in g.lanes {
                out.append("    lane \(l.name) | \(l.state) | \(l.cards.map(\.key).joined(separator: ","))")
                for a in l.agents { out.append("      agent \(a.who) | \(a.working ? "working" : "done") | \(a.tokens ?? "-")") }
            }
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
