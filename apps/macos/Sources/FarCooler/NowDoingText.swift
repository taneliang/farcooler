import Foundation

/// The orchestrator's status line, with the terminal's own furniture taken
/// off (ov-329): the phrase "esc to interrupt", a key hint left standing
/// alone at its end (⌘K, ⌃C) or in brackets, and the spinner, bullet and
/// box-drawing marks drawn around it. With nothing meaningful left, nil, so
/// the title bar shows no activity rather than a mark.
///
/// Only `Terminal.line` goes through this. A chord or "escape to dismiss" in
/// what an agent says or asks is its content, and `OrchestratorRow.nowDoing`
/// leaves that as written.
///
/// The title bar's "⌘K" beside the line was not this text: it is the field's
/// own shortcut hint, drawn after the activity (`TitleStatusView`).
enum NowDoingText {
    /// A key chord as one token: modifier symbols and a key (⌘K, ⌥⌘←), or
    /// spelled out (ctrl+c, shift+tab).
    private static let chord =
        #"(?:[⌘⌃⌥⇧]+[A-Za-z0-9⇥⏎↩←→↑↓⎋]|(?i:ctrl|control|cmd|command|opt|option|alt|shift)\+[A-Za-z0-9]+)"#

    /// The exact phrase, however it's cased.
    private static var interrupt: Regex<AnyRegexOutput> { try! Regex(#"(?i)\besc to interrupt\b"#) }

    /// A chord left alone at the end of the line, or by itself in brackets.
    private static var trailingChord: Regex<AnyRegexOutput> {
        try! Regex(#"(?:\s+|^)\(?\#(chord)\)?\s*$"#)
    }

    /// Brackets with nothing in them, or only separators, once a hint left.
    private static var emptyBrackets: Regex<AnyRegexOutput> { try! Regex(#"\(\s*[·•|,;\s]*\)"#) }

    /// A separator left hanging before a closing bracket by the phrase that went.
    private static var hanging: Regex<AnyRegexOutput> { try! Regex(#"\s*[·•|,;]\s*\)"#) }

    /// The marks an agent draws: spinners, bullets, tree corners.
    private static let marks: CharacterSet = {
        var set = CharacterSet()
        set.insert(charactersIn: Unicode.Scalar(0x2500)!...Unicode.Scalar(0x257F)!)  // box drawing
        set.insert(charactersIn: Unicode.Scalar(0x2800)!...Unicode.Scalar(0x28FF)!)  // braille spinners
        set.insert(charactersIn: "✻✶✳✢✽✺✹✸✷✦✧•●○◐◓◑◒⏺⎿⏵⏸∙")
        return set
    }()

    /// What a status line leaves at its ends: dashes, bars and separators.
    private static let edge = CharacterSet(charactersIn: "–—:;,|·… ")

    static func clean(_ text: String?) -> String? {
        guard var line = text else { return nil }
        line = line.replacing(interrupt, with: "")
        line = line.replacing(hanging, with: ")")
        line = line.replacing(emptyBrackets, with: " ")
        // Box drawing and braille are never prose: out wherever they stand.
        // The other marks only where they lead or trail the line.
        line = String(line.unicodeScalars.map { scalar -> Character in
            (0x2500...0x257F).contains(scalar.value) || (0x2800...0x28FF).contains(scalar.value) ? " " : Character(scalar)
        })
        line = line.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        while let first = line.unicodeScalars.first, marks.contains(first) || edge.contains(first) { line.removeFirst() }
        // A chord standing alone at the end, possibly behind a mark or two.
        var changed = true
        while changed {
            changed = false
            if let last = line.unicodeScalars.last, marks.contains(last) || (edge.contains(last) && last != "…") {
                line.removeLast()
                changed = true
            }
            let trimmed = line.replacing(trailingChord, with: "")
            if trimmed != line {
                line = trimmed
                changed = true
            }
        }
        line = line.trimmingCharacters(in: .whitespaces)
        guard line.contains(where: { $0.isLetter || $0.isNumber }) else { return nil }
        return line
    }
}
