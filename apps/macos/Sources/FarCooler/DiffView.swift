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
