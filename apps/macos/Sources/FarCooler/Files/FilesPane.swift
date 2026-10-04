import AgentKit
import AppKit
import SwiftUI

/// A worktree's files, read-only (ov-189): the tree on the left, filterable,
/// and the file chosen on the right, in `CodeView`.
///
/// A viewer, not an editor: agents edit, and Open in Editor is a click away
/// for the rare time a person wants to. `.env` and every other file is shown
/// as it is (owner, 3 Oct): the transport is ssh, and a viewer that hid what
/// an editor shows would only send people to the editor.
struct FilesPane: View {
    @ObservedObject var model: FilesModel
    /// The pane was clicked into: ⌘F and ⇧⌘L are for it now.
    var onFocus: () -> Void = {}
    /// Hide it, where it's an inspector rather than a tab.
    var onClose: (() -> Void)?

    @ObservedObject private var preferences = Preferences.shared
    @State private var editorFailure: String?
    @State private var lineField = ""
    @FocusState private var findFocused: Bool
    @FocusState private var lineFocused: Bool

    var body: some View {
        GeometryReader { geo in
            HStack(spacing: 0) {
                FilesTree(model: model)
                    .frame(width: min(280, max(180, geo.size.width * 0.3)))
                    .separator(.split, edge: .trailing)
                VStack(spacing: 0) {
                    header
                    if model.finding { findBar }
                    if model.goingToLine { lineBar }
                    document
                }
            }
        }
        .background(WorkspaceStyle.paper)
        .task(id: model.place.id) { await model.loadIfNeeded() }
        .simultaneousGesture(TapGesture().onEnded { onFocus() })
        .onChange(of: model.finding) { _, on in findFocused = on }
        .onChange(of: model.goingToLine) { _, on in lineFocused = on }
        .alert(
            "Couldn’t Open the Editor",
            isPresented: Binding(get: { editorFailure != nil }, set: { if !$0 { editorFailure = nil } })
        ) {
            Button("OK") { editorFailure = nil }
        } message: {
            Text(editorFailure ?? "")
        }
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 8) {
            if let opened = model.opened {
                Text(opened.path)
                    .font(.system(size: WorkspaceStyle.PaneText.body, weight: .semibold).monospaced())
                    .lineLimit(1)
                    .truncationMode(.head)
                    .textSelection(.enabled)
                    .help(opened.path)
                if let size = sizeNote(opened.content) {
                    Text(size)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .fixedSize()
                }
            } else {
                Text(model.place.title)
                    .font(WorkspaceStyle.paneTitle)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Button {
                model.finding.toggle()
            } label: {
                Image(systemName: "magnifyingglass")
            }
            .buttonStyle(.borderless)
            .help("Find in this file (⌘F)")
            .disabled(model.lines.isEmpty)
            Button {
                model.goingToLine.toggle()
            } label: {
                Image(systemName: "number")
            }
            .buttonStyle(.borderless)
            .help("Go to a line (⇧⌘L)")
            .disabled(model.lines.isEmpty)
            Menu {
                Button("Copy Path") { copy(model.opened?.path ?? "") }
                    .disabled(model.opened == nil)
                Button("Copy Reference") {
                    if let path = model.opened?.path { copy(FilesLogic.reference(path, range: model.selection)) }
                }
                .disabled(model.opened == nil)
                Button("Copy Lines") { copy(model.copiedText ?? "") }
                    .disabled(model.copiedText == nil)
                Divider()  // style-exempt: menu
                // A folder's files are read here only: its path isn't one the
                // editor is told.
                if model.worktree != nil {
                    Button("Open in Editor") { openInEditor() }
                        .disabled(model.opened == nil)
                }
                Button("Reload") { Task { await model.reload() } }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("More file actions")
            if let onClose {
                Button(action: onClose) {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
                .help("Hide files")
            }
        }
        .padding(.horizontal, 10)
        .frame(height: WorkspaceStyle.paneHeaderHeight)
        .background(WorkspaceStyle.fileHeader)
    }

    private func sizeNote(_ content: FilesModel.Opened.Content) -> String? {
        switch content {
        case .text(let lines): return lines.count == 1 ? "1 line" : "\(lines.count.formatted()) lines"
        case .binary(let size), .tooLarge(let size): return FilesLogic.size(size)
        default: return nil
        }
    }

    // MARK: - Find and go to line

    private var findBar: some View {
        HStack(spacing: 8) {
            TextField("Find in File", text: $model.findQuery)
                .textFieldStyle(.roundedBorder)
                .controlSize(.small)
                .focused($findFocused)
                .onAppear { findFocused = true }
                .onSubmit { model.step(forward: !NSEvent.modifierFlags.contains(.shift)) }
                .onExitCommand { model.finding = false }
                .frame(maxWidth: 280)
            Text(findCount)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
                .monospacedDigit()
                .fixedSize()
            Button {
                model.step(forward: false)
            } label: {
                Image(systemName: "chevron.up")
            }
            .buttonStyle(.borderless)
            .help("Previous match (⇧↩)")
            .disabled(model.matches.isEmpty)
            Button {
                model.step(forward: true)
            } label: {
                Image(systemName: "chevron.down")
            }
            .buttonStyle(.borderless)
            .help("Next match (↩)")
            .disabled(model.matches.isEmpty)
            Spacer(minLength: 0)
            Button("Done") { model.finding = false }
                .controlSize(.small)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(WorkspaceStyle.fileHeader)
        .onChange(of: model.findQuery) { _, _ in model.refind() }
    }

    private var findCount: String {
        guard !model.findQuery.isEmpty else { return "" }
        guard let current = model.currentMatch else { return "Not found" }
        return "\((current + 1).formatted()) of \(model.matches.count.formatted())"
    }

    private var lineBar: some View {
        HStack(spacing: 8) {
            TextField("Line", text: $lineField)
                .textFieldStyle(.roundedBorder)
                .controlSize(.small)
                .focused($lineFocused)
                .onAppear { lineFocused = true }
                .frame(width: 90)
                .onSubmit {
                    if let line = FilesLogic.line(from: lineField, count: model.lines.count) {
                        model.go(toLine: line)
                        model.goingToLine = false
                        lineField = ""
                    }
                }
                .onExitCommand { model.goingToLine = false }
            Text("of \(model.lines.count.formatted())")
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
            Button("Done") { model.goingToLine = false }
                .controlSize(.small)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(WorkspaceStyle.fileHeader)
    }

    // MARK: - The file

    @ViewBuilder
    private var document: some View {
        switch model.opened?.content {
        case nil:
            if model.folderGone {
                PaneNotice(title: "Folder Unavailable", detail: FileReadFailure.folderGone.sentence)
            } else {
                PaneNotice(title: "No File Open", detail: "Choose a file to read it here.")
            }
        case .loading?:
            // A read in flight isn't an answer (`ChangesPane.diffBody`).
            Color.clear
        case .text(let lines)?:
            if lines.isEmpty {
                PaneNotice(title: "Empty File", detail: "This file has nothing in it.")
            } else {
                CodeView(
                    lines: lines, widest: model.widest, font: preferences.terminalFont(),
                    selection: model.selection, matches: model.matches, currentMatch: model.currentMatch,
                    scroll: model.scroll,
                    onClickLine: { line, extending in model.click(line: line, extending: extending) })
                    .onCopyCommand {
                        guard let text = model.copiedText else { return [] }
                        return [NSItemProvider(object: text as NSString)]
                    }
            }
        case .binary(let size)?:
            notice(
                title: "Binary File",
                detail: "This file is \(FilesLogic.size(size)) and isn’t text. Open it in an editor to see it.")
        case .tooLarge(let size)?:
            notice(
                title: "Too Large to Show Here",
                detail: "This file is \(FilesLogic.size(size)). Far Cooler shows files up to 512 KB.")
        case .link(let target, let inside)?:
            VStack(spacing: 10) {
                PaneNotice(
                    title: "Link",
                    detail: inside == nil
                        ? "This links to \(target), which is outside this \(model.worktree == nil ? "folder" : "worktree")."
                        : "This links to \(target).")
                    .fixedSize(horizontal: false, vertical: true)
                if let inside {
                    Button("Open \(inside)") { Task { await model.open(inside, line: nil) } }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .failed(let why)?:
            VStack(spacing: 10) {
                PaneNotice(title: "Can’t Show This File", detail: why.sentence)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Try Again") {
                    if let path = model.opened?.path { Task { await model.open(path, line: nil, reveal: false) } }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func notice(title: String, detail: String) -> some View {
        VStack(spacing: 10) {
            PaneNotice(title: title, detail: detail)
                .fixedSize(horizontal: false, vertical: true)
            // A folder's files are read here only: its path isn't one the
            // editor is told.
            if model.worktree != nil { Button("Open in Editor") { openInEditor() } }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Actions

    private func copy(_ text: String) {
        guard !text.isEmpty else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    private func openInEditor() {
        guard let path = model.opened?.path, let worktree = model.worktree else { return }
        Task { editorFailure = await Editors.shared.open(path, in: worktree) }
    }
}

/// The tree, and the filter over it.
private struct FilesTree: View {
    @ObservedObject var model: FilesModel

    var body: some View {
        VStack(spacing: 0) {
            // Searching is the runner's, over a worktree: an extra folder has
            // nothing to search, so it has no filter.
            if model.canFilter {
                TextField("Filter", text: $model.filter)
                    .textFieldStyle(.roundedBorder)
                    .controlSize(.small)
                    .padding(8)
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if model.filter.trimmingCharacters(in: .whitespaces).isEmpty {
                        tree
                    } else {
                        filtered
                    }
                }
                .padding(.vertical, 4)
            }
        }
        // A filter typed on the runner, a beat after the last keystroke:
        // each keystroke cancels the one before, so a word typed fast is one
        // search, not five.
        .task(id: model.filter) {
            try? await Task.sleep(for: .milliseconds(180))
            guard !Task.isCancelled else { return }
            await model.runFilter()
        }
    }

    @ViewBuilder
    private var tree: some View {
        // A folder that's gone says so beside the file, not here too.
        if case .failed(let why)? = model.folders[""], !model.folderGone {
            Text(why.sentence)
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .foregroundStyle(.secondary)
                .padding(10)
        }
        ForEach(model.rows) { row in
            Button {
                choose(row)
            } label: {
                TreeRowLabel(row: row, selected: model.opened?.path == row.id, loading: model.folders[row.id] == .loading)
            }
            .buttonStyle(.plain)
            .disabled(row.kind == .other)
        }
        if model.listings[""]?.truncated == true {
            Text("Only the first 5,000 items are listed.")
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
                .padding(10)
        }
    }

    @ViewBuilder
    private var filtered: some View {
        if model.found.isEmpty {
            Text("No Matching Files")
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .foregroundStyle(.secondary)
                .padding(10)
        }
        ForEach(model.found, id: \.self) { path in
            Button {
                Task { await model.open(path, line: nil) }
            } label: {
                VStack(alignment: .leading, spacing: 1) {
                    Text((path as NSString).lastPathComponent)
                        .font(.system(size: WorkspaceStyle.PaneText.body))
                    Text(path)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary).monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.head)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 3)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(model.opened?.path == path ? WorkspaceStyle.navigatorSelection : .clear)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private func choose(_ row: FilesLogic.TreeRow) {
        switch row.kind {
        case .directory: Task { await model.toggle(row.id) }
        case .file, .link: Task { await model.open(row.id, line: nil, reveal: false) }
        case .other: break
        }
    }
}

/// One row of the tree: indent, chevron, symbol, name.
private struct TreeRowLabel: View {
    let row: FilesLogic.TreeRow
    let selected: Bool
    let loading: Bool

    var body: some View {
        HStack(spacing: 4) {
            // The app's one disclosure chevron (`CollapsibleSection`), drawn
            // for a folder only; the whole row is the toggle.
            DisclosureChevron(expanded: row.expanded, visible: row.kind == .directory)
                .frame(width: 10)
            Image(systemName: symbol)
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
                .frame(width: 14)
            Text(row.name)
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .lineLimit(1)
                .truncationMode(.middle)
                .foregroundStyle(row.kind == .other ? .secondary : .primary)
            Spacer(minLength: 0)
        }
        .padding(.leading, 6 + CGFloat(row.depth) * 12)
        .padding(.trailing, 8)
        .padding(.vertical, 2)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(selected ? WorkspaceStyle.navigatorSelection : .clear)
        .contentShape(Rectangle())
        .help(row.kind == .link ? "\(row.name) → \(row.linkTarget)" : row.id)
        .accessibilityLabel(accessibility)
    }

    private var symbol: String {
        switch row.kind {
        case .directory: return row.expanded ? "folder.fill" : "folder"
        case .link: return "arrow.turn.up.right"
        case .file: return "doc"
        case .other: return "questionmark.square.dashed"
        }
    }

    private var accessibility: String {
        switch row.kind {
        case .directory: return "\(row.name), folder, \(row.expanded ? "expanded" : "collapsed")"
        case .link: return "\(row.name), link to \(row.linkTarget)"
        case .file: return row.name
        case .other: return "\(row.name), can’t be shown"
        }
    }
}

extension Editors {
    /// Open one file of `worktree` in the editor a click would use for it,
    /// and say what went wrong.
    func open(_ relative: String, in worktree: Worktree) async -> String? {
        let host = worktree.host ?? ""
        guard let editor = preferred(host: host) else {
            return "No editor that can open this runner’s files is set up. Add one in Settings > Editors."
        }
        guard let argv = editor.command(path: FilesLogic.join(worktree.path, relative), host: host) else {
            return editor.unavailability(host: host) ?? "Couldn’t open \(editor.name)."
        }
        return await EditorLaunch.run(argv)
    }
}
