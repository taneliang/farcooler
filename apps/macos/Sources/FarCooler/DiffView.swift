import AgentKit
import SwiftUI

/// A unified diff, computed client-side from the two full texts a tool call
/// carries.
///
/// No syntax highlighting — cut in the spec. What earns the pixels here is
/// which lines changed, not what language they are in; coloring keywords on
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
                Text(DiffCounts.added(added)).foregroundStyle(.green)
            }
            if removed > 0 {
                Text(DiffCounts.removed(removed)).foregroundStyle(.red)
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
                    // The stripe the Changes pane draws, for the reason it
                    // draws it — see `DiffComputation.Kind.accent`.
                    Rectangle()
                        .fill(line.kind.accent)
                        .frame(width: 2)
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
                    .padding(.horizontal, 4)
                }
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(line.kind == .context ? .secondary : .primary)
                .padding(.vertical, 1)
                .background(line.kind.wash)
            }
        }
        .textSelection(.enabled)
        .padding(.vertical, 4)
        .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 6))
    }
}


/// The diff's own vocabulary: what marks a line, what washes it, and what runs
/// down its left edge.
///
/// Here rather than inside either view, because the Mac has two renderers of
/// the same model — this one, for the diff a tool call carries, and the Changes
/// pane's — and they can be on screen at the same time inside one window. They
/// had drifted into three disagreements: a wash of 0.13 against 0.07, an ASCII
/// hyphen against U+2212, and an accent stripe in one and not the other. Two
/// pictures of one diff, half a window apart.
///
/// The Changes pane's reading wins on all three. It is the one that reads a
/// whole repository rather than a snippet, its lighter wash is what lets a
/// hundred changed lines in a row stay readable, and the stripe is what carries
/// the signal at that alpha. U+2212 wins because it is the width of `+` in a
/// monospaced column, where an ASCII hyphen is not and the markers go ragged.
///
/// It belongs one layer down, beside `DiffComputation.Kind` in AgentKit, where
/// the phones' renderers could read it too. That is a shared-module change and
/// is deliberately not made here; this is the same table, in one place, ready
/// to move as a piece.
extension DiffComputation.Kind {
    var marker: String {
        switch self {
        case .context: return " "
        case .added: return "+"
        case .removed: return "\u{2212}"
        }
    }

    /// The wash over the whole line.
    var wash: Color {
        switch self {
        case .context: return .clear
        case .added: return .green.opacity(0.07)
        case .removed: return .red.opacity(0.07)
        }
    }

    /// The stripe down the line's leading edge, which is what makes a 7% wash
    /// legible without turning the page into a color field.
    var accent: Color {
        switch self {
        case .context: return .clear
        case .added: return .green.opacity(0.62)
        case .removed: return .red.opacity(0.62)
        }
    }
}

/// `+N` and `\u{2212}M`, spelled once.
///
/// The Mac had five spellings of the same pair — four sizes, two color
/// treatments, thousands separators in exactly one of the five, and the minus
/// scrambled between an ASCII hyphen and U+2212 — and all five can be on
/// screen at the same moment. Size and color still belong to the site that
/// draws it; the CHARACTERS do not, and they are here.
///
/// U+2212 because it is the width of `+` in a monospaced column and an ASCII
/// hyphen is not, so a column of pairs went ragged on the minus side. Thousands
/// separators everywhere, because a lockfile's `+35870` is a number nobody
/// reads at a glance and `+35,870` is.
///
/// Like the kind table above, this wants to be in AgentKit next to
/// `DiffComputation` so the phones stop spelling it their own way. That is a
/// shared-module change and is deliberately not made here.
enum DiffCounts {
    /// The minus. Not `-`.
    static let minus = "\u{2212}"

    static func added(_ n: Int) -> String { "+\(n.formatted())" }
    static func removed(_ n: Int) -> String { "\(minus)\(n.formatted())" }

    /// Both, separated by a space — what a column cell and a tooltip both say.
    static func pair(insertions: Int, deletions: Int) -> String {
        "\(added(insertions)) \(removed(deletions))"
    }
}
