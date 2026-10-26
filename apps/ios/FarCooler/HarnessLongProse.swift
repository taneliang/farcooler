#if DEBUG
import Foundation

/// Long Markdown as agents really write it (ov-788, ov-791): a lane's
/// report and a card's notes run to many kilobytes of headings, lists,
/// tables, code and `ov-N` keys. The harnesses' own text is a line or two,
/// which a fling never notices, so `-native-long-reports` and
/// `-phone-long-record` draw these instead, for measuring a fling over them.
enum HarnessLongProse {
    /// A lane's report, about 11 KB, different for each `n`.
    static func laneReport(_ n: Int) -> String {
        var out = "## Lane ios-\(700 + n % 90): the transcript keeps its place (ov-\(600 + n % 180))\n\n"
        out += "**Commits**\n\n"
        for c in 0..<5 {
            out += "- `\(String(format: "%07x", 0x1a2b3c4 &+ n &* 977 &+ c &* 131))` ios: a row above the screen is measured once, not twice (ov-\(610 + c))\n"
        }
        for section in 0..<4 {
            out += "\n### Finding \(section + 1): what the profile showed\n\n"
            out += "The fling over a long session measured **\(14 + (n + section) % 9) hitches** over one frame, the worst "
                + "\(40 + (n * 7 + section) % 60) ms, in `NativeAgentView.swift:\(400 + section * 37)`. Every one of them lands when a "
                + "row comes into view for the first time: its body runs, its `MarkdownText` is parsed, and its height is measured "
                + "before the frame can commit. See ov-\(626 + section) and ov-675 for the earlier rounds, and "
                + "[the Instruments guide](https://developer.apple.com/documentation/xcode/analyzing-hitches-in-your-app) for the method.\n\n"
            out += "A second paragraph says what was ruled out. The image decode is already off the main thread (ov-629), "
                + "the diff of the rows is linear in what changed, and the relative-time formatter is cached. What remained was "
                + "text layout: *every* line of a folded report, measured to learn that it folds.\n\n"
            out += "| Frame | Self (ms) | Share |\n|---|---:|---:|\n"
            for row in 0..<6 {
                out += "| `TextLayout.measure` step \(row) of finding \(section) | \(12 - row).\(n % 10) | \(30 - row * 4)% |\n"
            }
            out += "\n```swift\n"
            for line in 0..<14 {
                out += "        let height\(line) = layout.measure(text, width: proposal.width ?? 320) // \(section).\(line)\n"
            }
            out += "```\n\n"
            for item in 0..<5 {
                out += "- Point \(item + 1): the row is \(3 + item) screens below the fold, so its guess is wrong by "
                    + "\(item * 37 + 12) pt until it is built (ov-\(700 + item)).\n"
                out += "  - Nested: `onGeometryChange` reports it on the next pass, which moves every row under it.\n"
            }
            out += "\n> Measured on an iPhone 17 simulator at 120 Hz; the device was quieter but the order of costs was the same.\n"
        }
        out += "\n### Tests\n\n"
        for t in 1...6 {
            out += "\(t). `ConversationScrollUITests.testStep\(t)`: passed, and red with the fix reverted (\(t * 3) assertions).\n"
        }
        out += "\n---\n\nLeft open: nothing that blocks the train. The full report is in `.claude/agent/reports/ios-\(700 + n % 90)/`.\n"
        return out
    }

    /// The sizes of a real long card's notes, in bytes, oldest first: ov-752's
    /// seventeen (25 Oct), status lines among findings, decisions and
    /// questions of up to 1.7 KB. Real notes are short: of 281 notes on
    /// ov-740 to ov-791, the median was 44 bytes and the largest 1,751.
    static let noteSizes = [34, 33, 392, 1_747, 40, 207, 1_060, 600, 385, 292, 380, 696, 40, 803, 40, 336, 40]

    /// A card's note `n`, about `noteSizes`' size: a status line, or the
    /// start of a report cut at a line.
    static func note(_ n: Int) -> String {
        let size = noteSizes[n % noteSizes.count]
        if size < 60 { return "moved from in_progress to needs_decision" }
        let report = laneReport(n).split(separator: "\n", omittingEmptySubsequences: false).dropFirst(2)
        var out = "**Owner, 25 Oct, decision \(n % 7 + 1):** all as recommended, after spike ov-\(750 + n % 40).\n\n"
        for line in report {
            if out.utf8.count + line.utf8.count >= size { break }
            out += line + "\n"
        }
        return out
    }
}
#endif
