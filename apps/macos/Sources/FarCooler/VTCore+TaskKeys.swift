import AgentKit
import CFarCoolerVT
import Foundation

extension VTCore {
    /// The link under a cell, with where it sits: the URL the core finds, and
    /// failing that the task a key there names (ov-215). A URL wins, so a key
    /// inside one, or inside a labelled hyperlink, stays the URL's.
    func link(
        atRow row: Int, column: Int, columns: Int, linker: TaskKeyLinker
    ) -> (url: String, span: FarCoolerVtUrlSpan)? {
        if let found = url(atRow: row, column: column) { return found }
        guard !linker.index.isEmpty, let hit = word(atRow: row, column: column),
            let key = TaskKeyLinks.terminalHit(
                word: hit.word, offset: hit.offset, row: row, column: column, columns: columns,
                index: linker.index)
        else { return nil }
        var span = FarCoolerVtUrlSpan()
        span.start_row = UInt16(clamping: key.startRow)
        span.start_column = UInt16(clamping: key.startColumn)
        span.end_row = UInt16(clamping: key.endRow)
        span.end_column = UInt16(clamping: key.endColumn)
        return (key.url.absoluteString, span)
    }
}
