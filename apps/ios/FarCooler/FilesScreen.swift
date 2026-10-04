import SwiftUI

// The read-only Files browser (ov-259): a worktree's files, or one of the
// runner's extra read-only folders, a screen to a directory and a screen to a
// file, the way the system's own Files app drills down.
//
// A sheet with a stack of its own rather than a route over Needs You. A
// worktree covers the app's stack and each of its panes owns a
// `NavigationStack` (`ShellPaneBar`), and a stack inside another is drawn as
// neither, so Files brings its own and gives the app back on Done. What each
// screen shows is `FilesItemModel`'s, in AgentKit, where `swift test` reaches
// it: this file only draws.

/// What a Files sheet opens on, and the runner it reads.
struct FilesSheet: View {
    @ObservedObject var connection: Connection
    let root: FilesLocation
    @Environment(\.dismiss) private var dismiss
    @State private var path: [FilesLocation] = []

    private var source: FilesSource {
        FilesWire.source(
            call: { [connection] method, args in try await connection.rpc(method, args) },
            refusalWord: { ClientCore.refusalWord(of: $0) })
    }

    var body: some View {
        NavigationStack(path: $path) {
            FilesItemScreen(location: root, source: source, open: { path.append($0) })
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { dismiss() }
                            .accessibilityIdentifier("files-done")
                    }
                }
                .navigationDestination(for: FilesLocation.self) { location in
                    FilesItemScreen(location: location, source: source, open: { path.append($0) })
                }
        }
        .accessibilityIdentifier("files-sheet")
    }
}

/// One directory or one file.
struct FilesItemScreen: View {
    let location: FilesLocation
    let open: (FilesLocation) -> Void
    @StateObject private var model: FilesItemModel

    init(location: FilesLocation, source: FilesSource, open: @escaping (FilesLocation) -> Void) {
        self.location = location
        self.open = open
        _model = StateObject(wrappedValue: FilesItemModel(location: location, source: source))
    }

    var body: some View {
        Group {
            switch model.content {
            case .loading:
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("files-loading")
            case .directory(let directory):
                FilesDirectoryList(directory: directory, open: open)
            case .code(let code):
                FilesCodeView(code: code)
            case .message(let words):
                FilesNote(words: words)
            case .link(let target, let destination):
                FilesNote(words: "This is a link to \(target).") {
                    if let destination {
                        Button("Open") { open(destination) }
                            .buttonStyle(.borderedProminent)
                            .accessibilityIdentifier("files-open-link")
                    }
                }
            case .failed(let sentence):
                FilesNote(words: sentence) {
                    Button("Try Again") { Task { await model.load() } }
                        .buttonStyle(.bordered)
                        .accessibilityIdentifier("files-try-again")
                }
            }
        }
        .navigationTitle(location.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if !location.path.isEmpty {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        UIPasteboard.general.string = location.path
                    } label: {
                        Label("Copy Path", systemImage: "doc.on.doc")
                    }
                    .accessibilityIdentifier("files-copy-path")
                }
            }
        }
        .task { await model.load() }
    }
}

/// A directory's rows: folders and files as the runner ordered them.
private struct FilesDirectoryList: View {
    let directory: FilesDirectory
    let open: (FilesLocation) -> Void

    var body: some View {
        List {
            if let empty = directory.empty {
                Text(empty)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("files-empty")
            }
            ForEach(directory.rows) { row in
                if let destination = row.destination {
                    Button { open(destination) } label: { FilesRowLabel(row: row) }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("files-row-\(row.name)")
                } else {
                    FilesRowLabel(row: row)
                        .accessibilityIdentifier("files-row-\(row.name)")
                }
            }
            if let footer = directory.footer {
                Text(footer)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("files-footer")
            }
        }
        .listStyle(.plain)
    }
}

private struct FilesRowLabel: View {
    let row: FilesRow

    private var symbol: String {
        switch row.kind {
        case .directory: "folder"
        case .file: "doc.text"
        case .link: "link"
        case .other: "questionmark.square.dashed"
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .foregroundStyle(.secondary)
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(row.name).lineLimit(1).truncationMode(.middle)
                if !row.detail.isEmpty {
                    Text(row.detail)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            Spacer(minLength: 0)
            if row.kind == .directory || (row.kind == .link && row.destination != nil) {
                Image(systemName: "chevron.right")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
        }
        .contentShape(.rect)
        .frame(minHeight: PaneMetrics.target)
        .accessibilityElement(children: .combine)
    }
}

/// A file's text, numbered, in a monospaced face, never wrapped.
///
/// One scroll view that moves both ways, with rows drawn lazily and the width
/// stated from the longest line rather than measured: measuring makes a lazy
/// stack build every row (`FilesCode.widest`). No syntax coloring, as the
/// Mac's has none.
private struct FilesCodeView: View {
    let code: FilesCode
    @ScaledMetric(relativeTo: .footnote) private var size: CGFloat = 13

    /// The advance of one column of a monospaced face is a bit over half its size.
    private var column: CGFloat { size * 0.62 }
    private let gutterGap: CGFloat = 12
    private let edge: CGFloat = 16

    var body: some View {
        let gutter = CGFloat(code.gutterDigits) * column
        ScrollView([.vertical, .horizontal]) {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(Array(code.lines.enumerated()), id: \.offset) { index, line in
                    HStack(alignment: .firstTextBaseline, spacing: gutterGap) {
                        Text("\(index + 1)")
                            .foregroundStyle(.secondary)
                            .frame(width: gutter, alignment: .trailing)
                        Text(line.isEmpty ? " " : line)
                            .lineLimit(1)
                            .fixedSize(horizontal: true, vertical: false)
                    }
                    .font(.system(size: size, design: .monospaced))
                }
                if code.anyCut {
                    Text(FilesCode.cutNote)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .padding(.top, 12)
                        .accessibilityIdentifier("files-cut-note")
                }
            }
            .padding(.horizontal, edge)
            .padding(.vertical, 8)
            .frame(
                minWidth: gutter + gutterGap + CGFloat(code.widest) * column + 2 * edge,
                alignment: .leading)
        }
        .accessibilityIdentifier("files-code")
    }
}

/// A sentence in the middle of the screen, and what to do about it.
private struct FilesNote<Actions: View>: View {
    let words: String
    @ViewBuilder var actions: Actions

    init(words: String, @ViewBuilder actions: () -> Actions) {
        self.words = words
        self.actions = actions()
    }

    var body: some View {
        VStack(spacing: 16) {
            Text(words)
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("files-note")
            actions
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

extension FilesNote where Actions == EmptyView {
    init(words: String) { self.init(words: words) { EmptyView() } }
}
