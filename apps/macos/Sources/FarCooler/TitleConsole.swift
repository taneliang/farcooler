import AgentKit
import AppKit
import SwiftUI

// The title bar's field (ov-214, slice 4): the status area's activity line
// at rest; ⌘K, or a click on it, makes it a field.
//
//   ask    the default. Return sends what's typed to this workspace's
//          orchestrator, by the route its composer uses
//          (`terminal agent-prompt`), and only ever to the orchestrator.
//          Empty, the panel under it shows the activity (slice 2).
//   find   `/` at the start, or ⌘P, which opens it with `/` already typed:
//          Go to Anything, the palette's results, in place. There's no
//          separate floating palette any more.
//   Esc    back to the activity line. A message being written is kept for
//          the next ⌘K; a search is not.
//
// Never sent by accident: typing alone never sends (only Return does, and
// never mid-composition in an input method), an empty or whitespace-only
// message is refused with a sentence, and a second Return while one is on
// its way does nothing.
//
// The rules are this value's, so `TitleConsoleTests` pins them; the views
// draw it.

/// The field's state.
struct TitleConsole: Equatable {
    enum Mode: Equatable { case rest, ask, find }

    /// What a Return does.
    enum Intent: Equatable {
        case none
        /// Send this, trimmed, to the orchestrator.
        case send(String)
        /// Refused, with the sentence now in `notice`.
        case refuse
        /// Open the find result at this index.
        case open(Int)
    }

    private(set) var isOpen = false
    /// What's in the field.
    private(set) var text = ""
    /// The result the highlight is on, in find mode.
    private(set) var highlight = 0
    /// What the last Return refused or failed with; typing clears it.
    private(set) var notice: String?
    /// A message is on its way.
    private(set) var sending = false
    /// A message set aside while finding, given back by the next ⌘K.
    private(set) var draft = ""

    var mode: Mode {
        guard isOpen else { return .rest }
        return text.hasPrefix("/") ? .find : .ask
    }

    /// What find mode searches for: the text after its `/`.
    var query: String { String(text.dropFirst()).trimmingCharacters(in: .whitespaces) }

    /// What ask mode would send.
    var message: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    static let empty = "Type a message first."
    static let sendFailed = "Couldn’t send that to the orchestrator. Your message is still here."
    static let askPlaceholder = "Ask the orchestrator, or type / to find"
    static let findPlaceholder = "Find a workspace, task, or terminal"

    /// ⌘K, a click on the activity line, or Show Activity: open to ask,
    /// with any message set aside given back. ⌘P (`finding`): open to
    /// find, with `/` typed, setting a message being written aside.
    mutating func open(finding: Bool) {
        notice = nil
        highlight = 0
        if finding, !text.hasPrefix("/") {
            if !message.isEmpty { draft = text }
            text = "/"
        } else if !finding, text.hasPrefix("/") {
            text = draft
            draft = ""
        }
        isOpen = true
    }

    /// ⌘P or ⌘K again on a field already open in that mode closes it, as
    /// every switcher on the Mac does.
    mutating func toggle(finding: Bool) {
        if isOpen, (mode == .find) == finding { close() } else { open(finding: finding) }
    }

    /// Esc, or the field losing the keyboard: back to the activity line.
    /// A message is kept for next time; a search isn't.
    mutating func close() {
        guard !sending else { return }
        if text.hasPrefix("/") { text = draft; draft = "" }
        isOpen = false
        notice = nil
        highlight = 0
    }

    /// What was typed. Typing never sends.
    mutating func edit(_ new: String) {
        text = new
        notice = nil
        highlight = 0
    }

    /// ↑ ↓, Tab and Shift-Tab through `count` results, wrapping.
    mutating func move(_ by: Int, count: Int) {
        guard mode == .find, count > 0 else { return }
        highlight = ((highlight + by) % count + count) % count
    }

    /// Return. `results` is how many find results are listed; `refusal` is
    /// why there's nobody to send to, or nil.
    mutating func submit(results: Int, refusal: String?) -> Intent {
        switch mode {
        case .rest:
            return .none
        case .find:
            return results > 0 ? .open(min(highlight, results - 1)) : .none
        case .ask:
            guard !sending else { return .none }
            guard !message.isEmpty else {
                notice = Self.empty
                return .refuse
            }
            if let refusal {
                notice = refusal
                return .refuse
            }
            sending = true
            return .send(message)
        }
    }

    /// The send came back: nil when the orchestrator took it, which clears
    /// and closes the field; else the field keeps the message and says so.
    mutating func sent(failed: Bool) {
        sending = false
        if failed {
            notice = Self.sendFailed
        } else {
            text = ""
            isOpen = false
            notice = nil
        }
    }

    /// A find result was opened: the field closes.
    mutating func opened() { close() }
}

/// The field's state, owned by the window so its commands reach it.
@MainActor @Observable final class TitleConsoleModel {
    var console = TitleConsole()
}

/// Who there is to send to: why not, or nil when the orchestrator can take
/// a message.
enum TitleConsoleRecipient {
    static let noOrchestrator = "There’s no orchestrator here to ask."
    static let cantTake = "The orchestrator can’t take a message from here. Type in its pane instead."
    static let notRunning = "The orchestrator isn’t running. Restart it from its menu first."

    static func refusal(seat: Terminal?) -> String? {
        guard let seat else { return noOrchestrator }
        let kind = StateKind.parse(seat.state)
        guard kind == .running || kind == .starting else { return notRunning }
        guard seat.isAgentPane || seat.canSwitchPaneMode else { return cantTake }
        return nil
    }
}

/// What the field does, routed by the window.
struct TitleConsoleActions {
    /// Find results for a query; the recent terminals for an empty one.
    var find: (String) -> [PaletteEntry] = { _ in [] }
    var run: (PaletteAction) -> Void = { _ in }
    /// Send to the orchestrator: nil when it took the message, else what
    /// went wrong (never shown raw).
    var send: (String) async -> String? = { _ in nil }
    /// Why nothing can be sent now, or nil.
    var refusal: () -> String? = { nil }

    func entries(_ console: TitleConsole) -> [PaletteEntry] {
        console.mode == .find ? find(console.query) : []
    }

    /// Return, carried out.
    @MainActor
    func submit(_ model: TitleConsoleModel) {
        let results = entries(model.console)
        switch model.console.submit(results: results.count, refusal: refusal()) {
        case .none, .refuse:
            return
        case .open(let index):
            let action = results[index].action
            model.console.opened()
            run(action)
        case .send(let message):
            Task { @MainActor in
                let failure = await send(message)
                model.console.sent(failed: failure != nil)
            }
        }
    }
}

/// The field itself, in the status area's place while it's open.
struct TitleConsoleField: View {
    let model: TitleConsoleModel
    let actions: TitleConsoleActions

    var body: some View {
        let console = model.console
        HStack(spacing: 6) {
            Image(systemName: console.mode == .find ? "magnifyingglass" : "arrow.up.message")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .frame(width: 14)
            PaletteField(
                text: Binding(get: { model.console.text }, set: { model.console.edit($0) }),
                placeholder: console.mode == .find ? TitleConsole.findPlaceholder : TitleConsole.askPlaceholder,
                horizontalMoves: false, fontSize: NSFont.systemFontSize,
                onMove: { move in
                    let count = actions.entries(model.console).count
                    switch move {
                    case .up, .previous, .left: model.console.move(-1, count: count)
                    case .down, .next, .right: model.console.move(1, count: count)
                    }
                },
                onSubmit: { actions.submit(model) },
                onCancel: { model.console.close() },
                onEndEditing: { model.console.close() })
            .frame(height: 18)
            .accessibilityLabel(console.mode == .find ? "Find" : "Ask the orchestrator")
            .accessibilityIdentifier("title-console-field")
            if console.sending {
                Text("Sending…").font(.caption).foregroundStyle(.secondary).fixedSize()
            }
        }
        .padding(.horizontal, Spacing.tight + 2)
        .frame(height: 22)
        .surface(.inset, in: .control)
    }
}

/// Under the field: the activity while there's nothing typed; the message's
/// one action while there is; the results while finding.
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
            case .ask:
                if console.text.isEmpty {
                    TitleActivityPopover(activity: activity, actions: status) { model.console.close() }
                } else {
                    sendRow(console)
                }
            case .find:
                results(console)
            }
            if let notice = console.notice {
                Label(notice, systemImage: "info.circle")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, Spacing.inset)
                    .padding(.bottom, Spacing.group)
                    .accessibilityIdentifier("title-console-notice")
            }
            footer(console)
        }
        .frame(width: 420, alignment: .leading)
        .surface(.floating, in: .floating)
    }

    private func sendRow(_ console: TitleConsole) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Image(systemName: "arrow.up.message").foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text("Send to Orchestrator").font(.callout.weight(.semibold))
                Text(console.message).font(.callout).foregroundStyle(.secondary).lineLimit(3)
            }
            Spacer(minLength: 0)
            Text("↩").foregroundStyle(.tertiary)
        }
        .padding(Spacing.inset)
        .contentShape(Rectangle())
        .onTapGesture { actions.submit(model) }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("title-console-send")
    }

    @ViewBuilder
    private func results(_ console: TitleConsole) -> some View {
        let entries = actions.entries(console)
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
                            PaletteRow(entry: entry, isHighlighted: index == console.highlight)
                                .id(entry.id)
                                .onTapGesture {
                                    model.console.opened()
                                    actions.run(entry.action)
                                }
                                .probed("title-console-result")
                        }
                    }
                    .padding(Spacing.group)
                }
                .frame(maxHeight: 320)
                .onChange(of: console.highlight) { _, index in
                    guard entries.indices.contains(index) else { return }
                    proxy.scrollTo(entries[index].id)
                }
            }
        }
    }

    private func footer(_ console: TitleConsole) -> some View {
        HStack(spacing: Spacing.group) {
            switch console.mode {
            case .find:
                KeyHint(keys: "↑↓", label: "Move")
                KeyHint(keys: "↩", label: "Open")
            default:
                KeyHint(keys: "↩", label: "Send")
                KeyHint(keys: "/", label: "Find")
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
                        .foregroundStyle(.secondary)
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
        .accessibilityAddTraits(isHighlighted ? [.isButton, .isSelected] : .isButton)
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
        if let store = source.board {
            Observed(store: store, model: model, actions: actions, status: status, source: source, showsField: showsField)
        } else {
            TitleConsolePanel(
                model: model, actions: actions, activity: TitleStatus.activity(source, board: .empty, starts: [:]),
                status: status, showsField: showsField)
        }
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
