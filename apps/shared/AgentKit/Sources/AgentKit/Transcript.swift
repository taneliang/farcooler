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

public enum TranscriptRow: Sendable, Equatable, Identifiable {
    case message(role: Role, text: String)
    case tool(ToolRow)
    case gap(GapReason)

    public var id: String {
        switch self {
        case let .tool(t): "tool-\(t.id)"
        case let .message(role, text): "msg-\(role.rawValue)-\(text.hashValue)"
        case let .gap(reason): "gap-\(reason.rawValue)"
        }
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
    public private(set) var availableModes: [String] = []
    public private(set) var availableCommands: [String] = []
    /// The seq to ask for on reconnect: one past the highest seen.
    public private(set) var cursor: UInt64 = 0

    public init() {}

    public mutating func apply(_ events: [Sequenced]) {
        for item in events {
            cursor = max(cursor, item.seq + 1)
            apply(item.event)
        }
    }

    private mutating func apply(_ event: AgentEvent) {
        switch event {
        case let .sessionStarted(_, mode, modes, commands):
            agentMode = mode
            availableModes = modes
            // Only replace when the session actually offered some. A later
            // event carrying an empty list would otherwise empty the picker.
            if !commands.isEmpty { availableCommands = commands }

        case let .message(role, text):
            // Chunks of one message coalesce. One row per chunk would render a
            // streamed sentence as a column of one-word paragraphs.
            if case let .message(lastRole, lastText) = rows.last, lastRole == role {
                rows[rows.count - 1] = .message(role: role, text: lastText + text)
            } else {
                rows.append(.message(role: role, text: text))
            }

        case let .toolCall(id, title, kind, status, locations):
            rows.append(.tool(ToolRow(
                id: id, title: title, kind: kind, status: status,
                locations: locations, content: nil, diff: nil)))

        case let .toolUpdate(id, status, content, diff):
            // Mutate the call in place. Appending would fill the transcript
            // with duplicates of one tool reporting progress.
            guard let index = rows.lastIndex(where: {
                if case let .tool(t) = $0 { return t.id == id }
                return false
            }) else {
                rows.append(.tool(ToolRow(
                    id: id, title: id, kind: "", status: status,
                    locations: [], content: content, diff: diff)))
                return
            }
            guard case var .tool(tool) = rows[index] else { return }
            tool.status = status
            if let content { tool.content = content }
            if let diff { tool.diff = diff }
            rows[index] = .tool(tool)

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

        case let .modeSet(mode):
            agentMode = mode

        case .turnEnded:
            // Nothing to draw. The row's activity badge is the daemon's to
            // decide and arrives on the terminal, not here.
            break

        case let .gap(reason):
            // Never merged, never dropped. A gap that could be swallowed by a
            // neighbouring message would leave the user believing a transcript
            // is complete when it is not.
            rows.append(.gap(reason))
        }
    }
}
