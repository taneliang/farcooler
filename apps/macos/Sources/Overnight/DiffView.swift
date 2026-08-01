import AgentKit
import SwiftUI

/// A unified diff, computed client-side from the two full texts a tool call
/// carries.
///
/// No syntax highlighting — cut in the spec. What earns the pixels here is
/// which lines changed, not what language they are in; colouring keywords on
/// top of an add/remove background fights the one signal that actually
/// matters, and a diff view that also has opinions about Rust vs TOML is a
/// second thing to keep correct for no reader benefit.
struct DiffView: View {
    let diff: Diff

    /// Beyond this many lines, the diff opens collapsed. A four-line edit is
    /// worth seeing on arrival; a four-hundred-line rewrite is not something
    /// to scroll past to reach the message after it.
    private static let collapseThreshold = 20

    @State private var expanded = false

    private var lines: [DiffComputation.Line] {
        DiffComputation.compute(old: diff.oldText ?? "", new: diff.newText)
    }

    var body: some View {
        let rows = lines
        VStack(alignment: .leading, spacing: 6) {
            header(for: rows)

            if rows.count > Self.collapseThreshold && !expanded {
                Button {
                    expanded = true
                } label: {
                    Text("Show \(rows.count) lines")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            } else {
                diffBody(rows)
            }
        }
    }

    private func header(for rows: [DiffComputation.Line]) -> some View {
        let added = rows.filter { $0.kind == .added }.count
        let removed = rows.filter { $0.kind == .removed }.count
        return HStack(spacing: 6) {
            Text(diff.path)
                .font(.system(size: 11, weight: .medium, design: .monospaced))
                .lineLimit(1)
            Spacer(minLength: 8)
            if added > 0 {
                Text("+\(added)").foregroundStyle(.green)
            }
            if removed > 0 {
                Text("-\(removed)").foregroundStyle(.red)
            }
        }
        .font(.system(size: 10.5, design: .monospaced))
        .foregroundStyle(.secondary)
    }

    private func diffBody(_ rows: [DiffComputation.Line]) -> some View {
        // A fixed-width gutter for line numbers, so the code column lines up
        // whether a number is one digit or four.
        let gutterWidth: CGFloat = 34

        return VStack(alignment: .leading, spacing: 0) {
            ForEach(rows) { line in
                HStack(spacing: 0) {
                    Text(line.oldNumber.map(String.init) ?? "")
                        .frame(width: gutterWidth, alignment: .trailing)
                    Text(line.newNumber.map(String.init) ?? "")
                        .frame(width: gutterWidth, alignment: .trailing)
                    Text(line.kind.marker)
                        .frame(width: 14, alignment: .center)
                    Text(line.text.isEmpty ? " " : line.text)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .lineLimit(1)
                }
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(line.kind == .context ? .secondary : .primary)
                .padding(.horizontal, 4)
                .padding(.vertical, 1)
                .background(line.kind.background)
            }
        }
        .textSelection(.enabled)
        .padding(.vertical, 4)
        .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 6))
    }
}

/// The diffing itself, kept apart from the view so it has no SwiftUI in it.
enum DiffComputation {
    enum Kind { case context, added, removed }

    struct Line: Identifiable {
        let id: Int
        let kind: Kind
        let oldNumber: Int?
        let newNumber: Int?
        let text: String
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
    static func compute(old: String, new: String) -> [Line] {
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

extension DiffComputation.Kind {
    fileprivate var marker: String {
        switch self {
        case .context: return ""
        case .added: return "+"
        case .removed: return "-"
        }
    }

    fileprivate var background: Color {
        switch self {
        case .context: return .clear
        case .added: return .green.opacity(0.13)
        case .removed: return .red.opacity(0.13)
        }
    }
}
