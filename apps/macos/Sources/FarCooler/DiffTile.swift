import AgentKit
import SwiftUI

/// What this worktree changed.
///
/// One concept at two widths rather than two modes to learn. The jump bar along
/// the top is how you navigate when the tile is narrow; when it is wide that
/// same list is promoted into a column beside the diff. Same information, more
/// room, and the narrow layout teaches the wide one.
struct DiffTile: View {
    @ObservedObject var changes: ChangesStore

    /// Below this the file list is a bar; above it, a column.
    private static let wideEnough: CGFloat = 620

    var body: some View {
        GeometryReader { geo in
            VStack(spacing: 0) {
                header
                Divider()
                if changes.error != nil {
                    problem
                }
                if geo.size.width >= Self.wideEnough {
                    HStack(spacing: 0) {
                        fileColumn.frame(width: 200)
                        Divider()
                        diffBody
                    }
                } else {
                    VStack(spacing: 0) {
                        jumpBar
                        Divider()
                        diffBody
                    }
                }
            }
        }
        .task(id: changes.workspace.id) { await changes.load() }
    }

    // MARK: - Chrome

    private var header: some View {
        HStack(spacing: 8) {
            Picker("", selection: $changes.scope) {
                ForEach(DiffScope.allCases) { s in Text(s.label).tag(s) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            .onChange(of: changes.scope) { _, _ in
                Task { await changes.load(fresh: true) }
            }

            Spacer(minLength: 4)

            Text(changes.changeSet.branch)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .layoutPriority(-1)

            counts

            Button {
                Task { await changes.load(fresh: true) }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Read the worktree again")

            Button {
                Task { await changes.markRead() }
            } label: {
                Image(systemName: "checkmark.circle")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Mark Read")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }

    private var counts: some View {
        HStack(spacing: 4) {
            if changes.changeSet.insertions > 0 {
                Text("+\(changes.changeSet.insertions)").foregroundStyle(.green)
            }
            if changes.changeSet.deletions > 0 {
                Text("−\(changes.changeSet.deletions)").foregroundStyle(.red)
            }
            if changes.changeSet.isDirty {
                Circle().fill(.orange).frame(width: 5, height: 5)
            }
        }
        .font(.system(size: 10.5, design: .monospaced))
    }

    /// An older machine cannot do this at all, and that is worth saying rather
    /// than rendering as a worktree with no changes.
    private var problem: some View {
        let old = changes.client.changesSupported == false
        return VStack(alignment: .leading, spacing: 3) {
            Text(old ? "This machine can't show changes yet" : "Couldn't read this worktree")
                .font(.system(size: 11.5, weight: .medium))
            Text(
                old
                    ? "Its copy of Far Cooler is older than this. Update it in Settings › Machines."
                    : (changes.error ?? "")
            )
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(9)
        .background(.orange.opacity(0.12))
    }

    // MARK: - Navigation

    private var jumpBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(changes.changeSet.files) { f in
                    Button { Task { await changes.openFile(f.path) } } label: { chip(f) }
                        .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
        }
    }

    private func chip(_ f: ChangedFile) -> some View {
        HStack(spacing: 4) {
            Text(f.name).lineLimit(1)
            if f.binary {
                Text("bin").foregroundStyle(.secondary)
            } else {
                if f.insertions > 0 { Text("+\(f.insertions)").foregroundStyle(.green) }
                if f.deletions > 0 { Text("−\(f.deletions)").foregroundStyle(.red) }
            }
        }
        .font(.system(size: 10, design: .monospaced))
        .padding(.horizontal, 6)
        .padding(.vertical, 3)
        .background(
            changes.selectedFile == f.path
                ? AnyShapeStyle(Color.accentColor.opacity(0.18))
                : AnyShapeStyle(.quaternary.opacity(0.3)),
            in: Capsule())
    }

    private var fileColumn: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(changes.changeSet.files) { f in
                    Button { Task { await changes.openFile(f.path) } } label: {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(f.path)
                                .font(.system(size: 11, design: .monospaced))
                                .lineLimit(1)
                                .truncationMode(.head)
                            HStack(spacing: 4) {
                                if f.binary {
                                    Text("binary").foregroundStyle(.secondary)
                                } else {
                                    if f.insertions > 0 {
                                        Text("+\(f.insertions)").foregroundStyle(.green)
                                    }
                                    if f.deletions > 0 {
                                        Text("−\(f.deletions)").foregroundStyle(.red)
                                    }
                                }
                            }
                            .font(.system(size: 10, design: .monospaced))
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(
                            changes.selectedFile == f.path
                                ? Color.accentColor.opacity(0.15) : .clear,
                            in: RoundedRectangle(cornerRadius: 4))
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(6)
        }
        .scrollContentBackground(.hidden)
    }

    // MARK: - The diff

    @ViewBuilder
    private var diffBody: some View {
        if changes.changeSet.files.isEmpty {
            TilePlaceholder(
                title: "Nothing changed here",
                detail: "This branch matches \(changes.changeSet.baseRef).")
        } else if changes.selectedFile == nil {
            TilePlaceholder(title: "Pick a file", detail: "Everything it changed, in one scroll.")
        } else {
            ScrollView {
                // Code does not wrap, so the diff scrolls sideways on its own
                // rather than making the whole tile do it.
                ScrollView(.horizontal, showsIndicators: false) {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(changes.diff) { line in row(line) }
                    }
                }
            }
            .scrollContentBackground(.hidden)
        }
    }

    private func row(_ line: DiffComputation.Line) -> some View {
        HStack(spacing: 0) {
            Text(line.oldNumber.map(String.init) ?? "")
                .frame(width: 30, alignment: .trailing)
                .foregroundStyle(.tertiary)
            Text(line.newNumber.map(String.init) ?? "")
                .frame(width: 30, alignment: .trailing)
                .foregroundStyle(.tertiary)
            Text(marker(line.kind)).frame(width: 12)
            Text(line.text.isEmpty ? " " : line.text)
                .fixedSize(horizontal: true, vertical: false)
            Spacer(minLength: 0)
        }
        .font(.system(size: 10.5, design: .monospaced))
        .background(background(line.kind))
    }

    // No syntax highlighting, still. What earns the pixels is which lines
    // changed, and colouring keywords on top of an add/remove background fights
    // the one signal that matters.
    private func marker(_ kind: DiffComputation.Kind) -> String {
        switch kind {
        case .added: return "+"
        case .removed: return "−"
        case .context: return " "
        }
    }

    private func background(_ kind: DiffComputation.Kind) -> Color {
        switch kind {
        case .added: return .green.opacity(0.13)
        case .removed: return .red.opacity(0.13)
        case .context: return .clear
        }
    }
}
