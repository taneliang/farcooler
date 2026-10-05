import Foundation

/// What the orchestrator's pane says it's doing, with the terminal's own
/// furniture taken off (ov-329): the key hints an agent's status line carries
/// ("esc to interrupt", ⌘K, ctrl+o to expand), and the spinner, bullet and
/// box-drawing marks it draws before and between its words. What's left is a
/// sentence a person would say; with nothing meaningful left, nil, so the
/// title bar shows no activity rather than a mark.
///
/// The title bar's "⌘K" beside the line was not this text: it is the field's
/// own shortcut hint, drawn after the activity (`TitleStatusView`). The
/// agents' hints are stripped all the same, for they reach `line` and
/// `lastSaid` from the runner's screen reads.
enum NowDoingText {
    /// A key chord: a modifier symbol and a key (⌘K, ⌃C, ⌥⌘P, ⇧⇥), or
    /// spelled out (ctrl+c, shift+tab, cmd+k, esc, Esc).
    private static let chord =
        #"(?:[⌘⌃⌥⇧]+\s?[A-Za-z0-9⇥⏎↩←→↑↓⎋]|\b(?:ctrl|control|cmd|command|opt|option|alt|shift)\s?\+\s?[A-Za-z0-9]+(?:\s?\+\s?[A-Za-z0-9]+)*|\besc(?:ape)?\b(?=\s+to\s))"#

    /// A hint: a chord and what it does ("ctrl+o to expand", "esc to
    /// interrupt", "shift+tab to cycle"), or "? for shortcuts".
    private static var hint: Regex<AnyRegexOutput> {
        try! Regex(
        "(?i:\(chord)(?:\\s+to\\s+[a-z]+)?|\\?\\s+for\\s+shortcuts)")
    }

    /// A bracketed aside made of hints and the numbers beside them:
    /// "(esc to interrupt)", "(12s · ↓ 1.2k tokens · esc to interrupt)".
    private static var aside: Regex<AnyRegexOutput> {
        try! Regex(
        #"\([^()]*(?i:esc to interrupt|ctrl\+[a-z] to|tokens)[^()]*\)"#)
    }

    /// The marks an agent draws: spinners, bullets, tree corners.
    private static let marks: CharacterSet = {
        var set = CharacterSet()
        set.insert(charactersIn: Unicode.Scalar(0x2500)!...Unicode.Scalar(0x257F)!)  // box drawing
        set.insert(charactersIn: Unicode.Scalar(0x2800)!...Unicode.Scalar(0x28FF)!)  // braille spinners
        set.insert(charactersIn: "✻✶✳✢✽✺✹✸✷✦✧•●○◐◓◑◒⏺⎿⏵⏸∙")
        return set
    }()

    static func clean(_ text: String?) -> String? {
        guard var line = text else { return nil }
        line = line.replacing(aside, with: " ")
        line = line.replacing(hint, with: " ")
        line = String(line.unicodeScalars.map { marks.contains($0) ? " " : Character($0) })
        // Runs of space, and the dashes, dots and colons the removals left at the ends. A sentence's own period stays.
        line = line.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let edge = CharacterSet(charactersIn: "-–—:;,|/\\·… ")
        while let first = line.unicodeScalars.first, edge.contains(first) { line.removeFirst() }
        while let last = line.unicodeScalars.last, edge.contains(last) {
            // Keep a sentence's ellipsis, which says it's still going.
            if last == "…" { break }
            line.removeLast()
        }
        guard line.contains(where: { $0.isLetter || $0.isNumber }) else { return nil }
        return line
    }
}
