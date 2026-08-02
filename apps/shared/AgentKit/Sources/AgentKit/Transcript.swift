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
public struct TranscriptRow: Sendable, Equatable, Identifiable {
    public let id: Int
    public var kind: Kind

    public enum Kind: Sendable, Equatable {
        case message(role: Role, text: String)
        case tool(ToolRow)
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

    private mutating func append(_ kind: TranscriptRow.Kind) {
        rows.append(TranscriptRow(id: nextRowID, kind: kind))
        nextRowID += 1
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
    /// Only for a message going out NOW. One written mid-turn is queued
    /// instead, and joins the transcript when it is actually sent.
    public mutating func appendLocalUserMessage(_ text: String) {
        append(.message(role: .user, text: text))
        // The agent's reply is a new turn's worth of speech, not a
        // continuation of what the user just typed.
        breakBeforeNextMessage = true
    }

    private mutating func apply(_ event: AgentEvent) {
        switch event {
        case let .sessionStarted(_, mode, modes, currentModel, models, options, commands):
            agentMode = mode
            availableModes = modes
            model = currentModel
            if !models.isEmpty { availableModels = models }
            if !options.isEmpty { configOptions = options }
            // Only replace when the session actually offered some. A later
            // event carrying an empty list would otherwise empty the picker.
            if !commands.isEmpty { availableCommands = commands }

        case let .message(role, text):
            // Chunks of one message coalesce. One row per chunk would render a
            // streamed sentence as a column of one-word paragraphs.
            if case let .message(lastRole, lastText) = rows.last?.kind, lastRole == role,
                !breakBeforeNextMessage
            {
                rows[rows.count - 1].kind = .message(role: role, text: lastText + text)
            } else {
                append(.message(role: role, text: text))
            }
            breakBeforeNextMessage = false

        case let .toolCall(id, title, kind, status, locations):
            append(.tool(ToolRow(
                id: id, title: title, kind: kind, status: status,
                locations: locations, content: nil, diff: nil)))

        case let .toolUpdate(id, status, newTitle, content, diff, locations):
            // Mutate the call in place. Appending would fill the transcript
            // with duplicates of one tool reporting progress.
            guard let index = rows.lastIndex(where: {
                if case let .tool(t) = $0.kind { return t.id == id }
                return false
            }) else {
                append(.tool(ToolRow(
                    id: id, title: newTitle ?? id, kind: "", status: status,
                    locations: locations, content: content, diff: diff)))
                return
            }
            guard case var .tool(tool) = rows[index].kind else { return }
            tool.status = status
            // A call is renamed as it resolves — "Terminal" becomes the
            // command, "Read File" becomes the file — and the new name is the
            // informative one.
            if let newTitle, !newTitle.isEmpty { tool.title = newTitle }
            if let content { tool.content = content }
            if let diff { tool.diff = diff }
            if !locations.isEmpty { tool.locations = locations }
            rows[index].kind = .tool(tool)

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

        case .turnEnded:
            // Nothing to DRAW, but it is a seam: the next message begins a new
            // turn and must not be glued onto the tail of this one.
            breakBeforeNextMessage = true

        case let .gap(reason):
            // Never merged, never dropped. A gap that could be swallowed by a
            // neighbouring message would leave the user believing a transcript
            // is complete when it is not.
            append(.gap(reason))
        }
    }
}
