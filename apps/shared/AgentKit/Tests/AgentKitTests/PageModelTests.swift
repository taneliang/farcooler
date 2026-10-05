import Foundation
import Testing

@testable import AgentKit

/// Pages as the apps read them (ov-269, ov-284): the runner's normalized
/// documents in `test/fixtures/pages/normalized/`, and `page show --json` in
/// `test/fixtures/page.json`, which the CLI's and the client's tests hold
/// byte for byte. Each assertion names a value the renderer draws.
struct PageModelTests {
    static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root
    }

    static func doc(_ name: String) throws -> PageDoc {
        try PageDoc.decode(Data(contentsOf: root.appendingPathComponent("test/fixtures/pages/normalized/\(name).json")))
    }

    @Test("page show --json: the row's fields and the document under doc")
    func decodesThePageJSON() throws {
        let page = try BoardPage.decode(Data(contentsOf: Self.root.appendingPathComponent("test/fixtures/page.json")))
        #expect(page.slot == "train")
        #expect(page.title == "Train integ-10")
        #expect(page.summary == "In review · 3 of 4 lanes green")
        #expect(page.themeAnchor == "00000000-0000-0000-0000-000000001001")
        #expect(page.revision == 3 && page.actor == "manager")
        #expect(page.updatedAtMs == 1_791_151_320_000)
        let doc = try #require(page.doc)
        #expect(doc.staleAfterMin == 120)
        #expect(doc.blocks.count == 10)
    }

    @Test("the train mockup: each block decodes to what the design draws")
    func decodesTheTrain() throws {
        let doc = try Self.doc("train")
        #expect(doc.title == "Train integ-10")
        guard doc.blocks.count == 10 else {
            Issue.record("expected ten blocks, got \(doc.blocks.count)")
            return
        }
        #expect(doc.blocks[0] == .text(md: "Phones' Plan view and the LFS record, one build, one review.", tone: .neutral))
        guard case .stats(let stats) = doc.blocks[1] else { Issue.record("no stats"); return }
        #expect(stats.map(\.label) == ["Lanes", "Green", "Fixing", "Build"])
        #expect(stats[2].tone == .attention && stats[3].detail == "one Mac slot")
        guard case .steps(let steps) = doc.blocks[2] else { Issue.record("no steps"); return }
        #expect(steps.map(\.state) == [.done, .done, .active, .todo])
        #expect(doc.blocks[3] == .heading("Lanes"))
        guard case .table(let columns, let rows) = doc.blocks[4] else { Issue.record("no table"); return }
        #expect(columns.map(\.title) == ["Lane", "Cards", "Gate", "State"])
        #expect(columns.map(\.grow) == [false, false, true, false])
        #expect(rows.count == 4)
        #expect(rows[0][0] == PageCell(ref: PageRef(.lane("ov-274-phones"))))
        #expect(rows[0][1] == PageCell(ref: PageRef(.task("ov-274"))))
        #expect(rows[0][2] == PageCell(text: "iOS UI class, Android captures"))
        #expect(rows[0][3] == PageCell(ref: PageRef(.lane("ov-274-phones")), show: .state))
        #expect(rows[3][3] == PageCell(text: "Fixing · round 1", tone: .attention))
        guard case .list(let items) = doc.blocks[6] else { Issue.record("no list"); return }
        #expect(items[0].state == .waiting && items[0].ref == PageRef(.ask("ov-274")))
        #expect(items[1].detail == "run 812")
        #expect(items[1].ref == PageRef(.url("https://github.com/example/overnight/actions/runs/812")))
        guard case .timeline(let entries, let given) = doc.blocks[7] else { Issue.record("no timeline"); return }
        #expect(!given)
        #expect(entries.map(\.at) == [1_791_151_320_000, 1_791_157_200_000])
        #expect(entries[1].ref == PageRef(.task("ov-274")))
        #expect(doc.blocks[8] == .progress(label: "Cards closed", done: 2, total: 4, detail: nil, parts: []))
        #expect(
            doc.blocks[9]
                == .links([
                    PageRef(.terminal(worktree: "integ-10", name: "build"), label: "Build terminal"), PageRef(.page("spend")),
                    PageRef(.theme("Visual language")),
                ]))
    }

    @Test("every block once: tones, alignments, mono cells, parts and a given order")
    func decodesEveryBlock() throws {
        let doc = try Self.doc("blocks")
        guard doc.blocks.count == 9 else {
            Issue.record("expected nine blocks, got \(doc.blocks.count)")
            return
        }
        guard case .text(_, let tone) = doc.blocks[1] else { Issue.record("no text"); return }
        #expect(tone == .attention)
        guard case .progress(_, 3, 10, "since Monday", let parts) = doc.blocks[3] else { Issue.record("no progress"); return }
        #expect(parts == [PagePart(label: "Done", count: 3), PagePart(label: "Active", count: 2), PagePart(label: "Waiting", count: 1)])
        guard case .table(let columns, let rows) = doc.blocks[4] else { Issue.record("no table"); return }
        #expect(columns.map(\.align) == [.start, .end, .center])
        #expect(rows[1][0] == PageCell(text: "beta", mono: true))
        guard case .list(let items) = doc.blocks[5] else { Issue.record("no list"); return }
        #expect(items.map(\.state) == [.done, .active, .waiting, .blocked, .failed, .todo, .none])
        #expect(items[3].tone == .attention)
        guard case .timeline(_, let given) = doc.blocks[6] else { Issue.record("no timeline"); return }
        #expect(given)
    }

    @Test("the spend and risks mockups decode, with spend's live cell and risks' ask")
    func decodesSpendAndRisks() throws {
        let spend = try Self.doc("spend")
        #expect(spend.staleAfterMin == 240 && spend.glance == "Week 62% used, resets Thu 9am")
        guard case .table(_, let rows) = spend.blocks[3] else { Issue.record("no table"); return }
        #expect(rows[0][1] == PageCell(ref: PageRef(.lane("mac-ux")), show: .spend))
        let risks = try Self.doc("risks")
        guard case .list(let items) = risks.blocks[1] else { Issue.record("no list"); return }
        #expect(items.map(\.state) == [.blocked, .waiting, .active, .done])
        #expect(items[0].ref == PageRef(.ask("ov-222")))
    }

    @Test("a block from a newer runner is its alt; one without alt, or that won't decode, costs only itself")
    func newerBlocksDegrade() throws {
        let json = """
            {"v":2,"title":"T","blocks":[
              {"type":"diagram","alt":"Pick, then build","nodes":[]},
              {"type":"chart"},
              {"type":"table","columns":"not a list"},
              {"type":"heading","text":"Still here"}
            ]}
            """
        let doc = try PageDoc.decode(Data(json.utf8))
        #expect(
            doc.blocks == [
                .unknown(type: "diagram", alt: "Pick, then build"), .unknown(type: "chart", alt: nil),
                .unknown(type: "table", alt: nil), .heading("Still here"),
            ])
    }

    @Test("an unknown state, tone, show or target reads as none, neutral, name and plain text")
    func unknownWordsAreNeutral() throws {
        let json = """
            {"v":1,"title":"T","blocks":[
              {"type":"list","items":[{"text":"x","state":"exploded","tone":"red","ref":{"widget":"w","label":"W"}}]},
              {"type":"table","columns":[{"title":"A"}],"rows":[[{"ref":{"lane":"l"},"show":"vibes"}]]}
            ]}
            """
        let doc = try PageDoc.decode(Data(json.utf8))
        guard case .list(let items) = doc.blocks[0], case .table(_, let rows) = doc.blocks[1] else {
            Issue.record("blocks didn't decode")
            return
        }
        #expect(items[0].state == .none && items[0].tone == .neutral)
        #expect(items[0].ref == PageRef(.unknown, label: "W"))
        #expect(rows[0][0].show == .name)
    }

    @Test("a listed page whose document this build can't read keeps its row")
    func anUnreadableDocumentKeepsTheRow() throws {
        let json = #"{"pages":[{"id":"p","slot":"train","title":"Train","doc":"not a document"}]}"#
        let list = try BoardPageList.decode(Data(json.utf8))
        #expect(list.pages.map(\.slot) == ["train"])
        #expect(list.pages[0].doc == nil)
    }

    @Test("under a header that says its title, a first heading repeating it is left out, and only that")
    func underItsTitle() throws {
        let risks = try Self.doc("risks")
        #expect(risks.under("Risks").blocks.count == 1)
        #expect(risks.under(" risks ").blocks.count == 1)
        #expect(risks.under("Visual language").blocks.count == 2)
        let train = try Self.doc("train")
        #expect(train.under("Lanes").blocks == train.blocks, "a heading that isn't first stays")
    }

    /// A page far past the design's limits, as a runner that didn't check
    /// (or a hand-edited store) could send: 100 blocks, a table of 10,000
    /// rows and 12 columns, a 5,000-character text and a 1,000-character
    /// cell.
    static func oversize() -> Data {
        let row = "[" + (0..<12).map { #""c\#($0)""# }.joined(separator: ",") + "]"
        let long = String(repeating: "x", count: 1_000)
        let table = #"{"type":"table","columns":[\#((0..<12).map { #"{"title":"T\#($0)"}"# }.joined(separator: ","))],"rows":[[{"text":"\#(long)"}],\#(Array(repeating: row, count: 10_000).joined(separator: ","))]}"#
        let text = #"{"type":"text","md":"\#(String(repeating: "word ", count: 1_000))"}"#
        let headings = (0..<98).map { #"{"type":"heading","text":"H\#($0)"}"# }
        return Data(#"{"v":1,"title":"Huge","blocks":[\#(([table, text] + headings).joined(separator: ","))]}"#.utf8)
    }

    @Test("a page past the limits draws what fits, ends with one line saying so, and doesn't hang")
    func oversizeIsClamped() throws {
        let started = Date()
        let doc = try PageDoc.decode(Self.oversize())
        #expect(Date().timeIntervalSince(started) < 5, "decoding took too long")
        #expect(doc.blocks.count == PageCaps.blocks + 1)
        #expect(doc.blocks.last == .unknown(type: "too-large", alt: "This page is too large to show in full."))
        guard case .table(let columns, let rows) = doc.blocks[0], case .text(let md, _) = doc.blocks[1] else {
            Issue.record("the table and text didn't decode")
            return
        }
        #expect(columns.count == 8 && rows.count == 50 && rows.allSatisfy { $0.count <= 8 })
        #expect(rows[0][0].text?.count == 200)
        #expect(md.count == 2_000)
    }

    @Test("a page inside the limits isn't marked")
    func withinLimitsIsUntouched() throws {
        for name in ["train", "spend", "risks", "blocks", "refs"] {
            #expect(!(try Self.doc(name)).blocks.contains { if case .unknown("too-large", _) = $0 { true } else { false } }, "\(name)")
        }
    }
}
