import Foundation

// A range of a terminal's grid, selected for Copy (ov-609).
//
// A terminal pane is a canvas of cells, not text, so selection is cells: a
// press on a word selects the word (its run of non-blank cells), a drag or a
// handle moves one end, and Copy reads the cells back as text, a line break
// at the end of every row as the screen shows it, each row's trailing blanks
// dropped.
//
// Lines are counted from the oldest line of scrollback, not from the top of
// the screen, so a selection keeps its place while the pane scrolls and can
// run up into the scrollback: a screen row is `history - offset + row`.

public struct TerminalSelection: Equatable, Sendable {
    /// One cell: a line counted from the oldest line of scrollback, and a
    /// column.
    public struct Point: Comparable, Hashable, Sendable {
        public var line: Int
        public var column: Int

        public init(line: Int, column: Int) {
            self.line = line
            self.column = column
        }

        public static func < (a: Point, b: Point) -> Bool {
            a.line != b.line ? a.line < b.line : a.column < b.column
        }
    }

    /// A cell as selection reads it: its character (nil where blank), and
    /// whether it's the first half of a double-width character, whose
    /// second column is a spacer and no character of its own.
    public struct Cell: Equatable, Sendable {
        public var character: Character?
        public var wide: Bool

        public init(_ character: Character?, wide: Bool = false) {
            self.character = character
            self.wide = wide
        }
    }

    /// Where the selection began, and where it ends now: either may come
    /// first.
    public var anchor: Point
    public var head: Point

    public init(anchor: Point, head: Point) {
        self.anchor = anchor
        self.head = head
    }

    /// The first selected cell.
    public var start: Point { min(anchor, head) }
    /// The last selected cell, included.
    public var end: Point { max(anchor, head) }

    /// The columns selected on `line` of a grid `columns` wide, nil where
    /// none are: the first line from `start`, the last up to `end`, and
    /// every line between whole.
    public func columns(on line: Int, width columns: Int) -> ClosedRange<Int>? {
        guard columns > 0, line >= start.line, line <= end.line else { return nil }
        let from = line == start.line ? start.column : 0
        let to = line == end.line ? end.column : columns - 1
        let low = max(0, min(from, columns - 1)), high = max(0, min(to, columns - 1))
        return low <= high ? low...high : nil
    }

    /// A screen row's line, counted from the oldest line of scrollback.
    public static func line(row: Int, offset: Int, history: Int) -> Int { history - offset + row }

    /// A line's row on the screen, which may be above or below it.
    public static func row(line: Int, offset: Int, history: Int) -> Int { line - history + offset }

    /// The word under `column` in `row`: its run of non-blank cells, nil on
    /// a blank. A wide character's spacer belongs to the character before it.
    public static func word(at column: Int, in row: [Cell], line: Int) -> TerminalSelection? {
        guard row.indices.contains(column) else { return nil }
        var at = column
        if row[at].character == nil, at > 0, row[at - 1].wide { at -= 1 }
        guard row[at].character != nil else { return nil }
        func filled(_ i: Int) -> Bool {
            row[i].character != nil || (i > 0 && row[i - 1].wide && row[i - 1].character != nil)
        }
        var first = at, last = at
        while first > 0, filled(first - 1) { first -= 1 }
        while last < row.count - 1, filled(last + 1) { last += 1 }
        return TerminalSelection(anchor: Point(line: line, column: first), head: Point(line: line, column: last))
    }

    /// The selected text: each line's selected cells, a blank as a space and
    /// a spacer as nothing, its trailing blanks dropped, and the lines joined
    /// by line breaks. `row` reads a line's cells, nil for a line there's no
    /// longer any of (scrolled out of the scrollback), which is left out.
    public func text(width columns: Int, row: (Int) -> [Cell]?) -> String {
        var lines: [String] = []
        for line in start.line...end.line {
            guard let cells = row(line), let range = self.columns(on: line, width: columns) else { continue }
            var words = ""
            for column in range where column < cells.count {
                let cell = cells[column]
                if let character = cell.character {
                    words.append(character)
                } else if column > 0, cells[column - 1].wide, cells[column - 1].character != nil {
                    continue
                } else {
                    words.append(" ")
                }
            }
            while words.last == " " { words.removeLast() }
            lines.append(words)
        }
        return lines.joined(separator: "\n")
    }
}
