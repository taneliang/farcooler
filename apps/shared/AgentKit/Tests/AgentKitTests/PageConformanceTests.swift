import Foundation
import Testing

@testable import AgentKit

/// Every page fixture through the reader the Mac and the iPhone draw with
/// (ov-269 design 7, ov-285): the documents the runner takes, what it stores
/// of them, and each one it refuses. The runner is the gate; the reader is the
/// second one (the fix-round rulings on ov-284), so a refused document that
/// reaches an app anyway draws what fits, says the rest didn't, never opens a
/// link that isn't `https`, and never stops the page drawing. Android's
/// `PageConformanceTest` holds its reader to the same files.
struct PageConformanceTests {
    static var pages: URL { PageModelTests.root.appendingPathComponent("test/fixtures/pages") }

    /// Every `.json` under `test/fixtures/pages/` but the refusals' index.
    static func fixtures() throws -> [String] {
        let walker = try #require(FileManager.default.enumerator(atPath: pages.path))
        return walker.compactMap { $0 as? String }.filter { $0.hasSuffix(".json") && $0 != "refusals.json" }.sorted()
    }

    static func read(_ name: String) throws -> PageDoc {
        try PageDoc.decode(Data(contentsOf: pages.appendingPathComponent(name)))
    }

    static func tooLarge(_ doc: PageDoc) -> Bool { doc.blocks.last == .unknown(type: "too-large", alt: PageWords.tooLarge) }

    /// The refusals' index: which file breaks which cap.
    static func caps() throws -> [String: String] {
        let data = try Data(contentsOf: pages.appendingPathComponent("refusals.json"))
        let rows = try #require(try JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        return Dictionary(uniqueKeysWithValues: rows.compactMap { row in
            guard let file = row["file"] as? String, let cap = row["cap"] as? String else { return nil }
            return (file, cap)
        })
    }

    @Test("the reader reads every fixture: all of them decode, but the one that isn't an object")
    func everyFixtureIsRead() throws {
        let names = try Self.fixtures()
        #expect(names.count == 74, "a fixture came or went: \(names.count)")
        for name in names {
            if name == "refused/not-an-object.json" {
                #expect(throws: (any Error).self) { try Self.read(name) }
                continue
            }
            let doc = try Self.read(name)
            #expect(!doc.blocks.isEmpty || name == "refused/no-blocks.json", "\(name) drew nothing")
        }
    }

    @Test("a page listed with a document that isn't an object keeps its row and draws that it can't")
    func aBrokenDocumentKeepsItsRow() throws {
        let raw = try String(contentsOf: Self.pages.appendingPathComponent("refused/not-an-object.json"), encoding: .utf8)
        let page = try BoardPage.decode(Data(#"{"id":"p","slot":"s","title":"Kept","doc":\#(raw)}"#.utf8))
        #expect(page.title == "Kept" && page.doc == nil)
    }

    @Test("every size cap the runner refuses, the reader holds: what fits, then the too-large row")
    func everyCapIsHeld() throws {
        let caps = try Self.caps()
        #expect(caps.count == 16, "a cap came or went")
        for (file, cap) in caps {
            let doc = try Self.read(file)
            #expect(Self.tooLarge(doc), "\(file) (\(cap)) drew without saying it's too large")
            #expect(doc.blocks.count <= PageCaps.blocks + 1, "\(file)")
            #expect(doc.title.count <= PageCaps.title && doc.summary.count <= PageCaps.summary, "\(file)")
            #expect((doc.glance ?? "").count <= PageCaps.glance, "\(file)")
            #expect(Self.refs(doc) <= PageCaps.refs, "\(file)")
        }
    }

    @Test("the caps, by value: 60 title characters, 32 KiB, 200 references")
    func capsByValue() throws {
        let title = try Self.read("refused/title-chars.json")
        #expect(title.title.count == 60 && title.title.hasSuffix("…"))
        let bytes = try Self.read("refused/document-bytes.json")
        // Twenty 2,000-character paragraphs: sixteen fit in 32 KiB, and the
        // seventeenth would pass it.
        #expect(bytes.blocks.count == 17)
        let refs = try Self.read("refused/refs.json")
        #expect(Self.refs(refs) == 200)
        // The words of the references past the cap are still drawn.
        guard case .list(let items) = refs.blocks[4] else {
            Issue.record("refs.json's fifth block isn't a list")
            return
        }
        #expect(items.allSatisfy { !$0.text.isEmpty })
        #expect(items.contains { $0.ref == nil })
    }

    @Test("a document inside the caps isn't marked, and keeps every block")
    func insideTheCapsIsWhole() throws {
        for name in try Self.fixtures() where name.hasPrefix("normalized/") || !name.contains("/") {
            let doc = try Self.read(name)
            #expect(!Self.tooLarge(doc), "\(name)")
            let raw = try #require(
                try JSONSerialization.jsonObject(with: Data(contentsOf: Self.pages.appendingPathComponent(name))) as? [String: Any])
            #expect(doc.blocks.count == (raw["blocks"] as? [Any])?.count, "\(name) lost a block")
        }
    }

    @Test("no link the runner refuses opens: each is plain text, in a reference or in Markdown")
    func refusedLinksDontOpen() throws {
        let world = PageWorld()
        let linky = try Self.fixtures().filter { $0.hasPrefix("refused/") && ($0.contains("link") || $0.hasPrefix("refused/md-")) }
        #expect(linky.count == 16)
        for name in linky {
            let doc = try Self.read(name)
            for block in doc.blocks {
                switch block {
                case .links(let refs):
                    for ref in refs { #expect(world.resolve(ref).destination == nil, "\(name) opens \(ref.target.rawName)") }
                case .text(let md, _):
                    for piece in PageMarkdown.pieces(md) {
                        let text: String
                        switch piece {
                        case .prose(let t), .item(_, let t, _): text = t
                        case .plain: continue
                        }
                        for run in PageMarkdown.inline(text).runs {
                            if let url = run.link { #expect(PageLinks.https(url.absoluteString) != nil, "\(name) links \(url)") }
                        }
                    }
                default: break
                }
            }
        }
    }

    @Test("a link whose words name another domain is followed by the domain it goes to")
    func aLabelCantHideItsDomain() throws {
        for name in ["refused/md-label-names-another-domain.json", "refused/md-label-url-names-another-domain.json"] {
            guard case .text(let md, _)? = try Self.read(name).blocks.first, case .prose(let line)? = PageMarkdown.pieces(md).first else {
                Issue.record("\(name) isn't one paragraph")
                continue
            }
            #expect(String(PageMarkdown.inline(line).characters).contains("evil.example"), "\(name) hides where it goes")
        }
    }

    @Test("a block this build doesn't know is its alt, or says it needs a newer Far Cooler")
    func unknownBlocks() throws {
        let future = try Self.read("refused/future-version.json")
        #expect(future.blocks == [.heading("Known"), .unknown(type: "gauge", alt: "Disk is 80% full.")])
        #expect(try Self.read("refused/unknown-block.json").blocks == [.unknown(type: "diagram", alt: nil)])
    }

    static func refs(_ doc: PageDoc) -> Int {
        doc.blocks.reduce(0) { total, block in
            switch block {
            case .table(_, let rows): total + rows.joined().filter { $0.ref != nil }.count
            case .list(let items): total + items.filter { $0.ref != nil }.count
            case .timeline(let entries, _): total + entries.filter { $0.ref != nil }.count
            case .links(let refs): total + refs.count
            case .stats(let items): total + items.filter { $0.ref != nil }.count
            default: total
            }
        }
    }
}
