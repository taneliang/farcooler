import Foundation

extension Markdown {
    /// The most of a folded text's start drawn while it's folded: past the
    /// fold at any width a reply is set at (ov-788).
    static let foldCharacters = 1_500

    /// The start of `text` that a fold of `lines` lines shows, and enough
    /// past it to know it folds: what a folded report draws (ov-788).
    ///
    /// A fold shows the top `lines + 1` lines of the text (`FoldStack`), but
    /// a report drew all of itself under the fold, every line set by
    /// TextKit, to learn only that it was taller. A lane's report is many
    /// kilobytes, and an iPhone's fling measured one at 30% of the main
    /// thread, its hitches up to 237 ms, every time a report scrolled in.
    ///
    /// So the prefix ends once it holds `lines + 3` lines (a source line is
    /// at least a body line drawn, a line of code at least half of one) or
    /// `foldCharacters` characters (more lines than a fold at any width),
    /// whichever comes first. Either way it's taller than the fold, so
    /// whether it folds is the same answer, and the lines that show are the
    /// same lines, since blocks are drawn top down and the prefix is cut
    /// between source lines: never between a table's header and its rule,
    /// and with a code fence it's cut inside closed again. A text shorter
    /// than that is returned whole, so it's drawn exactly as before.
    ///
    /// Never inside a line: a line is a paragraph to TextKit, which breaks
    /// a paragraph's lines as a whole, so a paragraph cut short wraps
    /// differently from its first line on. A single line far longer than
    /// the fold is kept whole, as it was drawn before.
    ///
    /// It reads only the lines it keeps: a report of any length costs the
    /// same to fold.
    public static func foldPrefix(_ text: String, lines: Int) -> String {
        let wanted = Double(lines + 3)
        var counted = 0.0
        var fence = false
        var start = text.startIndex
        var characters = 0
        while start < text.endIndex {
            let newline = text[start...].firstIndex(of: "\n")
            let end = newline ?? text.endIndex
            let line = text[start..<end]
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("```") {
                fence.toggle()
            } else if fence {
                counted += 0.5
            } else if !trimmed.isEmpty, !["---", "***", "___"].contains(trimmed) {
                counted += 1
            }
            characters += line.count + 1
            guard let newline else { return text }
            let next = text.index(after: newline)
            let full = counted >= wanted || characters >= foldCharacters
            if full, !fence, !headsTable(text, at: next) {
                return String(text[..<newline])
            }
            if full, fence {
                // Closed where it's cut: an open fence would read the
                // rest of nothing as code, and draw nothing else wrong.
                return text[..<newline] + "\n```"
            }
            start = next
        }
        return text
    }

    /// Whether the line at `index` is a table's rule, which reads the line
    /// above it as the table's header.
    private static func headsTable(_ text: String, at index: String.Index) -> Bool {
        guard index < text.endIndex else { return false }
        let end = text[index...].firstIndex(of: "\n") ?? text.endIndex
        return isTableRule(String(text[index..<end]))
    }
}
