import AgentKit
import AppKit
import SwiftUI

// The title bar's field (ov-214, slice 4): the status area's activity line
// at rest; ⌘K, or a click on it, makes it a field.
//
//   ask    the default. Return sends what's typed to this workspace's
//          orchestrator, named in the field ("Ask Billing’s orchestrator"),
//          and only ever to it: a chat orchestrator by its composer's route
//          (`terminal agent-prompt`), one in a terminal through the
//          runner's answer gate (`terminal tell`: a proven agent, idle, an
//          empty box, bracketed paste known), which types nothing when any
//          check fails. Empty, the panel under it shows the activity.
//   find   `/` at the start, or ⌘P, which opens it with `/` already typed:
//          Go to Anything, the palette's results, in place, then the files
//          of the worktree on screen whose paths match (ov-189).
//   Esc    back to the activity line, even while a send is out. A message
//          being written is kept, for its own workspace: each workspace has
//          its own, so a message is never sent to another's orchestrator.
//
// Never sent by accident: only Return sends (never a click, and never
// mid-composition in an input method), an empty or whitespace-only message
// is refused with a sentence, a second Return while one is out does
// nothing, and a send the runner doesn't answer in `timeout` stops waiting
// and says so.
//
// The rules are this value's, so `TitleConsoleTests` pins them; the views
// draw it.

/// The field's state.
struct TitleConsole: Equatable {
    enum Mode: Equatable { case rest, ask, find }

    /// What a Return does.
    enum Intent: Equatable {
        case none
        /// Send this, trimmed, to the orchestrator, as send `id`.
        case send(String, id: Int)
        /// Refused, with the sentence now in `notice`.
        case refuse
        /// Open the find result at this index.
        case open(Int)
    }

    /// How a send came out.
    enum Outcome: Equatable {
        case sent
        /// Nothing was typed, and why: the message stays in the field.
        case refused(String)
        /// It was typed but not submitted, or landed somewhere it wasn't
        /// meant to: the field lets it go, and the sentence says where.
        case left(String)
    }

    /// A send that's out.
    struct Pending: Equatable {
        var id: Int
        var workspace: String
        var text: String
    }

    private(set) var isOpen = false
    /// What's in the field.
    private(set) var text = ""
    /// The result the highlight is on, once ↑ or ↓ has moved it.
    private(set) var highlight = 0
    /// Whether ↑ or ↓ has moved the highlight since the results changed:
    /// until then it's where the results say to start (`opening`).
    private(set) var moved = false
    /// What the last Return refused or failed with; typing clears it.
    private(set) var notice: String?
    /// A message is on its way, and the field is waiting for it.
    private(set) var sending = false
    /// The last send, until its answer comes, waited for or not.
    private(set) var pending: Pending?
    private var sends = 0
    /// A message set aside while finding, given back by the next ⌘K.
    private(set) var draft = ""
    /// The workspace on screen (`host|workspace`): whose message `text` is.
    private(set) var workspace = ""
    /// Every other workspace's message being written, by workspace.
    private(set) var drafts: [String: String] = [:]
    /// What the runner found in the files of the worktree on screen (ov-189),
    /// and the query it found them for: listed only while that's still the
    /// query, so a slow answer never lists files for what was typed before.
    private(set) var fileHits: [String] = []
    private(set) var fileHitsQuery: String?

    var mode: Mode {
        guard isOpen else { return .rest }
        return text.hasPrefix("/") ? .find : .ask
    }

    /// What find mode searches for: the text after its `/`.
    var query: String { String(text.dropFirst()).trimmingCharacters(in: .whitespaces) }

    /// What ask mode would send.
    var message: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// How long a send is waited for.
    static let timeout: Duration = .seconds(20)

    static let empty = "Type a message first."
    static let sendFailed = "Couldn’t send that to the orchestrator. Your message is still here."
    static let timedOut =
        "The runner didn’t answer in 20 seconds. Your message is still here; check the orchestrator before sending it again."
    static let findPlaceholder = "Find a workspace, task, terminal, or file"

    /// "Ask Billing’s orchestrator, or type / to find".
    static func askPlaceholder(recipient: String?) -> String {
        guard let recipient, !recipient.isEmpty else { return "Ask the orchestrator, or type / to find" }
        return "Ask \(recipient)’s orchestrator, or type / to find"
    }

    /// "To Billing’s orchestrator".
    static func sendTitle(recipient: String?) -> String {
        guard let recipient, !recipient.isEmpty else { return "To the orchestrator" }
        return "To \(recipient)’s orchestrator"
    }

    /// The window moved to workspace `key`: this one's message is kept for
    /// it, the field closes, and `key`'s own message, if any, comes back.
    mutating func enter(workspace key: String) {
        guard key != workspace else { return }
        let keep = text.hasPrefix("/") ? draft : text
        if !workspace.isEmpty { drafts[workspace] = keep.isEmpty ? nil : keep }
        workspace = key
        text = drafts.removeValue(forKey: key) ?? ""
        draft = ""
        isOpen = false
        sending = false
        notice = nil
        resetHighlight()
    }

    /// ⌘K, a click on the activity line, or Show Activity: open to ask,
    /// with any message set aside given back. ⌘P (`finding`): open to
    /// find, with `/` typed, setting a message being written aside.
    mutating func open(finding: Bool) {
        if !sending { notice = nil }
        resetHighlight()
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

    /// Esc, or the field losing the keyboard: back to the activity line. A
    /// message is kept for next time; a search isn't. A send that's out
    /// stops being waited for (`sent` still hears its answer).
    mutating func close() {
        if text.hasPrefix("/") { text = draft; draft = "" }
        isOpen = false
        sending = false
        notice = nil
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

    /// What was typed. Typing never sends.
    mutating func edit(_ new: String) {
        text = new
        if !sending { notice = nil }
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

    /// Return. `results` is how many find results are listed, `opening`
    /// where their highlight starts; `refusal` is why there's nobody to
    /// send to, or nil.
    mutating func submit(results: Int, opening: Int = 0, refusal: String?) -> Intent {
        switch mode {
        case .rest:
            return .none
        case .find:
            return results > 0 ? .open(min(highlight(opening: opening), results - 1)) : .none
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
            sends += 1
            sending = true
            notice = nil
            pending = Pending(id: sends, workspace: workspace, text: text)
            return .send(message, id: sends)
        }
    }

    /// Send `id` came back. Waited for, `.sent` clears and closes the
    /// field, `.refused` keeps the message and says why, `.left` lets it go
    /// and says where it is. Not waited for any more (Esc, a timeout,
    /// another workspace), only a message that went is taken out of the
    /// field or its workspace's drafts, so it can't be sent twice.
    mutating func sent(id: Int, _ outcome: Outcome) {
        guard let p = pending, p.id == id else { return }
        pending = nil
        let went: Bool = {
            switch outcome {
            case .sent, .left: return true
            case .refused: return false
            }
        }()
        guard sending else {
            if went {
                if p.workspace == workspace, text == p.text { text = "" }
                if drafts[p.workspace] == p.text { drafts[p.workspace] = nil }
            }
            return
        }
        sending = false
        switch outcome {
        case .sent:
            text = ""
            isOpen = false
            notice = nil
        case .refused(let why):
            notice = why
        case .left(let where_):
            text = ""
            notice = where_
        }
    }

    /// Send `id` wasn't answered in `timeout`: stop waiting, keep the
    /// message, and say so. Its answer, if one comes, is still heard.
    mutating func timedOut(id: Int) {
        guard sending, pending?.id == id else { return }
        sending = false
        notice = Self.timedOut
    }

    /// A find result was opened: the field closes.
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

/// Who there is to send to: why not, or nil when the orchestrator may take
/// a message (the runner still checks a terminal one, `TellRefusal`).
enum TitleConsoleRecipient {
    static let noOrchestrator = "There’s no orchestrator here to ask."
    static let cantTake = "The orchestrator can’t take a message from here. Type in its pane instead."
    static let notRunning = "The orchestrator isn’t running. Restart it from its menu first."

    static func refusal(seat: Terminal?) -> String? {
        guard let seat else { return noOrchestrator }
        let kind = StateKind.parse(seat.state)
        guard kind == .running || kind == .starting else { return notRunning }
        guard !seat.isChangesPane else { return cantTake }
        return nil
    }

    /// Which route a message to `seat` takes: a chat's composer, or the
    /// runner's typing gate for a terminal.
    enum Route: Equatable { case composer, terminal }

    static func route(_ seat: Terminal) -> Route { seat.isAgentPane ? .composer : .terminal }
}

/// What a terminal orchestrator's runner says when it types nothing, or
/// leaves the text somewhere: its stable word (`what:`) or code (`code:`),
/// in this app's sentences. Never the runner's own text.
enum TellRefusal {
    static func outcome(message: String?) -> TitleConsole.Outcome {
        let lines = (message ?? "").split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }
        let what = lines.first { $0.hasPrefix("what: ") }.map { String($0.dropFirst("what: ".count)) }
        let code = lines.first { $0.hasPrefix("code: ") }.map { String($0.dropFirst("code: ".count)) }
        switch what {
        case "busy"?: return .refused("The orchestrator is working. Send it again when it’s done.")
        case "prompt"?: return .refused("The orchestrator is asking something. Answer it in its pane first.")
        case "draft"?: return .refused("There’s text in the orchestrator’s box. Send or clear it in its pane first.")
        case "typing"?: return .refused("Someone is typing in the orchestrator’s pane. Try again in a moment.")
        case "not_an_agent"?: return .refused("No agent is running in the orchestrator’s pane.")
        case "unfamiliar"?:
            return .refused("Far Cooler doesn’t recognize what the orchestrator’s pane shows, so it typed nothing.")
        case "unproven"?:
            return .refused(
                "This runner’s tmux is older than 3.7, so Far Cooler can’t tell yet whether the orchestrator takes a paste.")
        case "too_long"?: return .refused("That’s too long to type into a terminal. Keep it under 500 characters.")
        case "not_running"?: return .refused(TitleConsoleRecipient.notRunning)
        case "text"?: return .refused(TitleConsole.empty)
        case "paste_left"?:
            return .left("Your message is in the orchestrator’s box but wasn’t sent. Press Return in its pane to send it.")
        case "left_at_shell"?:
            return .left("The orchestrator stopped before your message was sent. It was left at its shell prompt, not run.")
        default: break
        }
        if code == "capability-unsupported" {
            return .refused("This runner needs an update to take messages from the title bar.")
        }
        return .refused(TitleConsole.sendFailed)
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
    /// Send to the orchestrator, and how it came out.
    var send: (String) async -> TitleConsole.Outcome = { _ in .sent }
    /// Why nothing can be sent now, or nil.
    var refusal: () -> String? = { nil }
    /// The workspace whose orchestrator a message goes to, by name.
    var recipient: String? = nil
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

    /// Return, carried out.
    @MainActor
    func submit(_ model: TitleConsoleModel) {
        let results = entries(model.console)
        let start = opening(model.console, results)
        switch model.console.submit(results: results.count, opening: start, refusal: refusal()) {
        case .none, .refuse:
            return
        case .open(let index):
            let action = results[index].action
            model.console.opened()
            run(action)
        case .send(let message, let id):
            let send = send
            Task { @MainActor in
                let outcome = await send(message)
                model.console.sent(id: id, outcome)
            }
            Task { @MainActor in
                try? await Task.sleep(for: TitleConsole.timeout)
                model.console.timedOut(id: id)
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
                placeholder: console.mode == .find
                    ? TitleConsole.findPlaceholder : TitleConsole.askPlaceholder(recipient: actions.recipient),
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
            .accessibilityLabel(
                console.mode == .find ? "Find" : TitleConsole.sendTitle(recipient: actions.recipient))
            .accessibilityIdentifier("title-console-field")
            if console.sending {
                Text("Sending…").font(.caption).foregroundStyle(.secondary).fixedSize()
            }
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

    /// The send row was clicked: it lights, and still only Return sends.
    @State private var sendLit = false

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
                    TitleActivityPopover(activity: activity, actions: status, cache: model) { model.console.close() }
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
        // A click here mustn't close the field it belongs to
        // (`TitleConsoleModel.fieldEndedEditing`).
        .onHover { inside in model.pointerInPanel = inside }
        .onDisappear { model.pointerInPanel = false }
        .onChange(of: console.text) { _, _ in sendLit = false }
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

    private func sendRow(_ console: TitleConsole) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Image(systemName: "arrow.up.message").foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(TitleConsole.sendTitle(recipient: actions.recipient)).font(.callout.weight(.semibold))
                Text(console.message).font(.callout).foregroundStyle(.secondary).lineLimit(3)
            }
            Spacer(minLength: 0)
            Text(sendLit ? "Press ↩ to Send" : "↩").foregroundStyle(.tertiary).fixedSize()
        }
        .padding(Spacing.inset)
        .background(sendLit ? Fill.hover : Color.clear, in: .control)
        .contentShape(Rectangle())
        // Lights, never sends: only Return sends (ov-214 review).
        .onTapGesture { sendLit = true }
        .accessibilityElement(children: .combine)
        .accessibilityHint("Press Return to send")
        .accessibilityIdentifier("title-console-send")
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
