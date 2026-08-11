import Foundation

public struct ToolRow: Sendable, Equatable, Identifiable {
    public let id: String
    public var title: String
    public var kind: String
    public var status: ToolStatus
    public var locations: [String]
    public var content: String?
    public var diff: Diff?
}

/// One row, with an identity of its own.
///
/// The id is assigned when the row is created, NOT derived from its contents.
/// Deriving it meant asking the same question twice produced two rows with the
/// same id, and `ForEach` over duplicate ids does not merely look odd — it
/// renders blank bands and repeats rows in the wrong places. Content is not
/// identity: two identical messages are two messages.
/// A subagent's dispatch row, and everything it did.
///
/// The children are `TranscriptRow`s rather than a second row type, so a
/// subagent's messages, tools, and gaps render through exactly the same views
/// as the top level. Nesting is where they live, not what they are.
public struct SubagentBlock: Sendable, Equatable, Identifiable {
    public var id: String { tool.id }
    /// The `Task` call itself: title, status, location.
    public var tool: ToolRow
    public var children: [TranscriptRow]
    /// What it reported on finishing. Absent while it runs.
    public var summary: SubagentSummary?
    /// The turn ended before this reported back, so its outcome is unknown.
    /// Distinct from failure, and emphatically distinct from success.
    public var interrupted: Bool

    /// Still working, as far as anyone knows.
    ///
    /// Derived here rather than in each view because the interrupted case is
    /// easy to miss and expensive to miss: interruption does NOT change the
    /// tool's status — a cut-off subagent stays `inProgress` forever — so a
    /// surface that asks the status alone keeps the block auto-expanded and
    /// spinning for the rest of the session. The two apps derived this
    /// separately once and disagreed within a day.
    public var isRunning: Bool {
        !interrupted && (tool.status == .pending || tool.status == .inProgress)
    }

    /// What a collapsed block says instead of its contents.
    ///
    /// Here rather than in each view for the same reason as `isRunning`: the
    /// two apps wrote this twice and both said "1 tools".
    public var subtitle: String {
        // Ahead of the summary, deliberately. An interrupted block can carry a
        // partial one, and reporting a token count for a subagent whose
        // outcome nobody knows states the one thing this must never say.
        if interrupted { return "interrupted" }
        guard let summary else {
            return isRunning ? count(children.count, "step") : ""
        }
        let tokens = summary.tokens >= 1000
            ? "\(summary.tokens / 1000)k tok"
            : "\(summary.tokens) tok"
        let seconds = String(format: "%.1fs", Double(summary.durationMs) / 1000)
        return "\(summary.agentType) · \(count(Int(summary.toolUses), "tool")) · \(tokens) · \(seconds)"
    }

    private func count(_ n: Int, _ noun: String) -> String {
        "\(n) \(noun)\(n == 1 ? "" : "s")"
    }
}

public struct TranscriptRow: Sendable, Equatable, Identifiable {
    public let id: Int
    public var kind: Kind

    public enum Kind: Sendable, Equatable {
        /// `parent` is carried even though nesting already places the row,
        /// because coalescing needs it: an orphan whose block never arrived
        /// sits at the top level beside the agent's own words, and merging
        /// the two would re-create the very mis-attribution this fixes.
        case message(role: Role, text: String, parent: String?)
        case tool(ToolRow)
        case subagent(SubagentBlock)
        case gap(GapReason)
    }
}

public struct PendingPermission: Sendable, Equatable {
    public let id: String
    public let toolCall: String
    public let options: [PermissionOption]
}

/// Events in, something renderable out.
///
/// Shared rather than written twice, because a phone and a Mac that reduced
/// the same events differently would disagree about one session — the exact
/// failure the daemon-side derivation model exists to prevent.
public struct Transcript: Sendable {
    public private(set) var rows: [TranscriptRow] = []
    public private(set) var plan: [PlanEntry] = []
    public private(set) var pendingPermission: PendingPermission?
    public private(set) var agentMode: String?
    public private(set) var availableModes: [AgentChoice] = []
    public private(set) var model: String?
    public private(set) var availableModels: [AgentChoice] = []
    /// Every selector the agent offers. Render one control each.
    public private(set) var configOptions: [ConfigOption] = []
    /// What the agent calls this conversation, once it has named it.
    public private(set) var title: String?
    /// Which protocol is carrying this conversation: `acp`, `claude`, or
    /// `codex`.
    ///
    /// Defaults to `acp` rather than being optional, because that is what a
    /// session which never said is: ACP was the only backend that existed
    /// when those transcripts were written.
    public private(set) var backend: String = "acp"
    /// Context-window usage, or nil before the agent has reported any.
    public private(set) var contextUsed: UInt64?
    public private(set) var contextSize: UInt64?

    /// How full the context window is, 0...1, or nil if unknown.
    public var contextFraction: Double? {
        guard let used = contextUsed, let size = contextSize, size > 0 else { return nil }
        return min(1, Double(used) / Double(size))
    }
    /// The agent's slash commands, each with what it does.
    ///
    /// `AgentChoice`, not names: the adapter has always sent a description and
    /// the picker used to show only the name — a list of names is a list you
    /// have to already know.
    public private(set) var availableCommands: [AgentChoice] = []
    /// Written but not sent, in the order it will be sent.
    public private(set) var queue: [QueuedPrompt] = []
    /// The seq to ask for on reconnect: one past the highest seen.
    public private(set) var cursor: UInt64 = 0

    /// Whether the next message must start a new row rather than joining the
    /// last one.
    ///
    /// Set at a turn boundary. Without it two consecutive turns' replies
    /// coalesce — they are both `.agent` and adjacent — and the transcript
    /// renders "…prints hi to the console.Hello! I'm Claude Code…" as one
    /// paragraph, with no seam where a whole turn began.
    private var breakBeforeNextMessage = false

    /// The id the next row will take. Monotonic, never reused.
    private var nextRowID = 0

    /// One message drawn straight from the composer, and the row it drew.
    ///
    /// The row id is carried rather than the text alone so a withdrawal can
    /// name its row exactly. See `unconfirmedEchoes`.
    private struct LocalEcho {
        let rowID: Int
        let text: String
    }

    /// Messages drawn straight from the composer that the daemon has not yet
    /// accounted for.
    ///
    /// The composer used to PREDICT whether a message would be queued, by
    /// reading fleet state while the thing it predicted read the agent
    /// channel. In the window between a turn ending on one and the other
    /// noticing, it guessed wrong: the echo was suppressed for a queue row
    /// that never came, the CLI's own echo was dropped as a duplicate, and the
    /// message reached the model without ever being drawn.
    ///
    /// So the client draws first and asks after. Entries are cleared at the
    /// NEXT `promptQueue`, which is the event that answers the question — a
    /// queue that carries the text means it was held, and one that does not
    /// means it went out. On the ordinary path that event never arrives at
    /// all: `ChatSession::prompt` returns no events for a message that goes
    /// straight to the backend, so an entry can sit here for the rest of the
    /// epoch. That is not a leak to fix by dropping entries on a timer — it is
    /// why the match in `promptQueue` CONSUMES. Each queue item settles at
    /// most one echo, so an entry nobody ever answers can never withdraw a row
    /// that a later, different message drew.
    ///
    /// Carrying the row id also removes a trap. Re-finding the row by text
    /// would be correct only while no daemon-emitted `Message { role: User }`
    /// can land after a live echo carrying the same words — which holds solely
    /// because `send_next_queued` and `steer_queued` both emit their queue
    /// event BEFORE the message. Swap those two lines in the daemon and a text
    /// search would withdraw the genuine, already-delivered row instead of the
    /// echo, and nothing on screen would look wrong. An id cannot make that
    /// mistake, so the ordering stops being load-bearing.
    private var unconfirmedEchoes: [LocalEcho] = []

    public init() {}

    /// Throw away everything and start again.
    ///
    /// Used when the daemon reports a different epoch: the cursor this held
    /// counts positions in a stream that no longer exists, so the rows built
    /// from it describe a conversation that is not the one being served. The
    /// selectors survive, because they describe the AGENT rather than the
    /// stream and re-arrive with the next `SessionStarted` anyway — clearing
    /// them would blank the pickers on every toggle.
    public mutating func resetForNewEpoch() {
        rows = []
        plan = []
        queue = []
        pendingPermission = nil
        cursor = 0
        nextRowID = 0
        breakBeforeNextMessage = false
        // An echo names an uncertain send from THIS epoch. Left alive, it can
        // collide with a message replayed into the new epoch — a real row —
        // and the next promptQueue naming that text would delete restored
        // history that was never in question. Row ids restart here too, so a
        // surviving echo would not merely match by text: its recorded id would
        // name a real row of the new epoch.
        unconfirmedEchoes = []
    }

    public mutating func apply(_ events: [Sequenced]) {
        for item in events {
            // Already folded in, so skipped rather than applied twice.
            //
            // Within one epoch the daemon numbers by position, so a seq below
            // the cursor names an event this transcript already holds. It can
            // still arrive: anything that re-delivers a batch — a reconnect, a
            // replay racing a push — hands back numbers already seen, and
            // applying them again renders the conversation twice.
            guard item.seq >= cursor else { continue }
            cursor = item.seq + 1
            apply(item.event)
        }
    }

    /// Take the approval card down.
    ///
    /// The answer goes to the agent, which resumes without saying anything
    /// about the request it was blocked on — no `Resolved` comes back — so a
    /// card that waited for one stayed on screen after the work it was gating
    /// had already happened.
    public mutating func clearPendingPermission() {
        pendingPermission = nil
    }

    /// Which container a row belongs in.
    ///
    /// A parent nobody has seen resolves to `.top` rather than being dropped.
    /// The ring can trim a dispatch out from under its children, and a reload
    /// can replay only part of a turn. Nothing is missing in that case except
    /// the nesting, and a shorter transcript that looks complete is the one
    /// failure this design refuses.
    private enum Destination {
        case top
        case block(Int)
    }

    private func destination(for parent: String?) -> Destination {
        guard let parent else { return .top }
        guard
            let index = rows.lastIndex(where: {
                if case let .subagent(block) = $0.kind { return block.tool.id == parent }
                return false
            })
        else { return .top }
        return .block(index)
    }

    private mutating func append(_ kind: TranscriptRow.Kind, to destination: Destination = .top) {
        let row = TranscriptRow(id: nextRowID, kind: kind)
        nextRowID += 1
        switch destination {
        case .top:
            rows.append(row)
        case let .block(index):
            guard case var .subagent(block) = rows[index].kind else { return }
            block.children.append(row)
            rows[index].kind = .subagent(block)
        }
    }

    /// The last row of whichever container this names.
    private func lastRow(in destination: Destination) -> TranscriptRow? {
        switch destination {
        case .top:
            return rows.last
        case let .block(index):
            guard case let .subagent(block) = rows[index].kind else { return nil }
            return block.children.last
        }
    }

    private mutating func replaceLastRow(
        in destination: Destination, with kind: TranscriptRow.Kind
    ) {
        switch destination {
        case .top:
            guard !rows.isEmpty else { return }
            rows[rows.count - 1].kind = kind
        case let .block(index):
            guard case var .subagent(block) = rows[index].kind, !block.children.isEmpty else {
                return
            }
            block.children[block.children.count - 1].kind = kind
            rows[index].kind = .subagent(block)
        }
    }

    /// Update a call in place, wherever it lives.
    ///
    /// Three lookups, in order: the dispatch rows themselves, the top level,
    /// then inside a block. The flat search this replaced found nothing for a
    /// nested tool and fell through to appending, so one tool reporting
    /// progress rendered as two rows — the real one inside the block and a
    /// half-built duplicate beside it.
    private mutating func applyToolUpdate(
        id: String, status: ToolStatus, title: String?, content: String?, diff: Diff?,
        locations: [String], parent: String?, summary: SubagentSummary?
    ) {
        if let index = rows.lastIndex(where: {
            if case let .subagent(block) = $0.kind { return block.tool.id == id }
            return false
        }) {
            guard case var .subagent(block) = rows[index].kind else { return }
            block.tool = merged(
                block.tool, status: status, title: title, content: content, diff: diff,
                locations: locations)
            if let summary { block.summary = summary }
            rows[index].kind = .subagent(block)
            return
        }

        if let index = rows.lastIndex(where: {
            if case let .tool(tool) = $0.kind { return tool.id == id }
            return false
        }) {
            guard case let .tool(tool) = rows[index].kind else { return }
            rows[index].kind = .tool(merged(
                tool, status: status, title: title, content: content, diff: diff,
                locations: locations))
            return
        }

        for index in rows.indices.reversed() {
            guard case var .subagent(block) = rows[index].kind else { continue }
            guard
                let child = block.children.lastIndex(where: {
                    if case let .tool(tool) = $0.kind { return tool.id == id }
                    return false
                })
            else { continue }
            guard case let .tool(tool) = block.children[child].kind else { continue }
            block.children[child].kind = .tool(merged(
                tool, status: status, title: title, content: content, diff: diff,
                locations: locations))
            rows[index].kind = .subagent(block)
            return
        }

        // Nothing to update: an update whose call we never saw. Shown rather
        // than dropped, in whichever container it claims to belong to.
        append(
            .tool(ToolRow(
                id: id, title: title ?? id, kind: "", status: status, locations: locations,
                content: content, diff: diff)),
            to: destination(for: parent))
    }

    /// The merge rules for a tool update, in one place so the lookup paths
    /// above cannot drift apart from each other.
    private func merged(
        _ tool: ToolRow, status: ToolStatus, title: String?, content: String?, diff: Diff?,
        locations: [String]
    ) -> ToolRow {
        var tool = tool
        tool.status = status
        // A call is renamed as it resolves — "Terminal" becomes the command,
        // "Read File" becomes the file — and the new name is the useful one.
        if let title, !title.isEmpty { tool.title = title }
        if let content { tool.content = content }
        if let diff { tool.diff = diff }
        if !locations.isEmpty { tool.locations = locations }
        return tool
    }

    /// Show a selector's new value straight away.
    ///
    /// The adapter does not reliably send `config_option_update` after
    /// `session/set_config_option` — it applies the change and says nothing —
    /// so a picker that waited for confirmation snapped back to its old value
    /// and read as broken. If a real update does arrive it overwrites this
    /// with the same thing.
    public mutating func selectConfigOptionLocally(id: String, value: String) {
        setConfigValue(id: id, value: value)
    }

    /// Kept in the list rather than in a parallel dictionary, so the control a
    /// user is looking at and the value it shows can never be two different
    /// things.
    private mutating func setConfigValue(id: String, value: String) {
        configOptions = configOptions.map { option in
            guard option.id == id else { return option }
            return ConfigOption(
                id: option.id, name: option.name, description: option.description,
                category: option.category, kind: option.kind, currentValue: value,
                options: option.options)
        }
        if id == "model" { model = value }
        if id == "mode" { agentMode = value }
    }

    /// Show what the user just sent, before the agent says anything.
    ///
    /// The adapter echoes a `user_message_chunk` only when replaying a loaded
    /// session, never during a live turn — so without this a message vanishes
    /// the moment it is sent and does not reappear until the pane is reopened.
    /// A chat that swallows your own words reads as broken even when the turn
    /// underneath it is running perfectly.
    ///
    /// Called on EVERY send, including one written mid-turn. The client used
    /// to decide up front which sends would be queued and skip drawing those;
    /// it read that from stale state and a message that actually went straight
    /// out was never drawn at all. So a mid-turn message is drawn here like
    /// any other and withdrawn again by the `promptQueue` that reports it held.
    public mutating func appendLocalUserMessage(_ text: String) {
        // Recorded before the append, because that is the id the row is about
        // to be given — the withdrawal below has to name this exact row.
        let rowID = nextRowID
        // Always the top level: what the user typed is addressed to the
        // session, never to one subagent inside it.
        append(.message(role: .user, text: text, parent: nil))
        // The agent's reply is a new turn's worth of speech, not a
        // continuation of what the user just typed.
        breakBeforeNextMessage = true
        unconfirmedEchoes.append(LocalEcho(rowID: rowID, text: text))
    }

    /// Withdraw one row by the id it was created with.
    ///
    /// By id and not by contents: ids are handed out once per epoch and never
    /// reused, so this withdraws the row that was drawn and nothing that
    /// merely reads like it.
    private mutating func removeRow(id: Int) {
        guard let index = rows.firstIndex(where: { $0.id == id }) else { return }
        rows.remove(at: index)
    }

    private mutating func apply(_ event: AgentEvent) {
        switch event {
        case let .sessionStarted(
            _, mode, modes, currentModel, models, options, commands, wire):
            backend = wire
            agentMode = mode
            availableModes = modes
            model = currentModel
            if !models.isEmpty { availableModels = models }
            if !options.isEmpty { configOptions = options }
            // Only replace when the session actually offered some. A later
            // event carrying an empty list would otherwise empty the picker.
            if !commands.isEmpty { availableCommands = commands }

        case let .message(role, text, parent):
            let target = destination(for: parent)
            // Chunks of one message coalesce. One row per chunk would render a
            // streamed sentence as a column of one-word paragraphs.
            //
            // Only within one container, and only across one parent: merging
            // past either boundary splices a subagent's sentence onto the
            // dispatching agent's and attributes it to the wrong speaker.
            if case let .message(lastRole, lastText, lastParent) = lastRow(in: target)?.kind,
                lastRole == role, lastParent == parent,
                !(parent == nil && breakBeforeNextMessage)
            {
                replaceLastRow(
                    in: target, with: .message(role: role, text: lastText + text, parent: parent))
            } else {
                append(.message(role: role, text: text, parent: parent), to: target)
            }
            // The seam belongs to the top-level conversation; a subagent's
            // chunks must not consume it.
            if parent == nil { breakBeforeNextMessage = false }

        case let .toolCall(id, title, kind, status, locations, parent, subagent):
            let tool = ToolRow(
                id: id, title: title, kind: kind, status: status,
                locations: locations, content: nil, diff: nil)
            if subagent {
                // A dispatch OWNS a block rather than being a row inside one,
                // so it always lands at the level it was called from.
                append(.subagent(SubagentBlock(
                    tool: tool, children: [], summary: nil, interrupted: false)))
            } else {
                append(.tool(tool), to: destination(for: parent))
            }

        case let .toolUpdate(id, status, newTitle, content, diff, locations, parent, summary):
            applyToolUpdate(
                id: id, status: status, title: newTitle, content: content, diff: diff,
                locations: locations, parent: parent, summary: summary)

        case let .plan(entries):
            // Wholesale, because the daemon sends the whole plan each time.
            plan = entries

        case let .permission(id, toolCall, options):
            pendingPermission = PendingPermission(id: id, toolCall: toolCall, options: options)

        case let .resolved(id, _):
            if pendingPermission?.id == id { pendingPermission = nil }

        case let .commandsAvailable(commands):
            // Resent every turn, so this arrives repeatedly with the same
            // contents. Replacing is right; appending would grow the picker
            // without bound.
            availableCommands = commands

        case let .sessionInfo(newTitle):
            // Not a row. It is revised as the conversation goes on, and a line
            // of transcript per revision would bury the conversation it names.
            if !newTitle.isEmpty { title = newTitle }

        case let .usage(used, size):
            // Not a row. It is resent constantly as a turn burns context, and
            // one line of transcript per report would bury the conversation.
            contextUsed = used
            contextSize = size

        case let .configSet(id, value):
            setConfigValue(id: id, value: value)

        case let .modeSet(mode):
            agentMode = mode

        case let .promptQueue(items):
            // Wholesale, like the plan: the daemon sends the whole queue on
            // every change, and a client reconstructing it from adds and
            // removes could disagree with what will actually be sent.
            queue = items
            // Anything drawn from the composer that turns out to be HELD is
            // withdrawn from the conversation — it has not joined it yet, and
            // in the queue it can still be edited or taken back. Anything not
            // named here went out, so it stays. Either way every echo
            // outstanding when this arrived is now answered, so the list is
            // cleared. An echo that no `promptQueue` ever answers — the whole
            // ordinary path — simply waits here; see `unconfirmedEchoes`.
            //
            // Matched one at a time against a working copy, not with
            // `contains`: the same text sent twice before this event leaves
            // two identical echoes, and a non-consuming match would let one
            // held queue item cancel both rows — deleting the one that
            // genuinely went out, with no queue entry left to show for it.
            //
            // The LAST matching echo, because when two identical sends are
            // outstanding it is the earlier one that went out and the later
            // one that is still waiting. Withdrawing the earlier row would
            // leave the transcript showing the agent's answer above the
            // question it answered.
            var unmatchedEchoes = unconfirmedEchoes
            for item in items {
                guard
                    let index = unmatchedEchoes.lastIndex(where: { $0.text == item.text })
                else { continue }
                let echo = unmatchedEchoes.remove(at: index)
                // By id: the echo already knows which row it drew, so this
                // cannot mistake a real user message for the echo of one.
                removeRow(id: echo.rowID)
            }
            unconfirmedEchoes.removeAll()

        case .turnEnded:
            // Nothing to DRAW, but it is a seam: the next message begins a new
            // turn and must not be glued onto the tail of this one.
            breakBeforeNextMessage = true
            // A subagent still running when the turn ends never receives its
            // completion. Left alone it spins forever, and once the view stops
            // animating it reads as one that finished — a subagent whose fate
            // nobody knows wearing the mark of one that reported back.
            for index in rows.indices {
                guard case var .subagent(block) = rows[index].kind else { continue }
                guard block.tool.status == .pending || block.tool.status == .inProgress else {
                    continue
                }
                block.interrupted = true
                rows[index].kind = .subagent(block)
            }

        case let .gap(reason):
            // Never merged, never dropped. A gap that could be swallowed by a
            // neighbouring message would leave the user believing a transcript
            // is complete when it is not.
            append(.gap(reason))
        }
    }
}
