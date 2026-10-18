import Foundation
import Testing

@testable import AgentKit

// A terminal's grid selected for Copy (ov-609): the word a press lands on,
// and the text a range reads back as.
struct TerminalSelectionTests {
    typealias Cell = TerminalSelection.Cell
    typealias Point = TerminalSelection.Point

    /// A row of cells from a string: a space is a blank, `＊` a wide
    /// character followed by its spacer.
    private func row(_ text: String, width: Int = 20) -> [Cell] {
        var cells: [Cell] = []
        for character in text {
            if character == " " {
                cells.append(Cell(nil))
            } else if character.unicodeScalars.first!.properties.isEmojiPresentation || character == "漢" {
                cells.append(Cell(character, wide: true))
                cells.append(Cell(nil))
            } else {
                cells.append(Cell(character))
            }
        }
        while cells.count < width { cells.append(Cell(nil)) }
        return cells
    }

    @Test func aPressSelectsTheWordUnderIt() {
        let cells = row("git push origin main")
        let word = TerminalSelection.word(at: 6, in: cells, line: 3)
        #expect(word?.start == Point(line: 3, column: 4))
        #expect(word?.end == Point(line: 3, column: 7))
        #expect(word?.text(width: 20) { _ in cells } == "push")
    }

    @Test func aPressOnABlankSelectsNothing() {
        #expect(TerminalSelection.word(at: 3, in: row("git push"), line: 0) == nil)
        #expect(TerminalSelection.word(at: 15, in: row("git push"), line: 0) == nil)
    }

    @Test func aWideCharacterIsOneCharacterInItsWord() {
        let cells = row("a 漢字x b")
        // The press lands on the spacer of 漢.
        let word = TerminalSelection.word(at: 3, in: cells, line: 0)
        #expect(word?.text(width: 20) { _ in cells } == "漢字x")
    }

    @Test func linesBreakAsShownAndLoseTrailingBlanks() {
        let rows = [row("first line"), row("  second"), row("third and more")]
        let selection = TerminalSelection(anchor: Point(line: 2, column: 4), head: Point(line: 0, column: 6))
        let text = selection.text(width: 20) { rows.indices.contains($0) ? rows[$0] : nil }
        #expect(text == "line\n  second\nthird")
    }

    @Test func aRangeRunsIntoTheScrollback() {
        // Two lines of history above a screen of two: the screen's first row
        // is line 2 at the live screen, and line 1 when scrolled back one.
        #expect(TerminalSelection.line(row: 0, offset: 0, history: 2) == 2)
        #expect(TerminalSelection.line(row: 0, offset: 1, history: 2) == 1)
        #expect(TerminalSelection.row(line: 0, offset: 0, history: 2) == -2)
        let rows = [row("history one"), row("history two"), row("screen")]
        let selection = TerminalSelection(anchor: Point(line: 0, column: 8), head: Point(line: 2, column: 5))
        #expect(selection.text(width: 20) { rows[$0] } == "one\nhistory two\nscreen")
    }

    @Test func eachLinesColumnsAreTheRangesPart() {
        let selection = TerminalSelection(anchor: Point(line: 1, column: 5), head: Point(line: 3, column: 2))
        #expect(selection.columns(on: 0, width: 10) == nil)
        #expect(selection.columns(on: 1, width: 10) == 5...9)
        #expect(selection.columns(on: 2, width: 10) == 0...9)
        #expect(selection.columns(on: 3, width: 10) == 0...2)
        #expect(selection.columns(on: 4, width: 10) == nil)
    }
}
