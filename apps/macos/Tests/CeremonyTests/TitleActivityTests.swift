import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's activity panel (ov-214, slice 2): what's queued and why
/// (ov-212), the subagents the orchestrator recorded (ov-213), every agent at
/// work or failed, and today's spend (ov-195), worked out from fixtures of
/// what the runner sends.
@MainActor
struct TitleActivityTests {
    // MARK: - Starts, as the board's read carries them

    /// The CLI's own fixture (crates/client/src/testdata/task_starts.json),
    /// as rows of `task list --json`, with each one's status and title.
    static let listJSON = #"""
    {"tasks":[
      {"key":"ov-192","title":"Jump bar","status":"in_progress",
       "wait":{"kind":"in_line","line":"build","position":2,"ahead":["ov-177"],"since":1000},"waiting_on":[],
       "workers":[{"id":"07070707-0707-0707-0707-070707070707","harness":"claude","agent_id":"a3f","label":"ov-192 Mac polish",
         "model":"opus","state":"finished","started_at":2000,"ended_at":9000,"last_activity_at":null,"doing":""}]},
      {"key":"ov-12","title":"Later","status":"todo","wait":{"kind":"until","until":1759654800000,"since":1000},
       "waiting_on":["ov-191","ov-192"],"workers":[]},
      {"key":"ov-122","title":"After release","status":"backlog","wait":{"kind":"after","event":"release","since":1000},
       "waiting_on":[],"workers":[]},
      {"key":"ov-6","title":"Someday","status":"todo","wait":{"kind":"parked","since":1000},"waiting_on":[],"workers":[]},
      {"key":"ov-7","title":"Next up","status":"todo","wait":{"kind":"in_line","line":"agent","position":1,"ahead":[],"since":1000},
       "waiting_on":[],"workers":[]},
      {"key":"ov-213","title":"Subagents","status":"in_progress","wait":null,"waiting_on":[],
       "workers":[{"id":"09090909-0909-0909-0909-090909090909","harness":"codex","agent_id":"/root/lane","label":"",
         "model":"","state":"unobserved","started_at":3000,"ended_at":null,"last_activity_at":null,"doing":""}]},
      {"key":"old-runner","title":"Old","status":"todo"}
    ]}
    """#

    static var starts: [String: TaskStart] { TaskStarts.decode(Data(listJSON.utf8)) }

    @Test("Each row's wait, blockers and workers are read, and a row without them has none")
    func decodesStarts() {
        let starts = Self.starts
        #expect(starts.count == 7)
        #expect(starts["ov-192"]?.wait == .inLine(line: "build", position: 2, ahead: ["ov-177"]))
        #expect(starts["ov-12"]?.wait == .until(1_759_654_800_000))
        #expect(starts["ov-12"]?.waitingOn == ["ov-191", "ov-192"])
        #expect(starts["ov-122"]?.wait == .after("release"))
        #expect(starts["ov-6"]?.wait == .parked)
        #expect(starts["ov-213"]?.workers.first?.isActive == true)
        #expect(starts["ov-192"]?.workers.first?.isActive == false)
        #expect(starts["old-runner"]?.wait == nil && starts["old-runner"]?.workers.isEmpty == true)
        #expect(TaskStarts.decode(Data("not json".utf8)).isEmpty)
        #expect(TaskStarts.decode(Data(#"{"tasks":[]}"#.utf8)).isEmpty)
    }

    @Test("Only open tasks not yet started, held by a wait or a blocker, are queued")
    func queued() {
        let starts = Self.starts
        #expect(starts["ov-192"]?.isQueued == false, "in progress isn't queued, whatever its wait says")
        #expect(starts["ov-12"]?.isQueued == true)
        #expect(starts["ov-122"]?.isQueued == true)
        #expect(starts["ov-6"]?.isQueued == true)
        #expect(starts["old-runner"]?.isQueued == false)
        #expect(TitleActivity.queuedCount(starts) == 4)
    }

    @Test("Why a task waits, in a few words, a blocker first")
    func why() {
        let starts = Self.starts
        let clock: (Date, Bool) -> String = { _, today in today ? "3:00 PM" : "Sun 3:00 PM" }
        #expect(TaskStarts.why(starts["ov-12"]!, time: clock) == "Blocked by ov-191 and ov-192")
        #expect(TaskStarts.why(starts["ov-122"]!) == "Starts after the next release")
        #expect(TaskStarts.why(starts["ov-6"]!) == "Parked")
        #expect(TaskStarts.why(starts["ov-7"]!) == "Next in line for an agent")
        #expect(TaskStarts.why(starts["ov-192"]!) == "2nd in line for a build, after ov-177")
        var later = starts["ov-12"]!
        later.waitingOn = []
        #expect(TaskStarts.why(later, now: Date(timeIntervalSince1970: 0), time: clock) == "Starts at Sun 3:00 PM")
        #expect(TaskStarts.why(starts["old-runner"]!) == nil)
        #expect(["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "102nd"]
            == [1, 2, 3, 4, 11, 12, 13, 21, 102].map(TaskStarts.ordinal))
        #expect(TaskStarts.list(["a", "b", "c", "d", "e"]) == "a, b and 3 more")
    }

    // MARK: - The panel's content

    private static let worktree = Worktree(
        id: "w1", short: "w1", task: "lane", branch: "lane", repository: "overnight", host: "",
        path: "/tmp/lane", state: "active", terminals: [])

    private static func pane(
        _ id: String, activity: String? = "working", state: String = "running", exit: Int? = nil,
        role: String = "agent", task: String? = nil, line: String? = nil
    ) -> BoardPane {
        var t = Terminal(id: id, short: id, title: id, preset: "claude", state: state, epoch: 0)
        t.activity = activity
        t.exitCode = exit
        t.role = role
        t.taskId = task
        t.line = line
        return BoardPane(terminal: t, worktree: worktree)
    }

    private static func board() -> TaskBoardModel {
        let row = TaskRow(
            id: "t1", key: "ov-1", title: "Ship the relay", status: .inProgress, statusSince: Date(), intent: "",
            labels: [], acceptance: [], constraints: [])
        return TaskBoardModel(columns: [TaskBoardColumn(status: .inProgress, rows: [row])])
    }

    @Test("The panel lists agents at work, recorded subagents, the queue and failures, never the orchestrator")
    func panelContent() {
        var seat = Terminal(id: "o", short: "o", title: "o", preset: "claude", state: "running", epoch: 0)
        seat.activity = "working"
        seat.role = "orchestrator"
        seat.subagents = ["lane A", "lane B"]
        let panes = [
            Self.pane("a", task: "t1", line: "Running the Mac tests"),
            Self.pane("b", activity: "idle"),
            Self.pane("c", activity: nil, state: "exited", exit: 1),
            Self.pane("o", role: "orchestrator"),
        ]
        let activity = TitleActivity.make(
            orchestrator: .working, seat: seat, panes: panes, board: Self.board(), starts: Self.starts)
        let working = activity.working
        #expect(working.first?.title == "ov-1 Ship the relay", "a task's agent is named by its task")
        #expect(working.first?.detail == "Running the Mac tests")
        #expect(!working.contains { $0.id == "b" }, "an idle agent isn't at work")
        #expect(!working.contains { $0.id == "o" }, "the orchestrator has its own block")
        #expect(working.contains { $0.id == "worker:09090909-0909-0909-0909-090909090909" }, "a recorded subagent")
        #expect(!working.contains { $0.id.hasPrefix("worker:0707") }, "a finished subagent isn't at work")
        #expect(activity.failed.map(\.id) == ["c"])
        #expect(activity.failed.first?.detail == "Failed")
        #expect(activity.queued.map(\.taskKey) == ["ov-12", "ov-122", "ov-6", "ov-7"])
        #expect(activity.orchestrator?.subagents == ["lane A", "lane B"])
        #expect(!activity.isQuiet)
        #expect(TitleActivity.make(orchestrator: nil, seat: nil, panes: [], board: .empty, starts: [:]).isQuiet)
    }

    @Test("The status area counts what the panel lists: queued, and failed only above zero")
    func statusCounts() {
        let source = TitleStatusSource(
            orchestrator: .working, status: .working, nowDoing: nil,
            panes: [Self.pane("c", activity: nil, state: "exited", exit: 1)])
        let model = TitleStatus.model(source, board: Self.board(), starts: Self.starts)
        #expect(model.queued == 4)
        #expect(model.failed.count == 1)
        #expect(TitleStatus.failedWords(0) == nil)
        #expect(TitleStatus.failedWords(1) == "1 failed")
        #expect(TitleStatus.failedLabel(2) == "2 agents failed")
        let quiet = TitleStatus.model(TitleStatusSource(orchestrator: .idle, status: nil, nowDoing: nil), board: .empty)
        #expect(quiet.queued == 0 && quiet.failed.isEmpty)
    }

    // MARK: - Today's spend

    @Test("Today's spend reads the report's total, and says why when it can't")
    func spend() {
        let report = #"{"schema":1,"spend":{"total":{"turns":3,"input_tokens":1000,"output_tokens":200,"cost_reported_micros":3200000}}}"#
        guard case .spent(let cost, let tokens) = ActivitySpend.read(data: Data(report.utf8), message: nil) else {
            Issue.record("a report with spend read as something else")
            return
        }
        #expect(cost.hasPrefix("$3.20"))
        #expect(tokens == "1.2K tokens")
        #expect(ActivitySpend.read(data: Data(#"{"schema":1,"spend":null}"#.utf8), message: nil) == .nothing)
        #expect(ActivitySpend.read(data: Data(#"{"schema":1}"#.utf8), message: nil) == .nothing)
        #expect(
            ActivitySpend.read(data: nil, message: "this runner's Far Cooler is older than reports. update it and try again")
                == .needsUpdate)
        #expect(ActivitySpend.read(data: nil, message: "connection refused") == .couldntRead)
        #expect(ActivitySpend.read(data: Data("garbage".utf8), message: nil) == .couldntRead)
        #expect(!ActivitySpend.couldntRead.words.contains("refused"), "no raw error on screen")
    }

    // MARK: - Drawn

    @Test("The panel draws one row per line it was given")
    func panelDrawsItsRows() async {
        let activity = TitleActivity.make(
            orchestrator: .working, seat: nil, panes: [Self.pane("a", task: "t1")], board: Self.board(),
            starts: Self.starts)
        final class Seen { var rows = 0 }
        let seen = Seen()
        let panel = TitleActivityPanel(activity: activity, spend: .nothing, onOpen: { _ in })
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                let _ = seen.rows = probed.filter { $0.id == "activity-row" }.count
                Color.clear
            }
        let host = NSHostingView(rootView: panel)
        host.frame = NSRect(x: 0, y: 0, width: 360, height: 800)
        for _ in 0..<5 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        #expect(seen.rows == activity.working.count + activity.queued.count + activity.failed.count)
        #expect(seen.rows == 6)
    }
}
