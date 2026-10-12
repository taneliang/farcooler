import Foundation

/// An agent's reply as one plain line, for a place that draws no Markdown (ov-495):
/// the title bar, a row's quiet line, the phone's peek. The runner sends
/// `said` flattened to one line and cut at 120 characters, so a reply that
/// opened with a list arrives as "Four asks, in order: - **PDF export**
/// (ha-12): ...", and drawn as it is, the asterisks and dashes show.
///
/// Emphasis, code, links, headings, quotes and list markers go; the words
/// stay. Only the first line is kept, for text that does still have lines.
/// Android has the same rules in `PlainLine.kt`.
public enum PlainLine {
    public static func of(_ text: String) -> String {
        guard let first = text.split(whereSeparator: \.isNewline).first(where: { !$0.allSatisfy(\.isWhitespace) }) else {
            return ""
        }
        var line = String(first)
        // The rules, in order: each (pattern, template) is a regular expression.
        let rules: [(String, String)] = [
            (#"!?\[([^\]]*)\]\([^)]*\)"#, "$1"),  // a link or image: its text
            (#"^\s*(?:#{1,6}|>)\s+"#, ""),  // a heading or quote
            (#"^\s*(?:[-*+•]|\d{1,3}[.)])\s+"#, ""),  // a list marker that opens the line
            (#"(?<=\s)(?:[-*+•]|\d{1,3}[.)])\s+(?=[*_`\[])"#, ""),  // one flattened into the line, before a marked-up word
            (#"(?<=:)\s+(?:[-*+•]|\d{1,3}[.)])\s+"#, " "),  // or right after a colon
            (#"`+([^`]+)`+"#, "$1"),  // code
            (#"(\*\*|__)(?=\S)(.+?)(?<=\S)\1"#, "$2"),  // bold
            (#"(?<![\w*])\*(?=[^\s*])(.+?)(?<=[^\s*])\*(?![\w*])"#, "$1"),  // italic with asterisks
            (#"(?<![\w_])_(?=[^\s_])(.+?)(?<=[^\s_])_(?![\w_])"#, "$1"),  // italic with underscores
            (#"~~(?=\S)(.+?)(?<=\S)~~"#, "$1"),  // strikethrough
        ]
        for (pattern, template) in rules {
            line = line.replacingOccurrences(of: pattern, with: template, options: .regularExpression)
        }
        return line.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }
}
