import Foundation

// Task keys in terminal output (ov-215): a key an agent prints opens its task,
// where a URL printed there already opens.
//
// The terminal core (`farcooler_vt_word_at`) answers one question: the
// whitespace-delimited word under a cell, soft wraps followed, and where the
// cell sits in it. The words that are keys stay this file's and Kotlin's
// (`TaskKeyLinks.kt`), through `matches(in:index:)`, so the fixture
// `test/fixtures/task-key-links.json` holds terminals as it holds Markdown.

extension TaskKeyLinks {
    /// A key under a terminal cell: the link, and the cells it covers, in the
    /// view's coordinates, so a renderer can underline it as it does a URL.
    public struct TerminalHit: Equatable, Sendable {
        public var url: URL
        public var startRow: Int
        public var startColumn: Int
        public var endRow: Int
        public var endColumn: Int

        public init(url: URL, startRow: Int, startColumn: Int, endRow: Int, endColumn: Int) {
            self.url = url
            self.startRow = startRow
            self.startColumn = startColumn
            self.endRow = endRow
            self.endColumn = endColumn
        }
    }

    /// The key a terminal cell sits in, or nil.
    ///
    /// `word` and `offset` are the core's answer for the cell at `row`,
    /// `column` (`offset` in UTF-16 units). A key is ASCII, one cell per code
    /// unit, so its first cell is `offset - match.start` cells back from the
    /// cell, through the wrap onto the row above when the key crosses one.
    /// The span clamps at the top of the view, as a URL's does.
    public static func terminalHit(
        word: String, offset: Int, row: Int, column: Int, columns: Int, index: TaskKeyIndex
    ) -> TerminalHit? {
        guard columns > 0, offset >= 0, !index.isEmpty else { return nil }
        guard let match = matches(in: word, index: index).first(where: { offset >= $0.start && offset < $0.start + $0.length }),
            let url = url(runner: index.runner, key: match.key)
        else { return nil }
        let first = max(0, row * columns + column - (offset - match.start))
        let last = first + match.length - 1
        return TerminalHit(
            url: url, startRow: first / columns, startColumn: first % columns, endRow: last / columns,
            endColumn: last % columns)
    }
}
