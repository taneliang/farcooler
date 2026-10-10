import Foundation
import Observation

/// The agents at work in a pane, pinned above its composer (ov-453), as
/// claude's own panel lists them under its box: "main", then every subagent
/// still running, each with its type, what it's doing, how long it has run
/// and the tokens its newest call used.
///
/// The inline subagent rows stay in the transcript as history; this is what
/// doesn't scroll away. Built from the rows a store holds, so the Mac and the
/// phone list the same agents in the same words.
public enum AgentTray {
    public struct Entry: Equatable, Identifiable, Sendable {
        /// `AgentTray.mainID`, or the subagent's row id.
        public var id: String
        public var isMain: Bool
        /// "main", or the agent's type in words: "General purpose".
        public var title: String
        /// What it was asked to do, in claude's few words; empty for main.
        public var description: String
        /// What it's doing now: its newest call, as `Name summary`.
        public var action: String
        public var running: Bool
        public var startedMs: Int64?
        public var endedMs: Int64?
        public var tokens: Int?
        /// What its own rows are asked for by; nil for main, and for an agent
        /// the runner hasn't named yet.
        public var agentId: String?

        public init(
            id: String, isMain: Bool, title: String, description: String = "", action: String = "", running: Bool,
            startedMs: Int64? = nil, endedMs: Int64? = nil, tokens: Int? = nil, agentId: String? = nil
        ) {
            self.id = id
            self.isMain = isMain
            self.title = title
            self.description = description
            self.action = action
            self.running = running
            self.startedMs = startedMs
            self.endedMs = endedMs
            self.tokens = tokens
            self.agentId = agentId
        }
    }

    public static let mainID = "tray:main"

    /// The tray from `store`'s rows: empty while no subagent runs.
    @MainActor
    public static func entries(_ store: AgentRowStore) -> [Entry] {
        entries(ids: store.ids, subagentIds: store.subagentIds) { store.box($0)?.row }
    }

    /// The tray from rows found by id: `ids` every row oldest first,
    /// `subagentIds` the `Subagent` rows among them. Empty while none runs;
    /// otherwise main first, then each running agent in the order it was
    /// launched.
    public static func entries(ids: [String], subagentIds: [String], row: (String) -> AgentRow?) -> [Entry] {
        let running: [Entry] = subagentIds.compactMap { id in
            guard case .subagent(let sub)? = row(id)?.kind, sub.status == .running else { return nil }
            return Entry(
                id: id, isMain: false, title: agentType(sub.agentType), description: sub.description,
                action: sub.currentAction, running: true, startedMs: sub.startedMs, tokens: sub.tokens, agentId: sub.agentId)
        }
        guard !running.isEmpty else { return [] }
        return [main(ids: ids, row: row)] + running
    }

    /// Main, from its newest turn: the call it's running, else thinking or
    /// working while the turn is open, else waiting on the agents it left
    /// running.
    static func main(ids: [String], row: (String) -> AgentRow?) -> Entry {
        var newest: [AgentRow] = []
        var turn: AgentRow.Turn?
        for id in ids.reversed() {
            guard let found = row(id) else { continue }
            if case .turn(let t) = found.kind {
                turn = t
                break
            }
            newest.append(found)
        }
        let open = turn.map { $0.outcome == nil } ?? false
        var action = open ? working : waiting
        for found in newest {
            if case .tool(let tool) = found.kind, tool.status == .running {
                action = tool.summary.isEmpty ? tool.name : "\(tool.name) \(tool.summary)"
                break
            }
            if case .thinking(let thinking) = found.kind, thinking.endedMs == nil, open {
                action = thinkingNow
                break
            }
        }
        return Entry(
            id: mainID, isMain: true, title: mainTitle, action: action, running: open, startedMs: turn?.startedMs,
            endedMs: open ? nil : turn?.endedMs, tokens: turn?.tokens)
    }

    // MARK: - Words

    /// Claude's own name for the session's agent, as its panel says it.
    public static let mainTitle = "main"
    static let working = "Working"
    static let thinkingNow = "Thinking"
    static let waiting = "Waiting for agents"

    /// A subagent's type as words: `general-purpose` reads "General purpose".
    public static func agentType(_ raw: String) -> String {
        let words = raw.replacingOccurrences(of: "-", with: " ").replacingOccurrences(of: "_", with: " ")
        guard let first = words.first else { return "Agent" }
        return first.uppercased() + words.dropFirst()
    }

    /// "950 tokens", "87.2K tokens", "1.2M tokens"; `short`, the number
    /// alone, for a narrow row.
    public static func tokens(_ count: Int, short: Bool = false) -> String {
        let number = count.formatted(.number.notation(.compactName).precision(.fractionLength(0...1)).locale(Locale(identifier: "en_US")))
        return short ? number : "\(number) \(count == 1 ? "token" : "tokens")"
    }

    /// The tray's header: how many agents run besides main.
    public static func summary(_ entries: [Entry]) -> String {
        let count = entries.filter { !$0.isMain }.count
        return count == 1 ? "1 agent running" : "\(count) agents running"
    }

    /// The whole of one entry as VoiceOver reads it.
    public static func spoken(_ entry: Entry) -> String {
        var parts = [entry.title]
        if !entry.description.isEmpty { parts.append(entry.description) }
        if !entry.action.isEmpty { parts.append(entry.action) }
        if let tokens = entry.tokens { parts.append(Self.tokens(tokens)) }
        return parts.joined(separator: ", ")
    }

    /// The drilled-in view's title: the agent's type and its description.
    public static func title(_ sub: AgentRow.Subagent) -> String {
        sub.description.isEmpty ? agentType(sub.agentType) : "\(agentType(sub.agentType)): \(sub.description)"
    }

    /// Said in a drilled-in view whose agent the runner can't open.
    public static let unopenable = "This agent’s conversation can’t be shown here."
}

/// A pane's agent tray and the subagent it has open (ov-453), on the main
/// actor: whether the tray is folded, and the drilled-in agent's own rows,
/// followed while its view shows.
@MainActor
@Observable
public final class AgentDrill {
    /// One subagent's conversation, open in the pane's place.
    public struct Opened: Identifiable {
        /// The subagent's row id in the pane's rows.
        public let id: String
        public let agentId: String
        public let store: AgentRowStore
    }

    /// The tray shows only its header.
    public var collapsed = false
    public private(set) var opened: Opened?

    @ObservationIgnored private var source: (any AgentRowSource)?
    @ObservationIgnored private var following = false

    public init() {}

    /// Open `entry`'s own conversation from the pane's `source`, which gives
    /// the subagent's (`AgentRowSource.subagent`). False, changing nothing,
    /// for main, for an agent not yet named, or where the runner can't.
    @discardableResult
    public func open(_ entry: AgentTray.Entry, from source: (any AgentRowSource)?, pane: String) -> Bool {
        open(row: entry.id, agentId: entry.agentId, from: source, pane: pane)
    }

    /// `open`, by the subagent's row id and `agentId`.
    @discardableResult
    public func open(row: String, agentId: String?, from source: (any AgentRowSource)?, pane: String) -> Bool {
        guard let agentId, let own = source?.subagent(agentId) else { return false }
        if opened?.agentId == agentId { return true }
        close()
        let store = AgentRowStore(key: "\(pane)#agent:\(agentId)", cache: nil)
        opened = Opened(id: row, agentId: agentId, store: store)
        self.source = own
        following = true
        store.start(own)
        return true
    }

    /// Back to the pane's own conversation.
    public func close() {
        opened?.store.stop()
        opened = nil
        source = nil
        following = false
    }

    /// Follow the open agent only while its view is on screen, as the pane's
    /// own rows are.
    public func follow(_ shown: Bool) {
        guard let opened, let source else { return }
        if shown, !following || opened.store.phase == .unavailable {
            following = true
            opened.store.start(source)
        } else if !shown, following {
            following = false
            opened.store.stop()
        }
    }
}
