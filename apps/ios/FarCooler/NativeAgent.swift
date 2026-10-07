import Foundation
import SwiftUI

/// Where the conversation view's message goes (ov-373): `terminal.compose`,
/// which types it into claude's box on one line and presses Enter past the
/// same gate as `terminal tell`, or refuses with a word and types nothing.
/// True when claude was working and its own queue took it (R-29).
protocol ConversationSink: Sendable {
    func compose(terminal: String, text: String) async throws -> Bool
}

/// The runner's `terminal.compose`, over this phone's client core.
struct CoreComposeSink: ConversationSink {
    let core: ClientCore

    func compose(terminal: String, text: String) async throws -> Bool {
        let data = try await core.call("terminal.compose", ["terminal": terminal, "text": text])
        let object = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        return object["queued"] as? Bool ?? false
    }
}

/// A held ask's answer (ov-370): the runner's `terminal.agent_answer`, over
/// this phone's client core, which writes it to the hook claude waits on.
struct CoreAnswerSink: AgentAnswerSink {
    let core: ClientCore

    func answer(terminal: String, ask: String, option: String, answers: [String: String]) async throws {
        var args: [String: Any] = ["terminal": terminal, "requestId": ask, "optionId": option]
        if !answers.isEmpty { args["answers"] = answers }
        _ = try await core.call("terminal.agent_answer", args)
    }
}

/// One pane's rows over this phone's client core: `agent.rows` and
/// `agent.rows_follow`, their answers handed to `AgentRowLedger` undecoded.
/// The core is the connection's own, so the rows ride the ssh session the
/// rest of the app does.
struct CoreRowSource: AgentRowSource {
    let core: ClientCore
    let terminal: String

    func page(before: UInt64?, limit: Int) async throws -> Data {
        var args: [String: Any] = ["terminal": terminal, "limit": limit]
        if let before { args["before"] = before }
        return try await mapped { try await core.call("agent.rows", args) }
    }

    func follow(epoch: UInt64, afterRev: UInt64, waitMs: Int) async throws -> Data {
        try await mapped {
            try await core.call(
                "agent.rows_follow", ["terminal": terminal, "epoch": epoch, "afterRev": afterRev, "waitMs": waitMs])
        }
    }

    /// A runner that doesn't serve rows (its projector turned off since the
    /// hello), or a pane that's gone, is not worth retrying.
    private func mapped(_ call: () async throws -> Data) async throws -> Data {
        do {
            return try await call()
        } catch ClientCore.CoreError.rejected(_, let word?, _)
            where word == "capability-unsupported" || word == "not-found"
        {
            throw AgentRowsUnavailable()
        }
    }
}

/// Every conversation-view pane this launch has opened, by terminal id, so a
/// pane's draft and rows outlive the views that show them: a tab swiped
/// away, a layout change, the pane switched to its terminal and back.
@MainActor
final class NativePanes: ObservableObject {
    static let shared = NativePanes()
    private var panes: [String: NativePaneModel] = [:]
    /// The panes whose conversation covers their terminal now, for the
    /// pane's bar: its terminal-only controls (Send an Image types a path
    /// into the covered box) go while it does.
    @Published private(set) var covered: Set<String> = []

    func cover(_ terminal: String, _ on: Bool) {
        if on, !covered.contains(terminal) {
            covered.insert(terminal)
        } else if !on, covered.contains(terminal) {
            covered.remove(terminal)
        }
    }

    /// The pane's model, made once and kept.
    func model(for terminal: String, core: ClientCore) -> NativePaneModel {
        if let model = panes[terminal], model.core === core { return model }
        let model = NativePaneModel(
            terminal: terminal, store: AgentRowStore(key: "phone-\(terminal)"),
            source: CoreRowSource(core: core, terminal: terminal), sink: CoreComposeSink(core: core),
            answers: CoreAnswerSink(core: core))
        model.core = core
        panes[terminal] = model
        return model
    }

    #if DEBUG
    /// The pane's model if this launch made one. `NativeAgentHarness` only.
    func existing(_ terminal: String) -> NativePaneModel? { panes[terminal] }
    #endif

    /// A model a harness made, under the same rules.
    func adopt(_ model: NativePaneModel) {
        panes[model.terminal] = model
    }
}

/// One terminal-mode claude pane's conversation side (ov-373): its rows, the
/// composer's draft, and which of the two views shows.
///
/// Nothing here touches the pane's process: the terminal under the
/// conversation is the same tmux pane, never respawned.
@MainActor
final class NativePaneModel: ObservableObject {
    let terminal: String
    let store: AgentRowStore
    let source: any AgentRowSource
    let sink: any ConversationSink
    /// Where a held ask's answer goes (ov-370).
    let answers: (any AgentAnswerSink)?
    /// The held ask whose answer is on its way, by its id.
    @Published private(set) var answering: String?
    /// Why an ask's answer didn't land, by the ask's id.
    @Published private(set) var answerIssues: [String: String] = [:]
    /// The connection's core this model reads through, so a pane opened again
    /// on another connection gets a model of its own.
    weak var core: ClientCore?

    /// The composer's text, one line: line breaks become spaces as they
    /// arrive, so what you see is what's sent. Return typed at the end sends,
    /// as the keyboard's Send key says.
    @Published var draft = "" {
        didSet {
            if draft.hasSuffix("\n"), draft.dropLast() == oldValue {
                draft = oldValue
                Task { await send() }
                return
            }
            let flat = AgentConversation.flattened(draft)
            if flat != draft { draft = flat }
        }
    }
    @Published private(set) var sending = false
    /// What stopped the last send, until the next one or a dismissal.
    @Published var issue: AgentConversation.SendIssue?
    /// Messages claude's queue took that its transcript hasn't shown yet,
    /// drawn as Queued rows below the list.
    @Published private(set) var queued: [String] = []
    /// How many messages this pane has sent, for the view to bring each one
    /// into view (ov-383).
    @Published private(set) var sent = 0
    /// Whether the person wants the conversation here, remembered per pane
    /// (R-27).
    @Published var wantsConversation: Bool {
        didSet {
            AgentConversation.remember(conversation: wantsConversation, for: terminal, defaults: defaults)
            updateShowing()
            followIfDue()
        }
    }
    /// The runner stopped serving this pane's rows before any arrived: it
    /// shows its terminal, with no switch, rather than an empty view.
    @Published private(set) var unavailable = false
    /// What the pane shows: the conversation, when wanted and available.
    @Published private(set) var showing = false

    /// The follow loop is running.
    @Published private(set) var following = false
    /// How many times the follow has been stopped, for the harness's probe.
    private(set) var stops = 0

    private let defaults: UserDefaults
    /// Whether this pane is the one on screen, and the app in front.
    private var onScreen = false

    init(
        terminal: String, store: AgentRowStore, source: any AgentRowSource, sink: any ConversationSink,
        answers: (any AgentAnswerSink)? = nil, defaults: UserDefaults = .standard
    ) {
        self.terminal = terminal
        self.store = store
        self.source = source
        self.sink = sink
        self.answers = answers
        self.defaults = defaults
        wantsConversation = AgentConversation.showsConversation(terminal, defaults: defaults)
        updateShowing()
    }

    private func updateShowing() {
        let now = wantsConversation && !unavailable
        if showing != now { showing = now }
    }

    /// Whether the pane is on screen with the app in front. The follow runs
    /// only then, and only while the conversation shows: a phone reaches its
    /// runner over ssh, and a held follow per pane in the background is a
    /// held call per pane nobody is reading.
    func setOnScreen(_ now: Bool) {
        onScreen = now
        followIfDue()
    }

    private func followIfDue() {
        let due = onScreen && showing
        // A loop that ended because the runner stopped serving rows isn't
        // following, whatever was set when it started.
        if due, following, store.phase == .unavailable { following = false }
        if due, !following {
            following = true
            store.start(source)
        } else if !due, following {
            following = false
            stops += 1
            store.stop()
        }
    }

    /// The conversation stopped being offered: the runner's setting turned
    /// off, its build lost, claude exited. Stop following, and forget that
    /// the runner said it had no rows, so the pane starts afresh when it's
    /// offered again. A follow left running here, or ended `.unavailable`
    /// with `following` still set, held the pane on "isn't being read" until
    /// a relaunch (ov-373 review 1).
    func release() {
        setOnScreen(false)
        if unavailable {
            unavailable = false
            updateShowing()
        }
    }

    /// The runner said it doesn't serve this pane's rows. With none held, the
    /// pane falls back to its terminal; with some, the view says they're
    /// stale over them.
    func phaseChanged() {
        if store.phase == .unavailable, store.ids.isEmpty, !unavailable {
            unavailable = true
            updateShowing()
            followIfDue()
        }
    }

    var canSend: Bool {
        let text = draft.trimmingCharacters(in: .whitespaces)
        return !sending && !text.isEmpty && text.count <= AgentConversation.longest && !store.isStale
    }

    /// Send the draft.
    func send() async {
        let text = draft.trimmingCharacters(in: .whitespaces)
        guard canSend else {
            if text.count > AgentConversation.longest { issue = .said(AgentConversation.tooLong) }
            return
        }
        if AgentConversation.isCommand(text) {
            issue = .said(AgentConversation.command)
            return
        }
        sending = true
        issue = nil
        defer { sending = false }
        do {
            let wasQueued = try await sink.compose(terminal: terminal, text: text)
            if draft.trimmingCharacters(in: .whitespaces) == text { draft = "" }
            if wasQueued { queued.append(text) }
            sent += 1
        } catch {
            issue = AgentConversation.issue(for: Self.failure(error))
        }
    }

    /// The page above the oldest row held.
    func loadOlder() {
        store.loadOlder(source)
    }

    /// Drop a local Queued echo once the transcript shows the message.
    func settleQueued() {
        guard !queued.isEmpty else { return }
        let newest = store.ids.suffix(40).compactMap { store.box($0)?.row }
        let left = AgentConversation.unsettled(queued, newest: newest)
        if left != queued { queued = left }
    }

    /// Answer the held ask on `ask`'s row (ov-370, R-33): `option`, and a
    /// question's `answers`. The runner writes it to claude's hook; nothing
    /// is typed into its dialog. One at a time; refused, the row says why.
    func answer(_ ask: AgentRow.Ask, option: String, answers given: [String: String] = [:]) async {
        guard let sink = answers, let id = ask.held, answering == nil, AgentConversation.answerable(ask) else { return }
        answering = id
        answerIssues[id] = nil
        defer { answering = nil }
        do {
            try await sink.answer(terminal: terminal, ask: id, option: option, answers: given)
        } catch {
            switch error as? ClientCore.CoreError {
            case .timedOut?, .malformed?, .disconnected(_, notSent: false)?:
                answerIssues[id] = AgentConversation.answerIssue(what: nil, timedOut: true)
            case .rejected(_, _, let what)?: answerIssues[id] = AgentConversation.answerIssue(what: what)
            default: answerIssues[id] = AgentConversation.answerIssue(what: nil)
            }
        }
    }

    static func failure(_ error: Error) -> AgentConversation.SendFailure {
        switch error as? ClientCore.CoreError {
        case .rejected(_, let word, let what)?: .refused(what: what, word: word)
        // An answer that couldn't be read is still an answer: the runner may
        // have typed it, so it says what a timeout says.
        case .timedOut?, .malformed?: .timedOut
        case .disconnected(_, let notSent)?: .lost(notSent: notSent)
        case .notStarted?: .lost(notSent: true)
        // A connect's answer, never a call's.
        case .unreached?, nil: .refused(what: nil)
        }
    }
}
