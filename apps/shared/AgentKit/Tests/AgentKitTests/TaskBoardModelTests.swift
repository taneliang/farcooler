import Foundation
import Testing

@testable import AgentKit

// The board's rules, which are the only part of it a test can reach.
//
// Run by `swift test --package-path apps/shared/AgentKit`, on every push. The
// Mac view that draws these is verified by looking at it; everything below is
// what could be wrong while the drawing looks right — a task that stopped
// moving three days ago rendered identically to one that moved a minute ago,
// a status word from a newer runner quietly filed under the backlog, or a
// menu item that offers to edit a note the store will refuse to let it edit.

/// A card with only the fields a given test is about.
///
/// A fixture in the suite rather than on `TaskRow`, so nothing that ships can
/// reach it. Defaults are the boring case: filed just now, blocking on
/// nothing.
extension TaskRow {
    static func fixture(
        key: String = "fc-1",
        title: String = "A task",
        status: TaskStatus = .todo,
        statusSince: Date = .now,
        blockedBy: [TaskBlockRef] = []
    ) -> TaskRow {
        TaskRow(
            id: "0198f2c0-0000-7000-8000-00000000000\(abs(key.hashValue) % 10)",
            key: key,
            title: title,
            status: status,
            statusSince: statusSince,
            blockedBy: blockedBy)
    }
}

/// The board's whole job beyond showing state.
///
/// The failure mode of the factory is not an agent doing the wrong thing. It
/// is a task sitting in `todo` that you assumed was in flight, and a board
/// that renders it identically to one that moved a minute ago is what lets
/// that happen.
@Test func aTaskThatHasNotMovedReadsAsStale() {
    let fresh = TaskRow.fixture(status: .todo, statusSince: .now)
    let old = TaskRow.fixture(status: .todo, statusSince: .now.addingTimeInterval(-3 * 86_400))
    #expect(fresh.staleness == .fresh)
    #expect(old.staleness == .stale)
}

@Test func aFinishedTaskIsNeverStale() {
    let shipped = TaskRow.fixture(status: .done, statusSince: .now.addingTimeInterval(-90 * 86_400))
    #expect(shipped.staleness == .fresh, "done is finished, not forgotten")
    let dropped = TaskRow.fixture(
        status: .cancelled, statusSince: .now.addingTimeInterval(-90 * 86_400))
    #expect(dropped.staleness == .fresh, "cancelled work is not work that stopped moving")
}

/// The threshold is a threshold, not a rounding.
///
/// Both sides of it, because a comparison written the other way round passes
/// the three-day case above and gets the boundary exactly wrong — and the
/// boundary is where a real board spends its time.
@Test func stalenessTurnsOverExactlyAtTheNamedThreshold() {
    let now = Date()
    let justUnder = TaskRow.fixture(
        statusSince: now.addingTimeInterval(-TaskRow.staleAfter + 60))
    let atIt = TaskRow.fixture(statusSince: now.addingTimeInterval(-TaskRow.staleAfter))
    #expect(justUnder.staleness(at: now) == .fresh)
    #expect(atIt.staleness(at: now) == .stale)
}

/// A runner whose clock runs ahead does not produce a task that moved in the
/// future.
@Test func aStatusStampFromTheFutureReadsAsJustNow() {
    let now = Date()
    let ahead = TaskRow.fixture(statusSince: now.addingTimeInterval(3600))
    #expect(ahead.stoppedFor(at: now) == 0)
    #expect(ahead.staleness(at: now) == .fresh)
}

/// The sentence under a stale row says how long, because that is what makes
/// somebody act.
@Test func aStaleRowSaysHowLongItHasSatThere() {
    let now = Date()
    #expect(TaskRow.fixture(statusSince: now).stalenessNote(at: now) == nil)
    #expect(
        TaskRow.fixture(statusSince: now.addingTimeInterval(-30 * 3600))
            .stalenessNote(at: now) == "Hasn’t moved in a day")
    #expect(
        TaskRow.fixture(statusSince: now.addingTimeInterval(-3 * 86_400))
            .stalenessNote(at: now) == "Hasn’t moved in 3 days")
    #expect(
        TaskRow.fixture(status: .done, statusSince: now.addingTimeInterval(-3 * 86_400))
            .stalenessNote(at: now) == nil,
        "a finished task was given a sentence about having stopped")
}

@Test func aBlockedTaskSaysWhatItIsWaitingOn() {
    let row = TaskRow.fixture(
        status: .todo, blockedBy: [.init(key: "fc-3", reason: "needs the migration")])
    #expect(row.blockedSummary == "Waiting on fc-3")
}

@Test func aTaskWaitingOnSeveralNamesThemAll() {
    #expect(
        TaskRow.fixture().blockedSummary == nil, "a task waiting on nothing said it was waiting")
    #expect(
        TaskRow.fixture(blockedBy: [.init(key: "fc-3", reason: ""), .init(key: "fc-4", reason: "")])
            .blockedSummary == "Waiting on fc-3 and fc-4")
    #expect(
        TaskRow.fixture(blockedBy: [
            .init(key: "fc-3", reason: ""), .init(key: "fc-4", reason: ""),
            .init(key: "fc-9", reason: ""),
        ]).blockedSummary == "Waiting on fc-3, fc-4 and fc-9")
}

/// Copy rules: a sentence, in this app's own voice.
@Test func needsDecisionReadsAsSomethingToDoRatherThanAStatusName() {
    #expect(TaskStatus.needsDecision.title == "Needs Decision")
    #expect(TaskRow.fixture(status: .needsDecision).callToAction == "Answer to unblock this")
}

/// Only the state that is waiting on the person gets one.
@Test func noOtherColumnAsksThePersonForAnything() {
    for status in TaskStatus.allCases where status != .needsDecision {
        #expect(
            TaskRow.fixture(status: status).callToAction == nil,
            """
            \(status.rawValue) asked the user to do something, which makes the column that \
            really is waiting on them ordinary
            """)
    }
}

/// Every status word this app matches is one the protocol declares.
///
/// The drift that would break the board silently and completely: a raw value
/// that does not match what `status_word` prints in crates/cli/src/tasks.rs
/// makes every row of that status unreadable, and the board would show an
/// "unreadable" pile instead of a column. Read out of the proto itself, which
/// is the source both sides are generated from.
@Test func everyStatusWordNamesAStateTheProtocolDeclares() throws {
    var root = URL(fileURLWithPath: #filePath)
    // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
    for _ in 0..<6 { root.deleteLastPathComponent() }
    // Loudly rather than by skipping: a guard that quietly passes when it
    // cannot find what it guards is this repo's own defining failure mode.
    let text = try String(
        contentsOf: root.appendingPathComponent("proto/farcooler.proto"), encoding: .utf8)

    for status in TaskStatus.allCases {
        let declared = "TASK_STATUS_" + status.rawValue.uppercased()
        #expect(
            text.contains(declared),
            "\(status.rawValue) is not a status the protocol declares (\(declared))")
    }
    // And the other direction, which is the half that catches a status the
    // runner gained and this app never grew a column for.
    for line in text.split(separator: "\n") {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard trimmed.hasPrefix("TASK_STATUS_"), !trimmed.hasPrefix("TASK_STATUS_UNSPECIFIED")
        else { continue }
        let word = trimmed.prefix { $0 != " " && $0 != "=" }
            .replacingOccurrences(of: "TASK_STATUS_", with: "").lowercased()
        #expect(
            TaskStatus(rawValue: word) != nil,
            "the protocol declares \(word) and this board has no column for it")
    }
}

// ---------------------------------------------------------------------------
// The board, assembled
// ---------------------------------------------------------------------------

/// The exact bytes `farcooler task list --json` prints, keys and all.
///
/// Written out rather than round-tripped through an encoder this suite also
/// owns: a fixture minted by the code it checks agrees with any value at all,
/// including the wrong one. The other end of this is `render_list_json` and
/// `task_json` in crates/cli/src/tasks.rs.
private let realBoardJSON = """
    {"tasks":[
      {"id":"0198f2c0-0000-7000-8000-000000000001","short":"00000001",
       "repository_id":"0198f2c0-0000-7000-8000-0000000000ff","resource_version":3,
       "key":"fc-1","title":"Wire the board","status":"in_progress",
       "status_since":1757260800000,"stale_for_seconds":120,"intent":"Make it move",
       "acceptance":[{"id":"0198f2c0-0000-7000-8000-00000000000a","text":"It moves","met":false}],
       "constraints":["No new migrations"],"labels":["board"],
       "workspace_id":"0198f2c0-0000-7000-8000-0000000000ee"},
      {"id":"0198f2c0-0000-7000-8000-000000000002","short":"00000002",
       "repository_id":"0198f2c0-0000-7000-8000-0000000000ff","resource_version":1,
       "key":"fc-2","title":"Decide the threshold","status":"needs_decision",
       "status_since":1757260800000,"stale_for_seconds":120,"intent":"",
       "acceptance":[],"constraints":[],"labels":[],"workspace_id":null}
    ]}
    """

@Test func theBoardDecodesWhatTheRunnerActuallyPrints() throws {
    let board = try TaskBoardModel.decode(Data(realBoardJSON.utf8))
    #expect(board.unreadable.isEmpty)
    #expect(board.rows.count == 2)
    #expect(board.waitingOnYou == 1)

    let doing = try #require(board.columns.first { $0.status == .inProgress }?.rows.first)
    #expect(doing.key == "fc-1")
    #expect(doing.title == "Wire the board")
    #expect(doing.labels == ["board"])
    #expect(doing.constraints == ["No new migrations"])
    #expect(doing.acceptance.map(\.text) == ["It moves"])
    #expect(doing.workspaceID == "0198f2c0-0000-7000-8000-0000000000ee")
    // Milliseconds on the wire, seconds in Foundation. Getting this wrong by a
    // factor of a thousand puts every task in 1970 and marks the whole board
    // stale, which looks like a working feature.
    #expect(doing.statusSince == Date(timeIntervalSince1970: 1_757_260_800))

    let asking = try #require(board.columns.first { $0.status == .needsDecision }?.rows.first)
    #expect(asking.workspaceID == nil, "a task with no lane was given one")
}

/// A field this app has not heard of does not cost the board.
///
/// The rule every model in this tree that meets a runner keeps: a synthesized
/// decoder throws on a missing key, so one field added to the CLI after this
/// app shipped would fail the decode of the ENTIRE board and show "no tasks"
/// for a repository full of them.
@Test func aRowFromAnOlderOrNewerRunnerStillDraws() throws {
    let sparse = """
        {"tasks":[{"id":"a","key":"fc-7","title":"Old runner","status":"todo",
                   "something_new":{"nested":true}}]}
        """
    let board = try TaskBoardModel.decode(Data(sparse.utf8))
    let row = try #require(board.rows.first)
    #expect(row.key == "fc-7")
    #expect(row.intent == "")
    #expect(row.labels.isEmpty)
}

/// A status this build has never heard of is shown, not dropped and not
/// filed under the backlog.
///
/// Synthetic, and it has to be: the protocol declares seven statuses and this
/// app has a column for all seven, so nothing in the real tree can exercise
/// this arm. A guard written only against what exists today stays green right
/// up until the day it needed to speak.
@Test func aStatusThisBuildHasNeverHeardOfIsSurfacedRatherThanGuessedAt() throws {
    let ahead = """
        {"tasks":[{"id":"a","key":"fc-8","title":"From a newer runner","status":"awaiting_launch",
                   "status_since":1757260800000,"intent":"","acceptance":[],
                   "constraints":[],"labels":[]}]}
        """
    let board = try TaskBoardModel.decode(Data(ahead.utf8))
    #expect(board.rows.isEmpty, "an unknown status was filed under a column this app made up")
    let orphan = try #require(board.unreadable.first)
    #expect(orphan.key == "fc-8")
    #expect(
        orphan.status == "awaiting_launch",
        "the word the runner used is the only actionable thing")
}

/// Empty columns stay on the board.
@Test func aBoardWithNothingInItStillHasItsColumns() throws {
    let board = try TaskBoardModel.decode(Data(#"{"tasks":[]}"#.utf8))
    #expect(board.columns.map(\.status) == TaskBoardModel.order)
    #expect(board.waitingOnYou == 0)
}

/// The one sentence a board puts in its own title bar.
///
/// In the model rather than in the view, like every other sentence on this
/// board — it was the one piece of board copy composed inside a `View.body`,
/// ninety lines under a comment in that same file saying not to. A copy rule a
/// file states and then breaks is worse than one it never claimed, because the
/// next reader trusts the comment.
///
/// The plural boundary is the whole reason there is a function here at all, so
/// it is asserted on both sides of 1 and not only at 2.
@Test func theWaitingCountReadsAsASentenceOnBothSidesOfOne() {
    #expect(TaskBoardModel.waitingSentence(0) == nil, "a board with nothing to answer says nothing")
    #expect(TaskBoardModel.waitingSentence(1) == "1 task is waiting on you")
    #expect(TaskBoardModel.waitingSentence(2) == "2 tasks are waiting on you")
    #expect(TaskBoardModel.waitingSentence(17) == "17 tasks are waiting on you")
    // The verb agrees too. A sentence that pluralized the noun and left "is"
    // behind is the half-done version of this, and it reads as broken English
    // in a title bar rather than as a bug anybody files.
    #expect(TaskBoardModel.waitingSentence(1)?.contains(" is ") == true)
    #expect(TaskBoardModel.waitingSentence(2)?.contains(" are ") == true)
}

/// And the board composes it from its own count, so the title bar cannot
/// disagree with the column under it.
@Test func theSentenceCountsTheSameColumnTheBoardDraws() throws {
    let board = try TaskBoardModel.decode(Data(realBoardJSON.utf8))
    #expect(board.waitingOnYou == 1)
    #expect(board.waitingSentence == "1 task is waiting on you")
    let empty = try TaskBoardModel.decode(Data(#"{"tasks":[]}"#.utf8))
    #expect(empty.waitingSentence == nil)
}

/// The column that is waiting on the person leads.
@Test func needsDecisionIsTheFirstColumnOnTheBoard() {
    #expect(TaskBoardModel.order.first == .needsDecision)
    #expect(
        Set(TaskBoardModel.order) == Set(TaskStatus.allCases),
        "a status with no column would be work that vanished from the board")
}

/// A block arrives as an id and has to reach the card as a key.
@Test func aBlockIsResolvedAgainstTheBoardItCameFrom() throws {
    let board = try TaskBoardModel.decode(Data(realBoardJSON.utf8))
    let resolved = board.resolvingBlocks([
        RawTaskBlock(
            blockedBy: "0198f2c0-0000-7000-8000-000000000002", short: "00000002",
            reason: "needs the migration")
    ])
    #expect(resolved == [TaskBlockRef(key: "fc-2", reason: "needs the migration")])
}

/// A blocker the board is not showing still appears, by short id.
///
/// The case that decides whether "Waiting on…" can be trusted: a filtered or
/// newly-created blocker that silently dropped out of the list would leave a
/// card looking ready to move when it is not.
@Test func aBlockerThisBoardDoesNotHoldIsStillNamed() throws {
    let board = try TaskBoardModel.decode(Data(realBoardJSON.utf8))
    let resolved = board.resolvingBlocks([
        RawTaskBlock(blockedBy: "not-on-this-board", short: "deadbeef", reason: "waiting")
    ])
    #expect(resolved == [TaskBlockRef(key: "deadbeef", reason: "waiting")])
}

// ---------------------------------------------------------------------------
// The record stays append-only
// ---------------------------------------------------------------------------

/// Nothing the board offers edits or deletes a note.
///
/// The design's load-bearing idea, guarded where it could be undone: current
/// understanding is mutable and lives on the task row, while the record of how
/// you got there is append-only and lives in typed notes that can be
/// superseded but never edited. `task_notes` has a `BEFORE UPDATE` trigger
/// that refuses unconditionally, so an "Edit Note…" would compile, ship, and
/// fail at runtime in front of a user — there is no earlier place than this to
/// catch it.
@Test func nothingTheBoardOffersRewritesTheRecord() {
    let everyStatus = TaskStatus.allCases.map { TaskRow.fixture(status: $0) }
    let actions = TaskBoardModel.allActions(for: everyStatus)
    #expect(!actions.isEmpty, "a guard over an empty list is a guard over nothing")
    for action in actions {
        #expect(
            !action.rewritesTheRecord,
            """
            “\(action.title)” calls \(action.method ?? "nothing"), which changes a note \
            already written
            """)
    }
    // And every method named is one the protocol has, so the guard is reading
    // real words rather than whatever a typo produced.
    for action in actions where action.method != nil {
        #expect(
            action.method == "task.set_status",
            "\(action.method ?? "") is a method this board did not used to call")
    }
}

/// The guard reaches all the way down.
///
/// Synthetic on purpose. The shipped board has no action that edits a note —
/// which is exactly why the test above cannot prove the rule has any teeth. So
/// the forbidden case is built here, in the shapes it would really arrive in,
/// and the rule is asked about each one.
@Test func theGuardCatchesAnActionThatWouldRewriteANote() {
    for method in ["task.note.update", "task.note.delete", "task.delete_note", "task.note_edit"] {
        #expect(
            BoardAction(title: "Edit Note…", method: method).rewritesTheRecord,
            "\(method) would edit a note and the board would have offered it")
    }
    // The one note method that is allowed, and the shape a correction takes:
    // a NEW note pointing at the old one. Both must stay permitted, or the
    // guard would forbid the very thing the design says to do instead.
    #expect(!BoardAction(title: "Add Note…", method: "task.note").rewritesTheRecord)
    // And a write that has nothing to do with notes is not swept up.
    #expect(!BoardAction(title: "In Review", method: "task.set_status").rewritesTheRecord)
    #expect(!BoardAction(title: "Copy", method: nil).rewritesTheRecord)
}

/// Moving a task offers every column except the one it is in.
@Test func aMoveMenuLeavesOutTheStatusTheTaskIsAlreadyIn() {
    let row = TaskRow.fixture(status: .inProgress)
    let moves = TaskBoardModel.moves(for: row)
    #expect(moves.count == TaskStatus.allCases.count - 1)
    #expect(!moves.contains { $0.status == .inProgress })
    #expect(moves.first?.status == .needsDecision, "menu order follows the board")
    // The status is carried, not looked back up from the label — a menu item
    // matched to a column by its copy is one rename away from moving a task to
    // the wrong place.
    for move in moves {
        #expect(move.action.title == move.status.title)
    }
}

// ---------------------------------------------------------------------------
// The record, read back
// ---------------------------------------------------------------------------

/// Exactly what `farcooler task show --json` prints, keys and all. The other
/// end is `render_show_json` and `note_json` in crates/cli/src/tasks.rs.
private let realDetailJSON = """
    {"task":{"id":"0198f2c0-0000-7000-8000-000000000001","key":"fc-1",
             "title":"Wire the board","status":"in_progress","status_since":1757260800000},
     "notes":[
       {"id":"0198f2c0-0000-7000-8000-0000000000a1","short":"000000a1",
        "task_id":"0198f2c0-0000-7000-8000-000000000001","kind":"created",
        "actor":"user","at":1757260800000,"body":"Wire the board","extra":{},
        "supersedes":null},
       {"id":"0198f2c0-0000-7000-8000-0000000000a2","short":"000000a2",
        "task_id":"0198f2c0-0000-7000-8000-000000000001","kind":"decision",
        "actor":"agent:0198f2c0-0000-7000-8000-0000000000bb","at":1757264400000,
        "body":"A day, not an hour","extra":{"rejected":["an hour"]},
        "supersedes":"0198f2c0-0000-7000-8000-0000000000a1"}
     ],
     "blocks":[{"task_id":"0198f2c0-0000-7000-8000-000000000001",
                "blocked_by":"0198f2c0-0000-7000-8000-000000000002",
                "short":"00000002","reason":"needs the migration"}]}
    """

@Test func theRecordDecodesWithItsBylinesAndItsSupersedingLinks() throws {
    let detail = try TaskDetailModel.decode(Data(realDetailJSON.utf8))
    #expect(detail.unreadableNotes == 0)
    #expect(detail.notes.count == 2)
    #expect(detail.blocks.count == 1)

    let made = detail.notes[0]
    #expect(made.kind == .created)
    #expect(made.kind.isMachineWritten, "a note the store wrote reads as somebody's word")
    #expect(made.byline == "You")
    #expect(made.supersedes == nil)
    #expect(made.at == Date(timeIntervalSince1970: 1_757_260_800))

    let decided = detail.notes[1]
    #expect(decided.kind == .decision)
    #expect(!decided.kind.isMachineWritten)
    // Never the raw word: a uuid in a byline is noise on every row.
    #expect(decided.byline == "An agent")
    #expect(decided.actor == "agent:0198f2c0-0000-7000-8000-0000000000bb", "the id is still there")
    #expect(decided.supersedes == "0198f2c0-0000-7000-8000-0000000000a1")
}

/// A byline for every actor word the wire can carry, and for one it cannot.
@Test func everyActorWordGetsASentenceRatherThanAnID() {
    func byline(_ actor: String) -> String {
        TaskNoteRow(id: "n", kind: .comment, actor: actor, at: .now, body: "").byline
    }
    #expect(byline("user") == "You")
    #expect(byline("manager") == "The manager")
    #expect(byline("agent:0198f2c0-0000-7000-8000-0000000000bb") == "An agent")
    // A word from a runner this app has never met. Something rather than
    // nothing, and never the word itself.
    #expect(byline("robot") == "Someone else")
    // The failure worth guarding is an id in a byline, not the English word
    // "manager" appearing in a sentence about the manager. `agent:<uuid>` is
    // the only actor word carrying one, and it is the one that would put a
    // 36-character identifier on every row of a busy task's record.
    for actor in ["user", "manager", "agent:0198f2c0-0000-7000-8000-0000000000bb", "robot"] {
        #expect(!byline(actor).contains(":"), "a machine word crossed into a byline whole")
        #expect(
            !byline(actor).contains("0198f2c0"), "an identifier reached a byline")
    }
}

/// A note kind this build has no name for is counted, not dropped.
///
/// Synthetic, and it has to be: the protocol declares eight kinds and this app
/// names all eight, so the real tree cannot reach this arm. The record's whole
/// claim is that nothing in it is lost — a reader shown six of eight entries
/// with no sign of the other two is being told a smaller truth than the one
/// the record promises.
@Test func aNoteKindThisBuildHasNeverHeardOfIsCountedRatherThanDropped() throws {
    let ahead = """
        {"task":{},"notes":[
          {"id":"a","kind":"comment","actor":"user","at":0,"body":"fine"},
          {"id":"b","kind":"retrospective","actor":"user","at":0,"body":"from a newer runner"}
        ],"blocks":[]}
        """
    let detail = try TaskDetailModel.decode(Data(ahead.utf8))
    #expect(detail.notes.count == 1)
    #expect(detail.unreadableNotes == 1, "an entry vanished out of an append-only record")
}

/// Every note kind this app matches is one the protocol declares, and the
/// other way round.
@Test func everyNoteKindNamesOneTheProtocolDeclares() throws {
    var root = URL(fileURLWithPath: #filePath)
    for _ in 0..<6 { root.deleteLastPathComponent() }
    let text = try String(
        contentsOf: root.appendingPathComponent("proto/farcooler.proto"), encoding: .utf8)

    for kind in TaskNoteKind.allCases {
        let declared = "TASK_NOTE_KIND_" + kind.rawValue.uppercased()
        #expect(
            text.contains(declared),
            "\(kind.rawValue) is not a kind the protocol declares (\(declared))")
    }
    for line in text.split(separator: "\n") {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard trimmed.hasPrefix("TASK_NOTE_KIND_"),
            !trimmed.hasPrefix("TASK_NOTE_KIND_UNSPECIFIED")
        else { continue }
        let word = trimmed.prefix { $0 != " " && $0 != "=" }
            .replacingOccurrences(of: "TASK_NOTE_KIND_", with: "").lowercased()
        #expect(
            TaskNoteKind(rawValue: word) != nil,
            "the protocol declares \(word) and this board cannot draw it")
    }
}

/// A detail from a runner that sends neither key still decodes.
@Test func aDetailWithNoRecordAndNoBlocksIsNotAFailure() throws {
    let detail = try TaskDetailModel.decode(Data(#"{"task":{}}"#.utf8))
    #expect(detail == .empty)
}
