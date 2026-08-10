import Foundation

// The diff a tool call's edit is rendered from, shared by both apps.
//
// Pure Foundation: no SwiftUI, no AppKit, no UIKit. It was written once and
// then copied, so the two targets carried the same algorithm — the same
// trimming, the same LCS, the same 160,000-cell cutoff — with only the doc
// comment drifting between them. Two copies of a diff algorithm is two places
// for an off-by-one to live and one place for it to be fixed.

/// The diffing itself, kept apart from the view so it has no SwiftUI in it.
public enum DiffComputation {
    public enum Kind { case context, added, removed }

    public struct Line: Identifiable {
        public let id: Int
        public let kind: Kind
        public let oldNumber: Int?
        public let newNumber: Int?
        public let text: String

        /// Public so the review surface renders the SAME line model the agent
        /// transcript does, rather than declaring a second one that is free to
        /// disagree about what a changed line is.
        public init(id: Int, kind: Kind, oldNumber: Int?, newNumber: Int?, text: String) {
            self.id = id
            self.kind = kind
            self.oldNumber = oldNumber
            self.newNumber = newNumber
            self.text = text
        }
    }

    /// Line-based diff of two whole file texts.
    ///
    /// A real Myers diff is O(n·d) and would be the right tool against two
    /// arbitrary files; what a tool call actually hands over is usually one
    /// small change inside two mostly-identical texts, so trimming the common
    /// prefix and suffix first — O(n) — throws away nearly all of the work
    /// before the expensive part ever runs. The expensive part, an LCS over
    /// whatever is left in the middle, is only worth its O(n·m) on the
    /// leftover sliver; past a size where that would be slow it falls back to
    /// "the whole middle changed", which is still a correct diff, just not the
    /// minimal one.
    public static func compute(old: String, new: String) -> [Line] {
        let oldLines = old.isEmpty ? [] : old.components(separatedBy: "\n")
        let newLines = new.isEmpty ? [] : new.components(separatedBy: "\n")

        var prefix = 0
        while prefix < oldLines.count, prefix < newLines.count,
            oldLines[prefix] == newLines[prefix]
        {
            prefix += 1
        }

        var suffix = 0
        while suffix < oldLines.count - prefix, suffix < newLines.count - prefix,
            oldLines[oldLines.count - 1 - suffix] == newLines[newLines.count - 1 - suffix]
        {
            suffix += 1
        }

        let oldMiddle = Array(oldLines[prefix..<(oldLines.count - suffix)])
        let newMiddle = Array(newLines[prefix..<(newLines.count - suffix)])

        var result: [Line] = []
        var id = 0
        var oldNum = 1
        var newNum = 1

        func push(_ kind: Kind, old: Int?, new: Int?, text: String) {
            result.append(Line(id: id, kind: kind, oldNumber: old, newNumber: new, text: text))
            id += 1
        }

        for line in oldLines[0..<prefix] {
            push(.context, old: oldNum, new: newNum, text: line)
            oldNum += 1
            newNum += 1
        }

        // 400*400 is a worst case of 160,000 cells, which is fast; past that
        // the middle is shown as a flat replace rather than paying for the
        // table.
        if oldMiddle.count * newMiddle.count <= 160_000 {
            for op in lcsDiff(oldMiddle, newMiddle) {
                switch op {
                case let .equal(line):
                    push(.context, old: oldNum, new: newNum, text: line)
                    oldNum += 1
                    newNum += 1
                case let .removed(line):
                    push(.removed, old: oldNum, new: nil, text: line)
                    oldNum += 1
                case let .added(line):
                    push(.added, old: nil, new: newNum, text: line)
                    newNum += 1
                }
            }
        } else {
            for line in oldMiddle {
                push(.removed, old: oldNum, new: nil, text: line)
                oldNum += 1
            }
            for line in newMiddle {
                push(.added, old: nil, new: newNum, text: line)
                newNum += 1
            }
        }

        let suffixStart = oldLines.count - suffix
        for offset in 0..<suffix {
            push(.context, old: oldNum, new: newNum, text: oldLines[suffixStart + offset])
            oldNum += 1
            newNum += 1
        }

        return result
    }

    private enum Op {
        case equal(String)
        case removed(String)
        case added(String)
    }

    /// Classic LCS table, walked backwards to recover the edit script.
    private static func lcsDiff(_ a: [String], _ b: [String]) -> [Op] {
        guard !a.isEmpty else { return b.map { .added($0) } }
        guard !b.isEmpty else { return a.map { .removed($0) } }

        var table = [[Int]](repeating: [Int](repeating: 0, count: b.count + 1), count: a.count + 1)
        for i in stride(from: a.count - 1, through: 0, by: -1) {
            for j in stride(from: b.count - 1, through: 0, by: -1) {
                table[i][j] =
                    a[i] == b[j]
                    ? table[i + 1][j + 1] + 1
                    : max(table[i + 1][j], table[i][j + 1])
            }
        }

        var ops: [Op] = []
        var i = 0, j = 0
        while i < a.count && j < b.count {
            if a[i] == b[j] {
                ops.append(.equal(a[i]))
                i += 1
                j += 1
            } else if table[i + 1][j] >= table[i][j + 1] {
                ops.append(.removed(a[i]))
                i += 1
            } else {
                ops.append(.added(b[j]))
                j += 1
            }
        }
        while i < a.count { ops.append(.removed(a[i])); i += 1 }
        while j < b.count { ops.append(.added(b[j])); j += 1 }
        return ops
    }
}
