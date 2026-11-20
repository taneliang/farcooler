import Foundation

// Native types for the shared Rust task board view.
enum TaskBoardPresenter {
    struct Settings: Decodable, Sendable {
        let day: Double
        let doneFloor: Int
        let sectionLimit: Int
        let liveStates: Set<String>
    }

    struct Status: Decodable, Sendable {
        let wire: String
        let title: String
        let finished: Bool
        let holdsBlocks: Bool
        let action: String?
    }

    struct Clock: Decodable {
        let lastMoved: Double
        let stoppedFor: Double
        let staleNote: String?
        let timeNote: String?
    }

    fileprivate struct Board: Decodable {
        struct Column: Decodable {
            let status: String
            let rows: [BoardCard]
        }
        let columns: [Column]
        let unreadable: [UnreadableTaskRow]
    }

    struct Answer<View: Decodable>: Decodable {
        let taskBoard: View?
    }

    static func ask<View: Decodable>(_ operation: String, _ fields: [String: Any] = [:]) -> View? {
        var request = fields
        request["op"] = operation
        guard let data = try? JSONSerialization.data(withJSONObject: ["schema": 1, "task_board": request]),
            let answer = PlanViewPresenter.ask(String(decoding: data, as: UTF8.self))
        else { return nil }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return (try? decoder.decode(Answer<View>.self, from: Data(answer.utf8)))?.taskBoard
    }

    static let settings: Settings? = ask("settings")
    static let statuses: [Status] = ask("statuses") ?? []
    static let statusByWire = Dictionary(uniqueKeysWithValues: statuses.map { ($0.wire, $0) })
}

public enum TaskStatus: String, CaseIterable, Sendable, Hashable {
    case backlog
    case todo
    case needsDecision = "needs_decision"
    case inProgress = "in_progress"
    case inReview = "in_review"
    case done
    case cancelled

    public var title: String { TaskBoardPresenter.statusByWire[rawValue]?.title ?? rawValue }
    public var isFinished: Bool { TaskBoardPresenter.statusByWire[rawValue]?.finished == true }
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
    /// `in_progress` and `in_review` within a session, so one that has sat
    /// in either overnight is one whose agent stopped.
    public static var staleAfter: TimeInterval { TaskBoardPresenter.settings?.day ?? 86_400 }

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
    public var worktreeID: String?
    /// What this task is waiting on.
    ///
    /// From the list's `waiting_on` (ov-212): the keys of the blockers that
    /// are neither done nor cancelled, so a blocker that finished stops
    /// counting on its own and every row of the board can say it. That list
    /// has no reasons, so they are empty here; `task show` fills them when a
    /// card is opened (`resolvingBlocks`). Shown for Backlog, To Do and In
    /// Progress only, the statuses a block holds back.
    public var blockedBy: [TaskBlockRef] = []
    /// When the task was filed, or nil from a runner too old to say.
    public var createdAt: Date?
    /// When anything on the card last changed — a move, a note, or an edit —
    /// or nil from a runner too old to say. Equal to `createdAt` on a card
    /// nothing has happened to since it was filed. See `timeNote(at:)`.
    public var updatedAt: Date?
    /// The board this task is on: its workspace's id, or nil from a runner
    /// without `workstreams`, which has one board per repository.
    public var workspaceID: String?
    /// What it is waiting for before it starts (ov-212), or nil: nothing
    /// said, or a runner without `task_waits`. Read through `startLine`,
    /// which also knows which statuses it means something in.
    public var wait: TaskWait?
    /// The subagents recorded working it (ov-213): every open one, then the
    /// most recently closed. Empty from a runner without `task_workers`.
    public var workers: [TaskWorker] = []

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
        worktreeID: String? = nil,
        blockedBy: [TaskBlockRef] = [],
        createdAt: Date? = nil,
        updatedAt: Date? = nil,
        workspaceID: String? = nil,
        wait: TaskWait? = nil,
        workers: [TaskWorker] = []
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
        self.worktreeID = worktreeID
        self.blockedBy = blockedBy
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.workspaceID = workspaceID
        self.wait = wait
        self.workers = workers
    }

    private func clock(at now: Date) -> TaskBoardPresenter.Clock? {
        var fields: [String: Any] = [
            "status": status.rawValue,
            "since": statusSince.timeIntervalSinceReferenceDate,
            "now": now.timeIntervalSinceReferenceDate,
        ]
        fields["created"] = createdAt?.timeIntervalSinceReferenceDate
        fields["updated"] = updatedAt?.timeIntervalSinceReferenceDate
        return TaskBoardPresenter.ask("clock", fields)
    }

    public var lastMoved: Date {
        guard let clock = clock(at: statusSince) else { return statusSince }
        return Date(timeIntervalSinceReferenceDate: clock.lastMoved)
    }

    public func stoppedFor(at now: Date) -> TimeInterval { clock(at: now)?.stoppedFor ?? 0 }
    public func staleness(at now: Date) -> TaskStaleness { stalenessNote(at: now) == nil ? .fresh : .stale }
    public func stalenessNote(at now: Date) -> String? { clock(at: now)?.staleNote }
    public func timeNote(at now: Date) -> String? { clock(at: now)?.timeNote }

    public static func ago(_ interval: TimeInterval) -> String {
        TaskBoardPresenter.ask("age", ["seconds": interval]) ?? ""
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

    public static func holdsBlocks(_ status: TaskStatus) -> Bool {
        TaskBoardPresenter.statusByWire[status.rawValue]?.holdsBlocks == true
    }

    public var callToAction: String? { TaskBoardPresenter.statusByWire[status.rawValue]?.action }

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
public struct UnreadableTaskRow: Decodable, Equatable, Sendable, Hashable, Identifiable {
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
    public static let order = TaskBoardPresenter.statuses.compactMap { TaskStatus(rawValue: $0.wire) }

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

    public var waitingSentence: String? { TaskBoardModel.waitingSentence(waitingOnYou) }

    public static func waitingSentence(_ count: Int) -> String? {
        TaskBoardPresenter.ask("waiting", ["count": count])
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
    ///
    /// A blocker that is done or cancelled is dropped: the edge stays on the
    /// task as history, and "Waiting on ov-36" over a finished ov-36 was the
    /// card telling you it couldn't move when it could (ov-212).
    public func resolvingBlocks(_ raw: [RawTaskBlock]) -> [TaskBlockRef] {
        raw.compactMap { block in
            let blocker = rows.first { $0.id == block.blockedBy }
            if blocker?.status.isFinished == true { return nil }
            return TaskBlockRef(key: blocker?.key ?? block.short, reason: block.reason)
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
extension TaskBoardModel {
    /// Projects the runner's task list into board columns and cards.
    public static func decode(_ data: Data) throws -> TaskBoardModel {
        guard let body = try JSONSerialization.jsonObject(with: data) as? [String: Any],
            let tasks = body["tasks"] as? [Any],
            let view: TaskBoardPresenter.Board = TaskBoardPresenter.ask("board", ["tasks": tasks])
        else {
            throw CocoaError(.coderReadCorrupt)
        }
        return TaskBoardModel(columns: view.columns.compactMap { column in
            guard let status = TaskStatus(rawValue: column.status) else { return nil }
            return TaskBoardColumn(status: status, rows: column.rows.map { $0.render(status: status) })
        }, unreadable: view.unreadable)
    }
}

private struct BoardCard: Decodable {
    let id: String
    let key: String
    let title: String
    let statusSince: Int64
    let intent: String
    let labels: [String]
    let constraints: [String]
    let acceptance: [AcceptanceLine]
    let worktreeId: String?
    let createdAt: Int64?
    let updatedAt: Int64?
    let workspace: String?
    let wait: WireWait?
    let waitingOn: [String]
    let workers: [WireWorker]

    struct AcceptanceLine: Decodable {
        let id: String
        let text: String
        let met: Bool
    }

    func render(status: TaskStatus) -> TaskRow {
        func date(_ ms: Int64) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }
        return TaskRow(
            id: id, key: key, title: title, status: status, statusSince: date(statusSince),
            intent: intent, labels: labels,
            acceptance: acceptance.map { TaskAcceptanceLine(id: $0.id, text: $0.text, met: $0.met) },
            constraints: constraints, worktreeID: worktreeId,
            blockedBy: waitingOn.map { TaskBlockRef(key: $0, reason: "") },
            createdAt: createdAt.map(date), updatedAt: updatedAt.map(date),
            workspaceID: workspace, wait: wait?.wait, workers: workers.map(\.worker))
    }
}

// ---------------------------------------------------------------------------
// The record
//
// Read-only here, and that is not a stage this will grow out of. A note is
// what was understood at a moment; the board renders it and never offers to
// change it.
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
    /// A task's start changed: put in a line, held, parked, or let go
    /// (ov-212). Written only by the runner.
    case wait
    /// A subagent started, ended or resumed on the task (ov-213). Written
    /// only by the runner.
    case worker

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
        case .wait: return "Start"
        case .worker: return "Subagent"
        }
    }

    /// Whether a person wrote this or a transaction did.
    ///
    /// `status_change`, `created`, `wait` and `worker` are written by the store, in the same
    /// transaction that moves or makes a task — the CLI refuses to let anybody
    /// append one. They read as history rather than as somebody's word, and
    /// the board draws them quieter for it.
    public var isMachineWritten: Bool {
        self == .statusChange || self == .created || self == .wait || self == .worker
    }
}

/// One entry in the record, as the board draws it.
public struct TaskNoteRow: Equatable, Sendable, Hashable, Identifiable {
    public var id: String
    public var kind: TaskNoteKind
    /// `user`, `manager`, `agent:<uuid>` or `member:<handle>`, verbatim
    /// from the wire.
    public var actor: String
    public var at: Date
    public var body: String
    /// The note this one replaces, when a later understanding replaced an
    /// earlier one. Both stay readable forever; this is what lets the board
    /// draw the line between them.
    public var supersedes: String?
    /// A decision's rejected options, from the wire's `extra.rejected`.
    public var rejected: [String]
    /// Whose card an answer settled when it wasn't the answerer's, from the
    /// wire's `extra.for` (ov-737): a member's name, or "Owner". Nil for
    /// every other note, and for one written before the runner said.
    public var answeredFor: String?
    /// The handle of the person `answeredFor` names; empty for the owner. Kept
    /// so a byline can say "you" when the card was the reader's.
    public var answeredForHandle: String = ""

    public init(
        id: String, kind: TaskNoteKind, actor: String, at: Date, body: String,
        supersedes: String? = nil, rejected: [String] = [], answeredFor: String? = nil,
        answeredForHandle: String = ""
    ) {
        self.answeredFor = answeredFor
        self.answeredForHandle = answeredForHandle
        self.rejected = rejected
        self.id = id
        self.kind = kind
        self.actor = actor
        self.at = at
        self.body = body
        self.supersedes = supersedes
    }

    /// The note's body as the record draws it.
    ///
    /// A status change's body is written by the runner as "moved from
    /// in_progress to needs_decision", wire ids and all, and every record
    /// already written has it. It's read here as "In Progress → Needs
    /// Decision". A body that isn't that shape, or names a status this build
    /// doesn't know, is shown as written.
    public var displayBody: String {
        guard kind == .statusChange else { return body }
        let words = body.split(separator: " ").map(String.init)
        guard words.count == 5, words[0] == "moved", words[1] == "from", words[3] == "to",
            let from = TaskStatus(rawValue: words[2]), let to = TaskStatus(rawValue: words[4])
        else { return body }
        return "\(from.title) → \(to.title)"
    }

    /// Who wrote it, as the runner's owner reads it: `byline(on:)` is what
    /// a screen draws.
    ///
    /// Never the raw word: `agent:0198f2c0-…` is an id in a byline, which is
    /// noise on every row and only ever useful on one. The uuid is still on
    /// `actor` for anything that needs to match against it.
    ///
    /// `manager` is the orchestrator's actor word, and the word a person
    /// reads is "Orchestrator" (spec §1): `manager` stays the actor and the
    /// skill's name, which nobody sees.
    public var byline: String {
        switch actor {
        case "user": return "You"
        case "manager": return "Orchestrator"
        // What the runner did on its own: "Told Agent 2 about the decision".
        case "runner": return "Runner"
        // `unknown`: an agent the runner could not place in a pane (ov-630).
        case "unknown": return "An agent"
        default: return actor.hasPrefix("agent:") ? "An agent" : "Someone else"
        }
    }

    /// `byline`, as the person holding this device reads it, on a runner
    /// whose people are `people` (ov-707): a member's note by their name,
    /// the one Needs You uses, or "You" on their own device; the owner's
    /// `user` note is "Owner" on anyone else's. A member the list doesn't
    /// have (removed, or the list not read yet) is their handle.
    public func byline(on people: RunnerPeople?) -> String {
        let who = writer(on: people)
        guard let answeredFor else { return who }
        // "for You" when the card was the reader's: a member's own handle, or
        // the owner's (no handle) read on the owner's device (ov-737).
        let reader: Bool
        if let people {
            reader = answeredForHandle.isEmpty ? people.you.isEmpty : (people.isMember && people.you == answeredForHandle)
        } else {
            reader = false
        }
        return "\(who) for \(reader ? "You" : answeredFor)"
    }

    private func writer(on people: RunnerPeople?) -> String {
        if let handle = actor.strippingPrefix("member:") {
            if let people, people.isMember, people.you == handle { return "You" }
            return people?.members.first { $0.handle == handle }?.name ?? handle
        }
        if actor == "user", let people, !people.you.isEmpty { return "Owner" }
        return byline
    }
}

extension String {
    fileprivate func strippingPrefix(_ prefix: String) -> String? {
        hasPrefix(prefix) ? String(dropFirst(prefix.count)) : nil
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
    /// `extra.rejected`, when the note carries one; empty otherwise, and for
    /// an `extra` of any other shape.
    public var rejected: [String]
    /// `extra.for`, as a name: whose card an answer settled (ov-737).
    public var answeredFor: String?
    public var answeredForHandle: String = ""

    enum CodingKeys: String, CodingKey {
        case id, kind, actor, at, body, supersedes, extra
    }

    private struct Extra: Decodable {
        var rejected: [String]?
        var whose: Whose?
        struct Whose: Decodable {
            var handle: String?
            var name: String?
        }
        enum CodingKeys: String, CodingKey {
            case rejected
            case whose = "for"
        }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = try c.decode(String.self, forKey: .kind)
        actor = try c.decodeIfPresent(String.self, forKey: .actor) ?? ""
        at = try c.decodeIfPresent(Int64.self, forKey: .at) ?? 0
        body = try c.decodeIfPresent(String.self, forKey: .body) ?? ""
        supersedes = try c.decodeIfPresent(String.self, forKey: .supersedes)
        let extra = try? c.decodeIfPresent(Extra.self, forKey: .extra)
        rejected = extra?.rejected ?? []
        answeredForHandle = extra?.whose?.handle ?? ""
        answeredFor = extra?.whose.map { $0.name.flatMap { $0.isEmpty ? nil : $0 } ?? $0.handle.flatMap { $0.isEmpty ? nil : $0 } ?? "Owner" }
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
                    supersedes: note.supersedes, rejected: note.rejected, answeredFor: note.answeredFor,
                    answeredForHandle: note.answeredForHandle))
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

extension TaskBoardModel {
    /// Each card's last move in ms, by id: what keeps a theme from reading
    /// as quiet while its cards are still moving (the Plan on the Mac and the phones).
    public var activity: [String: Int64] {
        Dictionary(
            rows.map { ($0.id, Int64($0.lastMoved.timeIntervalSince1970 * 1000)) }, uniquingKeysWith: { a, _ in a })
    }

    /// Each task's status by id: what the Plan view asks whether a lane
    /// waits on the owner.
    public var statuses: [String: TaskStatus] {
        Dictionary(rows.map { ($0.id, $0.status) }, uniquingKeysWith: { a, _ in a })
    }
}
