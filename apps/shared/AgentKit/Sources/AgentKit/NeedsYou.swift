import Foundation

// Everything a person has to act on, across every runner.
//
// Each runner's daemon computes its own list (`crates/daemon/src/needs_you.rs`),
// and the client core renders it as JSON once (`crates/client/src/
// needs_you_json.rs`), shared by `farcooler needs-you --json`, which the Mac
// reads, and the FFI's `needs_you`, which the phones read. What is left for an
// app is here: decode that one shape, merge the runners by rank, and count. See
// spec §2 of `docs/superpowers/specs/2026-09-28-workspace-ui-design.md`.
//
// Nothing here decides what an item IS. That is the daemon's, because only the
// daemon holds all four facts (held asks, chat permissions, agent activity,
// tasks). The one exception is a runner too old to compute a list at all, and
// `NeedsYou.derived(fromTerminals:)` says exactly how little it guesses there.
//
// The keys are the proto's field names in snake_case, and the tests decode
// `test/fixtures/needs-you.json`, which holds the daemon's own values, so a key
// renamed on either end fails a test rather than going quiet on a phone.

/// What an item is about, in rank order: an ask outranks a block outranks a
/// decision outranks a review.
///
/// `unknown` is a kind a newer runner sends that this build doesn't define. It
/// decodes rather than failing the list, and sorts after every kind this build
/// knows: an item nobody here can describe shouldn't push aside one it can.
public enum NeedsYouKind: String, Hashable, Sendable, Decodable {
    case ask, blocked, decision, review, unknown

    public init(from decoder: Decoder) throws {
        let word = try decoder.singleValueContainer().decode(String.self)
        self = NeedsYouKind(rawValue: word) ?? .unknown
    }
}

/// A task named from somewhere else: an item's subject, and each row of a
/// worktree's `open_tasks`. `TaskRef` on the wire.
public struct NeedsYouTask: Hashable, Sendable, Decodable {
    /// The task's uuid, as a `TaskRow` carries it.
    public var id: String
    /// `bil-9`.
    public var key: String
    public var title: String
    /// The runner's word: `needs_decision`, `in_review`, `in_progress`, …
    public var status: String

    public init(id: String, key: String, title: String, status: String) {
        self.id = id
        self.key = key
        self.title = title
        self.status = status
    }
}

/// The terminal an item is about. `TerminalRef` on the wire.
public struct NeedsYouTerminal: Hashable, Sendable, Decodable {
    public var id: String
    public var worktreeID: String
    /// What the runner calls it: the agent running in it (`claude`), else its
    /// title.
    public var label: String
    /// `shell`, `agent` or `orchestrator`.
    public var role: String
    /// `terminal`, `agent` or `changes`.
    public var paneMode: String
    public var chatCapable: Bool

    public var isOrchestrator: Bool { role == "orchestrator" }

    public init(
        id: String, worktreeID: String, label: String, role: String, paneMode: String,
        chatCapable: Bool
    ) {
        self.id = id
        self.worktreeID = worktreeID
        self.label = label
        self.role = role
        self.paneMode = paneMode
        self.chatCapable = chatCapable
    }

    enum CodingKeys: String, CodingKey {
        case id, label, role
        case worktreeID = "worktree_id"
        case paneMode = "pane_mode"
        case chatCapable = "chat_capable"
    }
}

/// The worktree an item's work is in. `WorktreeRef` on the wire.
public struct NeedsYouWorktree: Hashable, Sendable, Decodable {
    public var id: String
    public var name: String
    public var branch: String
    public var insertions: UInt32
    public var deletions: UInt32

    public init(id: String, name: String, branch: String, insertions: UInt32, deletions: UInt32) {
        self.id = id
        self.name = name
        self.branch = branch
        self.insertions = insertions
        self.deletions = deletions
    }
}

/// One button an item offers.
///
/// `id` is what gets sent: an ask option's id (`allow`, `deny`) for
/// `terminal.agent_answer`, a decision option's text for `task.note`, or
/// `open`, which sends nothing.
public struct NeedsYouAction: Hashable, Sendable, Decodable {
    public var id: String
    /// "Allow touch x", "Deny", or a decision's option as written.
    public var title: String
    public var destructive: Bool
    public var primary: Bool

    public init(id: String, title: String, destructive: Bool, primary: Bool) {
        self.id = id
        self.title = title
        self.destructive = destructive
        self.primary = primary
    }

    /// Whether pressing it sends nothing and only goes somewhere.
    public var isOpen: Bool { id == "open" }
}

/// One thing a person has to act on.
public struct NeedsYouItem: Hashable, Sendable, Decodable, Identifiable {
    /// Stable across reads on one runner: `ask:<ask id>`, `blocked:<terminal>`,
    /// `decision:<task>` or `review:<task>`. Two runners can hold the same id,
    /// so a list of several runners' items is keyed by `key`, not this.
    public var itemID: String
    public var kind: NeedsYouKind
    /// The subject's other, less urgent signals. Informational: an item with
    /// `also` is still one item, and counts once.
    public var also: [NeedsYouKind]
    /// Where it sorts: smaller first. `Terminal.rank`'s scale, a tier per kind
    /// and then the oldest first, and a duration rather than a clock reading,
    /// which is what lets two runners' items be compared at all.
    public var rank: UInt32
    /// When the signal began, by the runner's clock. For showing an age, never
    /// for sorting: two runners' clocks don't agree.
    public var since: Date
    /// The workspace it's counted under, or nil for none (its repository's
    /// Unclaimed group).
    public var workspaceID: String?
    /// The workspace's name, or empty for none.
    public var workspaceName: String
    public var repositoryID: String?
    public var task: NeedsYouTask?
    public var terminal: NeedsYouTerminal?
    /// Absent below Control scope.
    public var worktree: NeedsYouWorktree?
    /// One row wide, and already redacted on the runner.
    public var question: String
    /// An ask's command, or a review's `+18 −40`. Absent below Control scope.
    public var detail: String?
    /// What `terminal.agent_answer` takes. Absent below Control scope.
    public var askID: String?
    /// Empty below Control scope, where the only button is Open.
    public var actions: [NeedsYouAction]
    /// The runner it came from. Not on the wire: set by `NeedsYou.merge`.
    public var runner: String

    /// This item's identity across every runner.
    public var key: String { "\(runner)\u{1F}\(itemID)" }
    public var id: String { key }

    public init(
        id: String, kind: NeedsYouKind, also: [NeedsYouKind] = [], rank: UInt32, since: Date,
        workspaceID: String? = nil, workspaceName: String = "", repositoryID: String? = nil,
        task: NeedsYouTask? = nil, terminal: NeedsYouTerminal? = nil,
        worktree: NeedsYouWorktree? = nil, question: String, detail: String? = nil,
        askID: String? = nil, actions: [NeedsYouAction] = [], runner: String = ""
    ) {
        self.itemID = id
        self.kind = kind
        self.also = also
        self.rank = rank
        self.since = since
        self.workspaceID = workspaceID
        self.workspaceName = workspaceName
        self.repositoryID = repositoryID
        self.task = task
        self.terminal = terminal
        self.worktree = worktree
        self.question = question
        self.detail = detail
        self.askID = askID
        self.actions = actions
        self.runner = runner
    }

    enum CodingKeys: String, CodingKey {
        case itemID = "id"
        case kind, also, rank, since, task, terminal, worktree, question, detail, actions
        case workspaceID = "workspace_id"
        case workspaceName = "workspace_name"
        case repositoryID = "repository_id"
        case askID = "ask_id"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        itemID = try c.decode(String.self, forKey: .itemID)
        kind = try c.decode(NeedsYouKind.self, forKey: .kind)
        also = try c.decodeIfPresent([NeedsYouKind].self, forKey: .also) ?? []
        rank = try c.decode(UInt32.self, forKey: .rank)
        // Unix milliseconds on the wire.
        let millis = try c.decode(Int64.self, forKey: .since)
        since = Date(timeIntervalSince1970: TimeInterval(millis) / 1000)
        workspaceID = try c.decodeIfPresent(String.self, forKey: .workspaceID)
        workspaceName = try c.decodeIfPresent(String.self, forKey: .workspaceName) ?? ""
        repositoryID = try c.decodeIfPresent(String.self, forKey: .repositoryID)
        task = try c.decodeIfPresent(NeedsYouTask.self, forKey: .task)
        terminal = try c.decodeIfPresent(NeedsYouTerminal.self, forKey: .terminal)
        worktree = try c.decodeIfPresent(NeedsYouWorktree.self, forKey: .worktree)
        question = try c.decode(String.self, forKey: .question)
        detail = try c.decodeIfPresent(String.self, forKey: .detail)
        askID = try c.decodeIfPresent(String.self, forKey: .askID)
        actions = try c.decodeIfPresent([NeedsYouAction].self, forKey: .actions) ?? []
        runner = ""
    }
}

/// One runner's list, as `needs_you_json` writes it: `{"items": [...]}`.
public struct NeedsYouList: Hashable, Sendable, Decodable {
    public var items: [NeedsYouItem]

    public init(items: [NeedsYouItem]) { self.items = items }
}

public enum NeedsYou {
    /// One tier's width on the rank scale, the daemon's `TIER_SPAN` and
    /// `farcooler_core::feed`'s.
    static let tierSpan: UInt32 = 100_000_000

    /// Every runner's items in one list: by rank, then by runner.
    ///
    /// Rank, not `since`, because a rank is a duration measured on its own
    /// runner and a `since` is a reading of that runner's clock: a build box
    /// whose clock runs two minutes slow would otherwise put its newest ask
    /// ahead of an older one on the studio. The runner name breaks ties so the
    /// order doesn't shuffle between reads. A kind this build doesn't know
    /// sorts after every kind it does, whatever its rank.
    ///
    /// Each item comes back with its `runner` set to its key here.
    public static func merge(_ runners: [String: [NeedsYouItem]]) -> [NeedsYouItem] {
        runners
            .flatMap { runner, items in
                items.map { item in
                    var item = item
                    item.runner = runner
                    return item
                }
            }
            .sorted { a, b in
                let ua = a.kind == .unknown, ub = b.kind == .unknown
                if ua != ub { return ub }
                if a.rank != b.rank { return a.rank < b.rank }
                if a.runner != b.runner { return a.runner < b.runner }
                return a.itemID < b.itemID
            }
    }

    /// What an older runner's section says in place of what it can't send.
    public static func olderRunnerNote(runner: String) -> String {
        "Update Far Cooler on \(runner) to see decisions and asks here."
    }
}

extension Collection where Element == NeedsYouItem {
    /// How many items are counted under `workspaceID`.
    ///
    /// Items, not signals: an ask with a decision in its `also` is one thing
    /// to act on and counts once. The badge on a workspace's row is this
    /// number, and the Needs You total is `count`.
    ///
    /// A workspace id is a runner's uuid, so this needs no runner to tell two
    /// workspaces apart.
    public func count(in workspaceID: String) -> Int {
        filter { $0.workspaceID == workspaceID }.count
    }

    /// How many items with no workspace belong to `repositoryID`: its
    /// Unclaimed group's count.
    public func unclaimedCount(inRepository repositoryID: String) -> Int {
        filter { $0.workspaceID == nil && $0.repositoryID == repositoryID }.count
    }
}

// MARK: - Older runners

extension NeedsYou {
    /// What an older runner already sends about one terminal: enough to say
    /// it's blocked, and nothing more.
    ///
    /// A value rather than a protocol, because the Mac's `Terminal` and the
    /// phone's are different types and neither knows the worktree it's in;
    /// each app fills one of these from its own model.
    public struct OlderPane: Hashable, Sendable {
        public var terminal: NeedsYouTerminal
        /// The runner's activity word: `blocked`, `working`, `done`, …
        public var activity: String?
        /// `Terminal.rank`, or nil from a runner too old to send one.
        public var rank: UInt32?
        /// When `activity` began, or nil when the runner didn't say.
        public var activitySince: Date?
        /// Already redacted on the runner. Empty reads as none.
        public var blockedQuestion: String?
        /// The terminal's workspace, else its worktree's owner: the daemon's
        /// order after the task's, which an older runner can't supply.
        public var workspaceID: String?
        public var repositoryID: String?
        /// The task the pane was dispatched for (its `taskId`), named from
        /// its worktree's open tasks, or nil. Never the worktree's one open
        /// task for a pane with no `taskId`: that's the daemon's subject rule,
        /// and `TaskLink`'s fallback is for display only.
        public var task: NeedsYouTask?
        /// The worktree it runs in.
        public var worktree: NeedsYouWorktree?

        public init(
            terminal: NeedsYouTerminal, activity: String?, rank: UInt32?, activitySince: Date?,
            blockedQuestion: String?, workspaceID: String?, repositoryID: String?,
            task: NeedsYouTask? = nil, worktree: NeedsYouWorktree? = nil
        ) {
            self.terminal = terminal
            self.activity = activity
            self.rank = rank
            self.activitySince = activitySince
            self.blockedQuestion = blockedQuestion
            self.workspaceID = workspaceID
            self.repositoryID = repositoryID
            self.task = task
            self.worktree = worktree
        }
    }

    /// The items a runner without `needs_you` can be said to have (spec §2.6).
    ///
    /// **Blocked agents only.** An older runner sends no ask ids or options,
    /// no board with the fleet, and no way to tell a failed turn that's been
    /// seen from one that hasn't, so it gets no asks, decisions or reviews,
    /// and nothing is guessed. Its section says so with `olderRunnerNote`.
    ///
    /// **Re-tiered.** `Terminal.rank`'s tier 0 is Blocked, and the item
    /// scale's tier 0 is ask. Copied as it stands, a derived block would
    /// outrank a real ask on a current runner, so it moves into the blocked
    /// tier, keeping its age within it. A pane with no rank is the youngest in
    /// that tier: a runner that can't say how long it's waited isn't claimed
    /// to have waited longest.
    ///
    /// Hidden worktrees' panes belong in `panes` too: their items still
    /// count (spec §2.2).
    ///
    /// `now` stands in for `since` when the runner didn't send one.
    /// Android's `NeedsYouItems.derived` is the same rule.
    public static func derived(fromTerminals panes: [OlderPane], now: Date = Date()) -> [NeedsYouItem] {
        panes
            .filter { $0.activity == "blocked" }
            .map { pane in
                let age = pane.rank.map { $0 % tierSpan } ?? (tierSpan - 1)
                return NeedsYouItem(
                    id: "blocked:\(pane.terminal.id)",
                    kind: .blocked,
                    rank: tierSpan + age,
                    since: pane.activitySince ?? now,
                    workspaceID: pane.workspaceID,
                    repositoryID: pane.repositoryID,
                    task: pane.task,
                    terminal: pane.terminal,
                    worktree: pane.worktree,
                    question: pane.blockedQuestion.flatMap { $0.isEmpty ? nil : $0 }
                        ?? "\(pane.terminal.label) needs you",
                    actions: [NeedsYouAction(id: "open", title: "Open", destructive: false, primary: false)])
            }
            .sorted { $0.rank < $1.rank }
    }
}
