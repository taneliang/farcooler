import Foundation
import Testing

@testable import AgentKit

/// The cases both phones' trees and strips must agree on (ov-300 review 7):
/// `test/fixtures/one-tree-cases.json`, read here and by Android's
/// `OneTreeCasesTest`. Inputs are in the wire's own shapes (a board's
/// `tasks`, `plan --json`, the fleet's worktrees, `page.list`, `needs_you`
/// items), decoded by the parsers the app reads them with; the expected
/// outline and strip are lowercased, since iOS titles in title case and
/// Android in sentence case.
struct PhoneTreeCasesTests {
    static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root
    }

    static func fixture() throws -> [String: Any] {
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/one-tree-cases.json"))
        return try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    static func data(_ object: Any) throws -> Data { try JSONSerialization.data(withJSONObject: object) }

    /// One line a node, as the fixture writes them.
    static func outline(_ nodes: [OneTreeNode], depth: Int = 0) -> [String] {
        nodes.flatMap { node -> [String] in
            var line = String(repeating: "  ", count: depth) + String(describing: node.kind).lowercased() + " "
            if !node.key.isEmpty { line += node.key + " " }
            line += node.title
            for extra in [node.detail, node.caption, node.also] where !extra.isEmpty { line += " · " + extra }
            if node.showsDot(expanded: false) { line += " •" }
            return [line.lowercased()] + outline(node.children, depth: depth + 1)
        }
    }

    static func filter(_ word: String) -> OneTreeFilter {
        switch word {
        case "in_review": .inReview
        case "all": .all
        default: .open
        }
    }

    @Test("the tree's outline is the fixture's, case by case")
    func outlines() throws {
        let cases = try #require(try Self.fixture()["cases"] as? [[String: Any]])
        #expect(!cases.isEmpty)
        for c in cases {
            let ws = try #require(c["workspace"] as? [String: Any])
            let summary = WorkspaceSummary(
                id: ws["id"] as? String ?? "", name: ws["name"] as? String ?? "", taskPrefix: "", isMain: false, ordinal: 0,
                repository: ws["repository"] as? String)
            let board = try TaskBoardModel.decode(Self.data(c["board"] as Any))
            let plan = try PlanModel.decode(Self.data(c["plan"] as Any))
            let worktrees = try JSONDecoder().decode([Worktree].self, from: Self.data(c["worktrees"] as Any))
            let pages = try BoardPageList.decode(Self.data(c["pages"] as Any)).pages
            let items = try JSONDecoder().decode([NeedsYouItem].self, from: Self.data(c["items"] as Any))
            let tree = OneTree.build(
                PhoneTree.input(
                    summary: summary, board: board, plan: plan, worktrees: worktrees, pages: pages, items: items,
                    filter: Self.filter(c["filter"] as? String ?? ""), needsYouCount: 0))
            let root = PhoneTree.root(tree)
            let actual = Self.outline(root.work + root.below)
            let expected = c["outline"] as? [String] ?? []
            #expect(actual == expected, "\(c["name"] ?? ""):\n\(actual.joined(separator: "\n"))")
        }
    }

    @Test("the strip's parts, state, line and spoken label are the fixture's")
    func strips() throws {
        let strips = try #require(try Self.fixture()["strips"] as? [[String: Any]])
        #expect(!strips.isEmpty)
        for s in strips {
            let plan = try PlanModel.decode(Self.data(s["plan"] as Any))
            let terminal = try (s["orchestrator"] as? [String: Any]).map {
                try JSONDecoder().decode(Terminal.self, from: Self.data($0))
            }
            let state = PhoneTree.orchestrator(terminal)
            let strip = PlanStrip(
                plan: plan, needsYou: s["needs_you"] as? Int ?? 0, orchestrator: state, line: PhoneTree.line(terminal, state: state))
            let name = s["name"] as? String ?? ""
            #expect(strip.parts.map { $0.lowercased() } == (s["parts"] as? [String] ?? []), "\(name): \(strip.parts)")
            #expect(strip.orchestrator.word.lowercased() == s["state"] as? String, "\(name): \(strip.orchestrator.word)")
            #expect(strip.line == s["line"] as? String, "\(name): \(strip.line ?? "nil")")
            #expect(strip.accessibilityLabel.lowercased() == s["label"] as? String, "\(name): \(strip.accessibilityLabel)")
        }
    }
}
