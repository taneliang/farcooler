import Foundation

// The board, as everything except the drawing of it.
//
// Here rather than beside the view, and the reason is reach rather than the
// Mac package being untested — it declares five test targets and CI runs them
// all. What it does not have is a caller for a rule that lives in a body:
// SwiftUI will evaluate a `View.body` that composes the wrong sentence and
// report nothing, and no suite in that package is chartered to look. Its five
// exist for claims only a rendered view can answer — what a mark paints, which
// cache a view reaches, how big a pane is — and a staleness rule is not one of
// those. Pulled out here it is an ordinary function that `swift test
// --package-path apps/shared/AgentKit` calls on every push, and every sentence
// below is one somebody acts on: "this task stopped moving three days ago" is
// the board's entire reason to exist.
//
// The producer is `farcooler task list --json` and `farcooler task show
// --json` (crates/cli/src/tasks.rs). Field names here are that JSON's, and the
// status words are `status_word`'s, which are the proto's `TaskStatus` names
// lowercased. Nothing here derives state the runner already derived; what it
// derives is how the board should READ, which is a client's own business.
//
// ## The split this file must not collapse
//
// A `Task` is what is understood NOW and every field of it may be revised. A
// `TaskNote` is the record of how that understanding was reached and no field
// of any note may ever change. Correcting the record is a NEW note carrying
// `supersedes`; both stay readable forever.
//
// The store enforces it — `task_notes` has a `BEFORE UPDATE` trigger that
// refuses unconditionally — which means a board offering an "Edit" on a note
// would compile, ship, and fail at runtime in front of a user. `BoardAction`
// at the bottom of this file is the guard, and `TaskBoardModelTests` is what
// makes it a guard rather than a comment.

/// Where a task sits, in the vocabulary the wire uses.
///
/// Raw values are the CLI's words verbatim, which are the proto's
/// `TASK_STATUS_*` names lowercased — so a row on screen reads as the row it
/// came from and there is one spelling of each state in this app.
///
/// Deliberately closed over the seven this build knows, with no `unknown`
/// case. A runner newer than this app can send an eighth, and the honest answer
/// to that is not to invent a column for it or to file it under the backlog —
/// see `TaskBoardModel.unreadable`, which surfaces such a row instead of
/// deciding on its behalf.
public enum TaskStatus: String, CaseIterable, Sendable, Hashable {
    case backlog
    case todo
    case needsDecision = "needs_decision"
    case inProgress = "in_progress"
    case inReview = "in_review"
    case done
    case cancelled

    /// The column heading, in this app's voice rather than the wire's.
    ///
    /// Title case, because these head a column and label a menu item. US
    /// English throughout — "Canceled" with one L on screen, while the raw
    /// value stays `cancelled` because that is what the wire says and this
    /// side does not get to rename it.
    public var title: String {
        switch self {
        case .backlog: return "Backlog"
        case .todo: return "To Do"
        case .needsDecision: return "Needs Decision"
        case .inProgress: return "In Progress"
        case .inReview: return "In Review"
        case .done: return "Done"
        case .cancelled: return "Canceled"
        }
    }

    /// Whether work on this task has stopped for good, either way.
    ///
    /// The two states staleness does not apply to. `done` is finished, not
    /// forgotten, and a board that flagged a task shipped in March would be
    /// training people to ignore the flag by summer. The runner draws the same
    /// line — `TaskListRequest.stale_after_millis` excludes both outright.
    public var isFinished: Bool { self == .done || self == .cancelled }
}

/// Whether a task has stopped moving.
///
/// Two states and not a duration, because the board draws one of two
/// treatments. The duration is still available — see `TaskRow.stoppedFor` —
/// for the sentence that goes under a stale row.
public enum TaskStaleness: Sendable, Hashable {
    case fresh
    case stale
}

/// One thing a task is waiting on, with the key a person can type.
///
/// The wire carries a task id here, not a key (`block_json` in
/// crates/cli/src/tasks.rs). Resolving it is
/// `TaskBoardModel.resolvingBlocks(_:)`, which can do it because the board
/// already holds every row.
public struct TaskBlockRef: Equatable, Sendable, Hashable {
    public var key: String
    public var reason: String

    public init(key: String, reason: String) {
        self.key = key
        self.reason = reason
    }
}

/// One checkable thing, and whether it holds.
public struct TaskAcceptanceLine: Equatable, Sendable, Hashable, Identifiable {
    public var id: String
    public var text: String
    public var met: Bool

    public init(id: String, text: String, met: Bool) {
        self.id = id
        self.text = text
        self.met = met
    }
}

/// One card on the board.
public struct TaskRow: Equatable, Sendable, Hashable, Identifiable {
    /// How long a task may sit in one status before the board says so.
    ///
    /// A named constant and not a literal at the comparison, so the one place
    /// this is tuned is findable — and so the number is arguable rather than
    /// buried. A day is the shortest span over which "nothing happened" is
    /// news rather than noise: an agent working a task moves it through
    /// `in_progress` and `in_review` within a session, and a person picking
    /// work up in the morning wants yesterday's untouched `todo` to look
    /// different from the one they just filed.
    public static let staleAfter: TimeInterval = 24 * 60 * 60

    /// The task's UUID, as the wire spells it. `Identifiable` uses it so a
    /// column's `ForEach` is stable across a re-read that reordered rows.
    public var id: String
    /// Short and typeable: `fc-42`. What a person and a prompt both name.
    public var key: String
    public var title: String
    public var status: TaskStatus
    /// When the status last changed — NOT when the board was read.
    ///
    /// The board's most important number, and the one easiest to get wrong by
    /// reading it as "how old is this news". It is how long the task has sat
    /// where it is.
    public var statusSince: Date
    public var intent: String
    public var labels: [String]
    public var acceptance: [TaskAcceptanceLine]
    public var constraints: [String]
    /// The lane this task is using, when it has one. A task exists in the
    /// backlog long before any worktree does.
    public var workspaceID: String?
    /// What this task is waiting on.
    ///
    /// Empty on a row that came from `task list`, which does not carry blocks
    /// — one call for a whole board is what makes the board cheap. It is
    /// filled from `task show` when a card is opened. Absent is therefore "not
    /// asked", not "nothing", and the detail is the only place a block
    /// summary is drawn.
    public var blockedBy: [TaskBlockRef] = []

    public init(
        id: String,
        key: String,
        title: String,
        status: TaskStatus,
        statusSince: Date,
        intent: String = "",
        labels: [String] = [],
        acceptance: [TaskAcceptanceLine] = [],
        constraints: [String] = [],
        workspaceID: String? = nil,
        blockedBy: [TaskBlockRef] = []
    ) {
        self.id = id
        self.key = key
        self.title = title
        self.status = status
        self.statusSince = statusSince
        self.intent = intent
        self.labels = labels
        self.acceptance = acceptance
        self.constraints = constraints
        self.workspaceID = workspaceID
        self.blockedBy = blockedBy
    }

    /// How long this task has sat where it is, as of `now`.
    ///
    /// Never negative. A runner whose clock is ahead of this Mac's hands back a
    /// `status_since` in the future, and a negative age would read as fresh by
    /// luck rather than by decision — clamping says "just now", which is the
    /// honest answer when the two clocks disagree about which just happened.
    public func stoppedFor(at now: Date) -> TimeInterval {
        max(0, now.timeIntervalSince(statusSince))
    }

    /// Whether this task has stopped moving.
    ///
    /// The board's whole job beyond showing state. The failure mode of the
    /// factory is not an agent doing the wrong thing; it is a task sitting in
    /// `todo` that you assumed was in flight, and a board that renders it
    /// identically to one that moved a minute ago is what lets that happen.
    public func staleness(at now: Date) -> TaskStaleness {
        if status.isFinished { return .fresh }
        return stoppedFor(at: now) >= TaskRow.staleAfter ? .stale : .fresh
    }

    /// `staleness(at:)` against the clock, for a caller with no reason to
    /// name a moment.
    public var staleness: TaskStaleness { staleness(at: Date()) }

    /// The sentence under a stale row, or nil for one that is still moving.
    ///
    /// A sentence and not a timestamp: "3 days" is what makes somebody act,
    /// and a date makes them subtract. Only ever hours or days, because
    /// nothing below `staleAfter` gets one at all.
    public func stalenessNote(at now: Date) -> String? {
        guard staleness(at: now) == .stale else { return nil }
        let days = Int(stoppedFor(at: now) / (24 * 60 * 60))
        // A typographic apostrophe, like every other sentence this board draws
        // — "couldn’t" and "can’t" in `TaskBoardStore.trouble`, and every
        // string in `Shortcuts.swift`. One screen with two conventions on it
        // is the kind of thing nobody can name and everybody sees.
        if days <= 1 { return "Hasn’t moved in a day" }
        return "Hasn’t moved in \(days) days"
    }

    /// What this task is waiting on, in one line, or nil if it is waiting on
    /// nothing.
    ///
    /// The keys and not the reasons: the reasons are per edge and belong
    /// beside each one, while the summary answers "can this move at all" at a
    /// glance.
    public var blockedSummary: String? {
        guard !blockedBy.isEmpty else { return nil }
        return "Waiting on " + TaskRow.listed(blockedBy.map(\.key))
    }

    /// The one thing the board asks the person looking at it to do, or nil.
    ///
    /// Only `needs decision` has one, and that is the point rather than an
    /// omission: it is the single status that is waiting on the PERSON. A
    /// board that put a call to action on every column would have none.
    public var callToAction: String? {
        switch status {
        case .needsDecision: return "Answer to unblock this"
        case .backlog, .todo, .inProgress, .inReview, .done, .cancelled: return nil
        }
    }

    /// `a`, `a and b`, `a, b and c`.
    ///
    /// Written out rather than taken from `ListFormatter`, which localizes —
    /// and this string is composed in a suite whose assertions are English. A
    /// board that read differently under a different locale would be a board
    /// whose tests say nothing about what ships.
    static func listed(_ items: [String]) -> String {
        switch items.count {
        case 0: return ""
        case 1: return items[0]
        case 2: return "\(items[0]) and \(items[1])"
        default:
            return items.dropLast().joined(separator: ", ") + " and " + (items.last ?? "")
        }
    }
}

/// A row whose status word this build has no column for.
///
/// A runner ahead of this app can name an eighth status. Three things could be
/// done with such a row and two of them are wrong: dropping it makes work
/// vanish from a board whose whole claim is that it shows the work, and
/// defaulting it to `backlog` reads a finished task as unstarted. So it is
/// carried, with the word the runner used, and the board shows it under a
/// heading that says this app is the one that is behind.
public struct UnreadableTaskRow: Equatable, Sendable, Hashable, Identifiable {
    public var id: String
    public var key: String
    public var title: String
    /// The word the runner sent, verbatim. Shown, because it is the only thing
    /// anybody can act on.
    public var status: String

    public init(id: String, key: String, title: String, status: String) {
        self.id = id
        self.key = key
        self.title = title
        self.status = status
    }
}

/// One column, with its rows in the order the runner listed them.
public struct TaskBoardColumn: Equatable, Sendable, Identifiable {
    public var status: TaskStatus
    public var rows: [TaskRow]

    public var id: String { status.rawValue }
    public var title: String { status.title }

    public init(status: TaskStatus, rows: [TaskRow]) {
        self.status = status
        self.rows = rows
    }
}

/// A repository's board.
public struct TaskBoardModel: Equatable, Sendable {
    /// The order the columns are drawn in.
    ///
    /// `needs decision` leads, and every other column is in the order work
    /// moves. It is out of lifecycle order on purpose: it is the only state
    /// waiting on the person looking at the board, and a column you have to
    /// scan for is a question that goes unanswered for a day. The lifecycle is
    /// still legible — the remaining six are in it, and each card says its own
    /// status anyway.
    public static let order: [TaskStatus] = [
        .needsDecision, .backlog, .todo, .inProgress, .inReview, .done, .cancelled,
    ]

    public var columns: [TaskBoardColumn]
    /// Rows this build has no column for. Usually empty; never dropped. See
    /// `UnreadableTaskRow`.
    public var unreadable: [UnreadableTaskRow]

    public static let empty = TaskBoardModel(columns: [], unreadable: [])

    public init(columns: [TaskBoardColumn], unreadable: [UnreadableTaskRow] = []) {
        self.columns = columns
        self.unreadable = unreadable
    }

    /// Every row on the board, in column order.
    public var rows: [TaskRow] { columns.flatMap(\.rows) }

    /// How many tasks are waiting on the person looking at the board.
    ///
    /// The one count worth putting in a title bar. Zero is a board with
    /// nothing to answer, which is a different thing from an empty board and
    /// must not read as one.
    public var waitingOnYou: Int {
        columns.first { $0.status == .needsDecision }?.rows.count ?? 0
    }

    /// What the board says in its title bar, or nil when nothing is waiting.
    ///
    /// Here rather than in the view, like every other sentence this board
    /// draws. It was the one exception, composed inside `TaskBoardSheet`'s
    /// body under a comment in that same file saying not to — a copy rule a
    /// file states and then breaks is worse than one it never claimed, because
    /// the next reader trusts the comment.
    ///
    /// Nil at zero, and that is the copy decision rather than a missing case.
    /// A badge reading "0 tasks are waiting on you" is a badge people learn to
    /// stop seeing, and the fact worth showing is the presence of a question,
    /// not the arithmetic.
    public var waitingSentence: String? { TaskBoardModel.waitingSentence(waitingOnYou) }

    /// The same sentence for a count that did not come off a board.
    ///
    /// Static so the plural boundary can be asserted at 0, 1 and 2 without
    /// building three boards to carry three counts — the boundary is the whole
    /// reason this is a function and not a literal.
    ///
    /// Written out rather than taken from a formatter, for the reason
    /// `TaskRow.listed` gives: this suite's assertions are English, and copy
    /// that changed under another locale would be copy the tests say nothing
    /// about.
    public static func waitingSentence(_ count: Int) -> String? {
        guard count > 0 else { return nil }
        // The verb agrees as well as the noun. "1 tasks are waiting" and
        // "2 task is waiting" are both one careless edit away, and both read as
        // broken English in a title bar rather than as a bug anybody files.
        return count == 1
            ? "1 task is waiting on you"
            : "\(count) tasks are waiting on you"
    }

    /// The key for a task id, when this board holds that task.
    public func key(forTaskID id: String) -> String? {
        rows.first { $0.id == id }?.key
    }

    /// Turn a detail's raw blocks into keys a person can read.
    ///
    /// An id this board does not hold falls back to its short id rather than
    /// being dropped. That case is real: a task can be blocked by one that has
    /// since been filtered out of the list, and a card that showed a shorter
    /// list of blockers than the task actually has would say it is ready to
    /// move when it is not. A short id is worse copy than a key and is still
    /// something you can type into `farcooler task show`.
    public func resolvingBlocks(_ raw: [RawTaskBlock]) -> [TaskBlockRef] {
        raw.map { block in
            TaskBlockRef(key: key(forTaskID: block.blockedBy) ?? block.short, reason: block.reason)
        }
    }
}

/// One edge as `task show --json` sends it: an id, not a key.
public struct RawTaskBlock: Equatable, Sendable, Decodable {
    public var blockedBy: String
    public var short: String
    public var reason: String

    enum CodingKeys: String, CodingKey {
        case blockedBy = "blocked_by"
        case short
        case reason
    }

    public init(blockedBy: String, short: String, reason: String) {
        self.blockedBy = blockedBy
        self.short = short
        self.reason = reason
    }
}

// ---------------------------------------------------------------------------
// Decoding
//
// Hand-written rather than synthesized, for the reason every model in this
// tree that meets a runner is: Swift's synthesized `Decodable` throws on a
// missing key, so one field added to the CLI's JSON after an app shipped would
// fail the decode of the ENTIRE board and show "no tasks" for a repository
// full of them. Everything past the four fields a board cannot draw without is
// read with `decodeIfPresent`.
// ---------------------------------------------------------------------------

extension TaskBoardModel {
    /// The board as `farcooler task list --json` sends it.
    ///
    /// Throws only when the payload is not that shape at all. A single row
    /// missing a field it needs is skipped rather than fatal — see
    /// `WireTask.row` — because one unreadable row must not cost the other
    /// forty.
    public static func decode(_ data: Data) throws -> TaskBoardModel {
        let list = try JSONDecoder().decode(WireTaskList.self, from: data)
        return board(from: list.tasks)
    }

    /// Sort wire rows into columns, keeping the ones this build cannot place.
    public static func board(from wire: [WireTask]) -> TaskBoardModel {
        var rows: [TaskStatus: [TaskRow]] = [:]
        var unreadable: [UnreadableTaskRow] = []
        for task in wire {
            guard let status = TaskStatus(rawValue: task.status) else {
                unreadable.append(
                    UnreadableTaskRow(
                        id: task.id, key: task.key, title: task.title, status: task.status))
                continue
            }
            rows[status, default: []].append(task.row(status: status))
        }
        // Every column, including the empty ones. A board that hid its empty
        // columns would move sideways under you every time a task was filed,
        // and "nothing is in review" is worth seeing.
        let columns = order.map { TaskBoardColumn(status: $0, rows: rows[$0] ?? []) }
        return TaskBoardModel(columns: columns, unreadable: unreadable)
    }
}

/// `{"tasks": [...]}`, which is what `task list --json` prints.
public struct WireTaskList: Decodable, Sendable {
    public var tasks: [WireTask]
}

/// One task exactly as the CLI spells it, before this app has an opinion.
///
/// A separate type from `TaskRow` on purpose: `status` here is the runner's
/// word and may be one this build has never heard of, which `TaskRow.status`
/// cannot represent and must not have to.
public struct WireTask: Decodable, Sendable {
    public var id: String
    public var key: String
    public var title: String
    public var status: String
    /// Unix milliseconds, from the wire.
    public var statusSince: Int64
    public var intent: String
    public var labels: [String]
    public var constraints: [String]
    public var acceptance: [WireAcceptance]
    public var workspaceID: String?

    enum CodingKeys: String, CodingKey {
        case id, key, title, status, intent, labels, constraints, acceptance
        case statusSince = "status_since"
        case workspaceID = "workspace_id"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        // The four a card cannot be drawn without. A row missing one of these
        // is a row from a producer this app does not understand at all.
        id = try c.decode(String.self, forKey: .id)
        key = try c.decode(String.self, forKey: .key)
        title = try c.decode(String.self, forKey: .title)
        status = try c.decode(String.self, forKey: .status)
        // Everything else defaults. A zero `status_since` is the epoch, which
        // reads as very stale — correct for a runner that would not say, and
        // loud rather than quiet, which is the right direction to fail in for
        // a value whose whole job is to make a stopped task visible.
        statusSince = try c.decodeIfPresent(Int64.self, forKey: .statusSince) ?? 0
        intent = try c.decodeIfPresent(String.self, forKey: .intent) ?? ""
        labels = try c.decodeIfPresent([String].self, forKey: .labels) ?? []
        constraints = try c.decodeIfPresent([String].self, forKey: .constraints) ?? []
        acceptance = try c.decodeIfPresent([WireAcceptance].self, forKey: .acceptance) ?? []
        workspaceID = try c.decodeIfPresent(String.self, forKey: .workspaceID)
    }

    /// This wire row as a card, given the status it was placed under.
    func row(status: TaskStatus) -> TaskRow {
        TaskRow(
            id: id,
            key: key,
            title: title,
            status: status,
            statusSince: Date(timeIntervalSince1970: Double(statusSince) / 1000),
            intent: intent,
            labels: labels,
            acceptance: acceptance.map {
                TaskAcceptanceLine(id: $0.id, text: $0.text, met: $0.met)
            },
            constraints: constraints,
            workspaceID: workspaceID)
    }
}

public struct WireAcceptance: Decodable, Sendable {
    public var id: String
    public var text: String
    public var met: Bool
}

// ---------------------------------------------------------------------------
// The record
//
// Read-only here, and that is not a stage this will grow out of. A note is
// what was understood at a moment; the board renders it and never offers to
// change it. See `BoardAction.rewritesTheRecord`.
// ---------------------------------------------------------------------------

/// What one entry in the record is.
///
/// Raw values are `kind_word`'s in crates/cli/src/tasks.rs, which are the
/// proto's `TASK_NOTE_KIND_*` names lowercased.
public enum TaskNoteKind: String, CaseIterable, Sendable, Hashable {
    case decision
    case finding
    case question
    case answer
    case progress
    case comment
    case statusChange = "status_change"
    case created

    /// The label above a note. Title case, like every other heading.
    public var title: String {
        switch self {
        case .decision: return "Decision"
        case .finding: return "Finding"
        case .question: return "Question"
        case .answer: return "Answer"
        case .progress: return "Progress"
        case .comment: return "Comment"
        case .statusChange: return "Status Change"
        case .created: return "Created"
        }
    }

    /// Whether a person wrote this or a transaction did.
    ///
    /// `status_change` and `created` are written by the store, in the same
    /// transaction that moves or makes a task — the CLI refuses to let anybody
    /// append one. They read as history rather than as somebody's word, and
    /// the board draws them quieter for it.
    public var isMachineWritten: Bool { self == .statusChange || self == .created }
}

/// One entry in the record, as the board draws it.
public struct TaskNoteRow: Equatable, Sendable, Hashable, Identifiable {
    public var id: String
    public var kind: TaskNoteKind
    /// `user`, `manager`, or `agent:<uuid>`, verbatim from the wire.
    public var actor: String
    public var at: Date
    public var body: String
    /// The note this one replaces, when a later understanding replaced an
    /// earlier one. Both stay readable forever; this is what lets the board
    /// draw the line between them.
    public var supersedes: String?

    public init(
        id: String, kind: TaskNoteKind, actor: String, at: Date, body: String,
        supersedes: String? = nil
    ) {
        self.id = id
        self.kind = kind
        self.actor = actor
        self.at = at
        self.body = body
        self.supersedes = supersedes
    }

    /// Who wrote it, as a person reads it.
    ///
    /// Never the raw word: `agent:0198f2c0-…` is an id in a byline, which is
    /// noise on every row and only ever useful on one. The uuid is still on
    /// `actor` for anything that needs to match against it.
    public var byline: String {
        switch actor {
        case "user": return "You"
        case "manager": return "The manager"
        default: return actor.hasPrefix("agent:") ? "An agent" : "Someone else"
        }
    }
}

/// One note as `task show --json` sends it.
public struct WireTaskNote: Decodable, Sendable {
    public var id: String
    public var kind: String
    public var actor: String
    public var at: Int64
    public var body: String
    public var supersedes: String?

    enum CodingKeys: String, CodingKey {
        case id, kind, actor, at, body, supersedes
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = try c.decode(String.self, forKey: .kind)
        actor = try c.decodeIfPresent(String.self, forKey: .actor) ?? ""
        at = try c.decodeIfPresent(Int64.self, forKey: .at) ?? 0
        body = try c.decodeIfPresent(String.self, forKey: .body) ?? ""
        supersedes = try c.decodeIfPresent(String.self, forKey: .supersedes)
    }
}

/// `{"task": …, "notes": […], "blocks": […]}`, which is what `task show
/// --json` prints.
public struct TaskDetailModel: Equatable, Sendable {
    public var blocks: [RawTaskBlock]
    public var notes: [TaskNoteRow]
    /// Entries whose kind this build has no name for, kept as a count rather
    /// than drawn. The record's claim is that nothing is lost; a reader who
    /// can see six of eight entries and is told so can go to the CLI, while
    /// one silently shown six cannot.
    public var unreadableNotes: Int

    public static let empty = TaskDetailModel(blocks: [], notes: [], unreadableNotes: 0)

    public init(blocks: [RawTaskBlock], notes: [TaskNoteRow], unreadableNotes: Int) {
        self.blocks = blocks
        self.notes = notes
        self.unreadableNotes = unreadableNotes
    }

    public static func decode(_ data: Data) throws -> TaskDetailModel {
        let wire = try JSONDecoder().decode(WireTaskDetail.self, from: data)
        var notes: [TaskNoteRow] = []
        var unreadable = 0
        for note in wire.notes {
            guard let kind = TaskNoteKind(rawValue: note.kind) else {
                unreadable += 1
                continue
            }
            notes.append(
                TaskNoteRow(
                    id: note.id,
                    kind: kind,
                    actor: note.actor,
                    at: Date(timeIntervalSince1970: Double(note.at) / 1000),
                    body: note.body,
                    supersedes: note.supersedes))
        }
        return TaskDetailModel(blocks: wire.blocks, notes: notes, unreadableNotes: unreadable)
    }
}

public struct WireTaskDetail: Decodable, Sendable {
    public var notes: [WireTaskNote]
    public var blocks: [RawTaskBlock]

    enum CodingKeys: String, CodingKey { case notes, blocks }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        notes = try c.decodeIfPresent([WireTaskNote].self, forKey: .notes) ?? []
        blocks = try c.decodeIfPresent([RawTaskBlock].self, forKey: .blocks) ?? []
    }
}

// ---------------------------------------------------------------------------
// What the board is allowed to do
//
// The board is the one surface where a person could quietly undo the design's
// load-bearing idea: current understanding is mutable and lives on the task
// row, while the record of how you got there is append-only and lives in
// typed notes that can be superseded but never edited.
//
// So every write the board offers is a value here, carrying the daemon method
// it would call, and the view builds its menus out of these rather than out of
// buttons written by hand. That is what makes `rewritesTheRecord` a guard
// instead of a comment: a future "Edit Note…" would have to arrive as a
// `BoardAction`, and the suite iterates every one of them.
// ---------------------------------------------------------------------------

/// One thing the board offers to do.
public struct BoardAction: Equatable, Sendable, Hashable, Identifiable {
    /// Title case, because these are buttons and menu items.
    public var title: String
    /// The daemon method this would call, or nil for an action that writes
    /// nothing at all.
    public var method: String?

    public var id: String { title }

    public init(title: String, method: String?) {
        self.title = title
        self.method = method
    }

    /// Whether a method would change or remove something already written to
    /// the record.
    ///
    /// `task.note` — the append — is the ONLY note method the protocol has,
    /// and a correction is that same append carrying `supersedes`. So any
    /// other method naming a note is by construction one that edits or deletes
    /// one, and there must never be an affordance for it: `task_notes` has a
    /// `BEFORE UPDATE` trigger that refuses unconditionally, so such a button
    /// would compile, ship, and fail at runtime in front of a user.
    ///
    /// Deliberately a rule over the method WORD rather than a list of the
    /// actions that exist today. A guard written as "none of the four actions
    /// we currently have is an edit" stays green until somebody adds a fifth,
    /// which is precisely when it needed to speak.
    public static func rewritesTheRecord(_ method: String) -> Bool {
        method.contains("note") && method != "task.note"
    }

    /// Whether this action would rewrite the record. Nil methods write
    /// nothing, so they cannot.
    public var rewritesTheRecord: Bool {
        guard let method else { return false }
        return BoardAction.rewritesTheRecord(method)
    }
}

/// One menu item that moves a task, and the status it moves it to.
///
/// The status is carried rather than looked back up from the title. A view
/// that matched a menu item to a status by its label would be one rename away
/// from moving a task to the wrong column, and the label is copy — the thing
/// most likely to be rewritten.
public struct BoardMove: Equatable, Sendable, Identifiable {
    public var status: TaskStatus
    public var action: BoardAction

    public var id: String { status.rawValue }

    public init(status: TaskStatus, action: BoardAction) {
        self.status = status
        self.action = action
    }
}

extension TaskBoardModel {
    /// Where this task may be moved to, as menu items.
    ///
    /// The status it is already in is left out — a menu item that does nothing
    /// is a menu item somebody clicks twice to check. In board order, so the
    /// menu reads the way the columns do.
    public static func moves(for row: TaskRow) -> [BoardMove] {
        order
            .filter { $0 != row.status }
            .map {
                BoardMove(
                    status: $0, action: BoardAction(title: $0.title, method: "task.set_status"))
            }
    }

    /// What the board would offer to do with a note that is already written.
    ///
    /// **No view draws this today, and that is stated rather than implied.**
    /// The record on `TaskCard` is text with `.textSelection` and nothing
    /// else: no menu, no gesture, no field. So the shipped board is stricter
    /// than this list, and a reader must not take the list for a description
    /// of what is on screen.
    ///
    /// It is here for two reasons. It is the declared place a note affordance
    /// goes when one is wanted, so that it arrives as a `BoardAction` the
    /// suite walks rather than as a `Button` written by hand in a view where
    /// nothing checks it. And it keeps `allActions` covering the note axis at
    /// all, so the rule below is exercised over a note action and not only
    /// over status moves.
    ///
    /// One item, and the short list is the design rather than an unfinished
    /// menu. A note is a fact about what was understood at a moment; the only
    /// thing a reader legitimately wants from one is to take its text
    /// somewhere else. Revising the understanding is a new note — which the
    /// CLI writes with `farcooler task note --supersedes`, and which this
    /// board does not compose.
    public static func noteActions() -> [BoardAction] {
        [BoardAction(title: "Copy", method: nil)]
    }

    /// Every write this board can make, for the suite to walk.
    ///
    /// A row is needed because `moves(for:)` depends on where the task
    /// currently sits — so the caller supplies one per status to cover them
    /// all.
    public static func allActions(for rows: [TaskRow]) -> [BoardAction] {
        rows.flatMap { moves(for: $0).map(\.action) } + noteActions()
    }
}
