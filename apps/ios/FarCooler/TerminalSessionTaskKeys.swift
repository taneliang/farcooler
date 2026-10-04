import Foundation

extension TerminalSession {
    /// What a long press on a cell holds: the URL the core finds, and failing
    /// that the task link of a key there (ov-215). A URL wins, so a key inside
    /// one, or inside a labelled hyperlink, stays the URL's.
    func link(atRow row: Int, column: Int, linker: TaskKeyLinker) -> String? {
        if let url = url(atRow: row, column: column) { return url }
        guard !linker.index.isEmpty, let hit = word(atRow: row, column: column),
            let key = TaskKeyLinks.terminalHit(
                word: hit.word, offset: hit.offset, row: row, column: column, columns: grid?.columns ?? 0,
                index: linker.index)
        else { return nil }
        return key.url.absoluteString
    }
}
