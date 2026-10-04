import AgentKit
import AppKit
import SwiftUI

// The title bar's field (ov-214, slice 4; ov-264): the status area's activity
// line at rest; ⌘K, ⌘P, or a click on it, makes it a field that searches and
// goes to things. Typing always searches: Go to Anything, the palette's
// results, in place, then the files of the worktree on screen whose paths
// match (ov-189). It sends nothing anywhere: the orchestrator is talked to in
// its own pane, and "Go to Orchestrator" is one of the results.
//
//   ⌘K     opens it; with nothing typed, the panel under it shows the
//          workspace's activity.
//   ⌘P     opens it; with nothing typed, it lists the recent terminals.
//   Esc    back to the activity line. A search isn't kept.
//
// The rules are this value's, so `TitleConsoleTests` pins them; the views
// draw it.

/// The field's state.
struct TitleConsole: Equatable {
    enum Mode: Equatable {
        case rest
        /// Open, with nothing typed, from ⌘K: the activity.
        case activity
        /// Open and searching: what's typed, or the recent terminals.
        case find
    }

    private(set) var isOpen = false
    /// What's in the field: the query.
    private(set) var text = ""
    /// Opened by ⌘P: with nothing typed, the recent terminals rather than
    /// the activity.
    private(set) var recents = false
    /// The result the highlight is on, once ↑ or ↓ has moved it.
    private(set) var highlight = 0
    /// Whether ↑ or ↓ has moved the highlight since the results changed:
    /// until then it's where the results say to start (`opening`).
    private(set) var moved = false
    /// The workspace on screen (`host|workspace`), for what's kept by it.
    private(set) var workspace = ""
    /// What the runner found in the files of the worktree on screen (ov-189),
    /// and the query it found them for: listed only while that's still the
    /// query, so a slow answer never lists files for what was typed before.
    private(set) var fileHits: [String] = []
    private(set) var fileHitsQuery: String?

    var mode: Mode {
        guard isOpen else { return .rest }
        return query.isEmpty && !recents ? .activity : .find
    }

    /// What's searched for.
    var query: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    static let placeholder = "Find a workspace, task, terminal, or file"

    /// The window moved to workspace `key`: the field closes.
    mutating func enter(workspace key: String) {
        guard key != workspace else { return }
        workspace = key
        close()
    }

    /// ⌘K, a click on the activity line, or Show Activity: open, the
    /// activity under it. ⌘P (`recents`): open, the recent terminals under it.
    mutating func open(recents: Bool) {
        resetHighlight()
        self.recents = recents
        isOpen = true
    }

    /// ⌘P or ⌘K again on a field already open that way closes it, as every
    /// switcher on the Mac does.
    mutating func toggle(recents: Bool) {
        if isOpen, self.recents == recents { close() } else { open(recents: recents) }
    }

    /// Esc, the field losing the keyboard, or a result opened: back to the
    /// activity line, the search gone.
    mutating func close() {
        isOpen = false
        text = ""
        recents = false
        fileHits = []
        fileHitsQuery = nil
        resetHighlight()
    }

    /// The files found for `query`, kept only if it's still what's searched.
    mutating func found(files: [String], for query: String) {
        guard mode == .find, query == self.query else { return }
        fileHits = files
        fileHitsQuery = query
    }

    /// The files found for what's searched now: none while that search is
    /// still out, or with nothing typed.
    var foundFiles: [String] {
        mode == .find && !query.isEmpty && fileHitsQuery == query ? fileHits : []
    }

    /// What was typed. Typing searches; it never does anything else.
    mutating func edit(_ new: String) {
        text = new
        resetHighlight()
    }

    private mutating func resetHighlight() {
        highlight = 0
        moved = false
    }

    /// Where the highlight is, given where the results say to start.
    func highlight(opening: Int) -> Int { moved ? highlight : opening }

    /// ↑ ↓, Tab and Shift-Tab through `count` results, wrapping, from where
    /// the highlight is.
    mutating func move(_ by: Int, count: Int, opening: Int = 0) {
        guard mode == .find, count > 0 else { return }
        highlight = ((highlight(opening: opening) + by) % count + count) % count
        moved = true
    }

    /// Return: the index of the result to open, of `results` listed whose
    /// highlight starts at `opening`; nil with nothing to open.
    func submit(results: Int, opening: Int = 0) -> Int? {
        guard mode == .find, results > 0 else { return nil }
        return min(highlight(opening: opening), results - 1)
    }

    /// A result was opened: the field closes.
    mutating func opened() { close() }
}

/// The field's state, owned by the window so its commands reach it.
@MainActor @Observable final class TitleConsoleModel {
    var console = TitleConsole()
    /// The pointer is over the panel under the field: losing the keyboard to
    /// a click there doesn't close the field (`fieldEndedEditing`).
    var pointerInPanel = false
    /// Today's spend, by workspace, and when it was read: read at most once
    /// a minute, not each time the panel comes back.
    var spend: [String: (at: Date, spend: ActivitySpend)] = [:]

    /// The field lost the keyboard: closed, unless it went to a click in
    /// the panel under it, whose rows act on their own.
    func fieldEndedEditing() {
        if !pointerInPanel { console.close() }
    }
}

/// What the field does, routed by the window.
struct TitleConsoleActions {
    /// Find results for a query; the recent terminals for an empty one.
    var find: (String) -> [PaletteEntry] = { _ in [] }
    var run: (PaletteAction) -> Void = { _ in }
    /// The files find searches (ov-189): the worktree on screen's, or nil
    /// with none, or on a runner too old to show files.
    var files: PaletteFiles? = nil
    /// The terminal the window has the keyboard in, so finding with nothing
    /// typed starts on the one before it, as ⌘P and Return always went back.
    var current: String? = nil

    func entries(_ console: TitleConsole) -> [PaletteEntry] {
        guard console.mode == .find else { return [] }
        let files = files.map { f in console.foundFiles.map(f.entry) } ?? []
        return find(console.query) + files
    }

    /// Where the highlight starts in `entries`: on the second when the
    /// first is the terminal you're already in, with nothing typed. Alt-Tab's
    /// rule, kept from the palette: ⌘P then Return is "the other one".
    func opening(_ console: TitleConsole, _ entries: [PaletteEntry]) -> Int {
        guard console.query.isEmpty, let current, entries.count > 1 else { return 0 }
        if case .openTerminal(_, let terminal) = entries[0].action, terminal == current { return 1 }
        return 0
    }

    /// Return, carried out: the highlighted result, opened.
    @MainActor
    func submit(_ model: TitleConsoleModel) {
        let results = entries(model.console)
        guard let index = model.console.submit(results: results.count, opening: opening(model.console, results))
        else { return }
        let action = results[index].action
        model.console.opened()
        run(action)
    }
}

/// The field itself, in the status area's place while it's open.
struct TitleConsoleField: View {
    let model: TitleConsoleModel
    let actions: TitleConsoleActions

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .frame(width: 14)
            PaletteField(
                text: Binding(get: { model.console.text }, set: { model.console.edit($0) }),
                placeholder: TitleConsole.placeholder,
                horizontalMoves: false, fontSize: NSFont.systemFontSize,
                onMove: { move in
                    let entries = actions.entries(model.console)
                    let start = actions.opening(model.console, entries)
                    switch move {
                    case .up, .previous, .left: model.console.move(-1, count: entries.count, opening: start)
                    case .down, .next, .right: model.console.move(1, count: entries.count, opening: start)
                    }
                    if let entry = entries[safe: model.console.highlight(opening: start)] {
                        // The highlight moved without the keyboard leaving the
                        // field: VoiceOver says where it landed.
                        AccessibilityNotification.Announcement(entry.title).post()
                    }
                },
                onSubmit: { actions.submit(model) },
                onCancel: { model.console.close() },
                onEndEditing: { model.fieldEndedEditing() })
            .frame(height: 22)
            .accessibilityLabel("Find")
            .accessibilityIdentifier("title-console-field")
        }
        .padding(.horizontal, Spacing.tight + 2)
        .frame(height: 28)
        .surface(.inset, in: .control)
    }
}

extension Array {
    /// The element at `index`, or nil past either end.
    subscript(safe index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}

/// Under the field: the activity from ⌘K with nothing typed; else the
/// results.
struct TitleConsolePanel: View {
    let model: TitleConsoleModel
    let actions: TitleConsoleActions
    let activity: TitleActivity
    let status: TitleStatusActions
    /// Whether the field is drawn here too, for a status area too narrow to
    /// hold it.
    var showsField = false

    var body: some View {
        let console = model.console
        VStack(alignment: .leading, spacing: 0) {
            if showsField {
                TitleConsoleField(model: model, actions: actions).padding(Spacing.group)
            }
            switch console.mode {
            case .rest:
                EmptyView()
            case .activity:
                TitleActivityPopover(activity: activity, actions: status, cache: model) { model.console.close() }
            case .find:
                results(console)
            }
            footer(console.mode)
        }
        .frame(width: 420, alignment: .leading)
        .surface(.floating, in: .floating)
        // A click here mustn't close the field it belongs to
        // (`TitleConsoleModel.fieldEndedEditing`).
        .onHover { inside in model.pointerInPanel = inside }
        .onDisappear { model.pointerInPanel = false }
        // A file search on the runner, a beat after the last keystroke: each
        // keystroke cancels the one before (ov-189).
        .task(id: console.mode == .find ? console.query : nil) {
            guard console.mode == .find, !console.query.isEmpty, let files = actions.files else { return }
            let query = console.query
            try? await Task.sleep(for: .milliseconds(120))
            guard !Task.isCancelled else { return }
            let found = await files.search(query)
            if !Task.isCancelled { model.console.found(files: found, for: query) }
        }
    }

    @ViewBuilder
    private func results(_ console: TitleConsole) -> some View {
        let entries = actions.entries(console)
        let lit = console.highlight(opening: actions.opening(console, entries))
        if entries.isEmpty {
            Text(console.query.isEmpty ? "Nothing running" : "No Results for “\(console.query)”")
                .font(.callout)
                .foregroundStyle(.secondary)
                .padding(Spacing.inset)
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 1) {
                        ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                            Button {
                                model.console.opened()
                                actions.run(entry.action)
                            } label: {
                                PaletteRow(entry: entry, isHighlighted: index == lit)
                            }
                            .buttonStyle(.plain)
                            .id(entry.id)
                            .probed("title-console-result")
                        }
                    }
                    .padding(Spacing.group)
                }
                .frame(maxHeight: 320)
                .onChange(of: lit) { _, index in
                    guard entries.indices.contains(index) else { return }
                    proxy.scrollTo(entries[index].id)
                }
            }
        }
    }

    private func footer(_ mode: TitleConsole.Mode) -> some View {
        HStack(spacing: Spacing.group) {
            if mode == .find {
                KeyHint(keys: "↑↓", label: "Move")
                KeyHint(keys: "↩", label: "Open")
            } else {
                Text("Type to find anything")
            }
            Spacer(minLength: 0)
            KeyHint(keys: "⎋", label: "Close")
        }
        .font(.subheadline)
        .foregroundStyle(.tertiary)
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.tight + 2)
    }
}

/// One find result: the palette's row, as it was.
struct PaletteRow: View {
    let entry: PaletteEntry
    let isHighlighted: Bool

    var body: some View {
        HStack(spacing: 9) {
            Group {
                if let terminal = entry.terminal {
                    StatusGlyph(status: terminal.status)
                } else if let symbol = entry.symbol {
                    Image(systemName: symbol)
                        .font(.subheadline)
                        .foregroundStyle(isHighlighted ? AnyShapeStyle(.white) : AnyShapeStyle(.secondary))
                }
            }
            .frame(width: 14)

            VStack(alignment: .leading, spacing: 1) {
                Text(entry.title)
                    .font(.body)
                    .foregroundStyle(isHighlighted ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
                    .lineLimit(1)
                if !entry.detail.isEmpty {
                    Text(entry.detail)
                        .font(.subheadline)
                        .foregroundStyle(isHighlighted ? AnyShapeStyle(.white) : AnyShapeStyle(.secondary))
                        .lineLimit(1)
                }
            }

            Spacer(minLength: 8)

            Text(entry.kind)
                .font(.footnote)
                .foregroundStyle(isHighlighted ? AnyShapeStyle(.white) : AnyShapeStyle(.tertiary))
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        // The Mac's own list selection: an accent fill and light text.
        .background(isHighlighted ? Color.accentColor : Color.clear, in: .control)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isHighlighted ? .isSelected : [])
    }
}

/// The panel under the title bar, with its activity read from the
/// workspace's board as the status area reads it.
struct TitleConsoleDropdown: View {
    let model: TitleConsoleModel
    let actions: TitleConsoleActions
    let status: TitleStatusActions
    let source: TitleStatusSource
    var showsField = false

    var body: some View {
        Group {
            if let store = source.board {
                Observed(
                    store: store, model: model, actions: actions, status: status, source: source, showsField: showsField)
            } else {
                TitleConsolePanel(
                    model: model, actions: actions, activity: TitleStatus.activity(source, board: .empty, starts: [:]),
                    status: status, showsField: showsField)
            }
        }
        .environment(\.statusGlyphStill, true)
    }

    private struct Observed: View {
        @ObservedObject var store: TaskBoardStore
        let model: TitleConsoleModel
        let actions: TitleConsoleActions
        let status: TitleStatusActions
        let source: TitleStatusSource
        let showsField: Bool

        var body: some View {
            TitleConsolePanel(
                model: model, actions: actions,
                activity: TitleStatus.activity(source, board: store.board, starts: store.starts), status: status,
                showsField: showsField)
        }
    }
}
