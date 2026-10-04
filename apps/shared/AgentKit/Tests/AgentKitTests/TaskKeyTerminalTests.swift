import Foundation
import Testing

@testable import AgentKit

/// Task keys in terminal output (ov-215): which cells hit, by the same fixture
/// the Markdown links read.
struct TaskKeyTerminalTests {
    /// The index the fixture describes: its prefixes, and each known key a
    /// task.
    static func index(_ fixture: TaskKeyLinksFixture) -> TaskKeyIndex {
        var targets: [String: TaskKeyTarget] = [:]
        for key in fixture.known {
            targets[key] = TaskKeyTarget(runner: "r1", workspace: "w", task: "t-\(key)", key: key)
        }
        return TaskKeyIndex(runner: "r1", prefixes: Set(fixture.prefixes), targets: targets)
    }

    /// What the core would answer for the cell `at` of a one-row screen
    /// showing `text`: the ASCII-whitespace-delimited word and the cell's
    /// offset in it, or nil on a blank.
    static func word(in text: String, at cell: Int) -> (word: String, offset: Int)? {
        let units = Array(text.utf16)
        let space: (UInt16) -> Bool = { (0x09...0x0D).contains($0) || $0 == 0x20 }
        guard cell < units.count, !space(units[cell]) else { return nil }
        var lo = cell
        while lo > 0, !space(units[lo - 1]) { lo -= 1 }
        var hi = cell
        while hi + 1 < units.count, !space(units[hi + 1]) { hi += 1 }
        return (String(decoding: units[lo...hi], as: UTF16.self), cell - lo)
    }

    @Test("Every fixture key hits from each of its cells, and no cell beside it does")
    func theFixture() throws {
        let fixture = try TaskKeyLinksFixture.load()
        let index = Self.index(fixture)
        for item in fixture.cases {
            let width = item.text.utf16.count + 1
            for cell in 0..<width {
                let hit = Self.word(in: item.text, at: cell).flatMap {
                    TaskKeyLinks.terminalHit(
                        word: $0.word, offset: $0.offset, row: 0, column: cell, columns: width, index: index)
                }
                let link = item.links.first { ($0.start..<($0.start + $0.key.utf16.count)).contains(cell) }
                #expect(
                    hit?.url == link.flatMap { TaskKeyLinks.url(runner: "r1", key: $0.key) },
                    "\(item.text.debugDescription) cell \(cell)")
                if let link, let hit {
                    #expect(hit.startColumn == link.start && hit.endColumn == link.start + link.key.utf16.count - 1)
                }
            }
        }
    }

    @Test("A key wrapped over two rows reports its span across both")
    func wrapped() throws {
        let index = Self.index(try TaskKeyLinksFixture.load())
        // "abcdefgh ov-190 z" on a 10-column screen: the key starts at the
        // last cell of row 0.
        let hit = try #require(
            TaskKeyLinks.terminalHit(word: "ov-190", offset: 5, row: 1, column: 4, columns: 10, index: index))
        #expect((hit.startRow, hit.startColumn, hit.endRow, hit.endColumn) == (0, 9, 1, 4))
        let top = try #require(
            TaskKeyLinks.terminalHit(word: "ov-190", offset: 0, row: 0, column: 9, columns: 10, index: index))
        #expect(top.url == hit.url)
    }

    @Test("A key's span clamps at the top of the view")
    func clamped() throws {
        let index = Self.index(try TaskKeyLinksFixture.load())
        // The pointer is on row 0 but the key began on a row scrolled out.
        let hit = try #require(
            TaskKeyLinks.terminalHit(word: "ov-190", offset: 5, row: 0, column: 2, columns: 10, index: index))
        #expect((hit.startRow, hit.startColumn) == (0, 0))
    }

    @Test("A runner with nothing known links nothing")
    func empty() {
        #expect(
            TaskKeyLinks.terminalHit(word: "ov-190", offset: 1, row: 0, column: 1, columns: 10, index: .empty) == nil)
    }
}
