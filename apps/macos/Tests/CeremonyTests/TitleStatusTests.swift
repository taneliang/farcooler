import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The title bar's status area as values (ov-214): which form a width
/// affords, what it says for each state of the orchestrator, and that its
/// counts are the board's.
@MainActor
struct TitleStatusTests {
    // MARK: - Forms

    @Test("Each form is chosen from the width it needs, the ring below them all")
    func formsByWidth() {
        for form in TitleStatus.Form.allCases {
            #expect(TitleStatus.form(available: form.width) == form, "\(form) at exactly its width")
            if form > .ring {
                let narrower = TitleStatus.Form(rawValue: form.rawValue - 1)!
                #expect(TitleStatus.form(available: form.width - 1) == narrower, "\(form) a point short")
            }
        }
        #expect(TitleStatus.form(available: 0) == .ring)
        #expect(TitleStatus.form(available: -200) == .ring)
        #expect(TitleStatus.form(available: 5000) == .wide)
        // Widest last, so a wider window never says less.
        let widths = TitleStatus.Form.allCases.map(\.width)
        #expect(widths == widths.sorted() && Set(widths).count == widths.count)
    }

    @Test("A longer switcher label or more trailing items leave less room")
    func roomShrinks() {
        let short = TitleStatus.leading(switcher: "Main", repository: "")
        let long = TitleStatus.leading(switcher: "Billing reconciliation", repository: "shop-frontend")
        #expect(long > short)
        let bare = TitleStatus.trailing(editor: false, changes: false, trouble: nil)
        let full = TitleStatus.trailing(editor: true, changes: true, trouble: "carl offline", needsYou: 11)
        #expect(full > bare)
        #expect(TitleStatus.trailing(editor: false, changes: false, trouble: nil, needsYou: 11) > bare)
        #expect(
            TitleStatus.available(window: 900, leading: long, trailing: full)
                < TitleStatus.available(window: 900, leading: short, trailing: bare))
    }

    // MARK: - What it says

    private static func model(
        _ state: OrchestratorRow.State?, doing: String? = nil, needYou: Int = 0
    ) -> TitleStatus.Model {
        TitleStatus.Model(
            orchestrator: state, status: nil, nowDoing: doing, needYou: needYou, running: [], inReview: [])
    }

    @Test("The orchestrator's line and label for each state")
    func orchestratorWords() {
        #expect(TitleStatus.orchestratorLine(Self.model(.working, doing: "Reading the diff")) == "Working — Reading the diff")
        #expect(TitleStatus.orchestratorLine(Self.model(.idle)) == "Idle")
        #expect(TitleStatus.orchestratorLine(Self.model(.none)) == "No Orchestrator")
        #expect(TitleStatus.orchestratorLine(Self.model(.needsYou, doing: "Ship it?")) == "Needs You — Ship it?")
        #expect(TitleStatus.orchestratorLine(Self.model(.unread)) == "Done")
        #expect(TitleStatus.orchestratorLine(Self.model(.starting)) == "Starting")
        #expect(TitleStatus.orchestratorLine(Self.model(.stopped)) == "Stopped")
        // No conversation column, no orchestrator part.
        #expect(TitleStatus.orchestratorLine(Self.model(nil)) == nil)

        #expect(
            TitleStatus.orchestratorLabel(Self.model(.working, doing: "Reading the diff"))
                == "Orchestrator, Working, Reading the diff")
        #expect(TitleStatus.orchestratorLabel(Self.model(.idle)) == "Orchestrator, Idle")
        #expect(TitleStatus.orchestratorLabel(Self.model(.none)) == "No Orchestrator")
        #expect(TitleStatus.orchestratorLabel(Self.model(nil)) == nil)
    }

    @Test("Counts in words at the wide form, nothing at zero, 99+ past 99")
    func countWords() {
        #expect(TitleStatus.needYouWords(0) == nil)
        #expect(TitleStatus.needYouWords(1) == "1 need you")
        #expect(TitleStatus.needYouWords(3) == "3 need you")
        #expect(TitleStatus.runningWords(0) == nil)
        #expect(TitleStatus.runningWords(2) == "2 running")
        #expect(TitleStatus.inReviewWords(1) == "1 in review")
        #expect(TitleStatus.needYouWords(100) == "99+ need you")
        #expect(TitleStatus.number(99) == "99")
        #expect(TitleStatus.number(100) == "99+")
        #expect(TitleStatus.runningLabel(1) == "1 task running")
        #expect(TitleStatus.runningLabel(4) == "4 tasks running")
        #expect(TitleStatus.inReviewLabel(1) == "1 task in review")
        #expect(TitleStatus.needYouLabel(0) == "Nothing needs you")
        #expect(TitleStatus.needYouLabel(2) == "2 need you")
    }

    @Test("Only a need-you count above zero wears the attention color")
    func onlyNeedYouIsTinted() {
        #expect(!TitleStatus.needYouIsTinted(0))
        #expect(TitleStatus.needYouIsTinted(1))
    }

    // MARK: - The board's counts

    /// A store read through a stubbed CLI, with tasks in every status.
    private static func store() async -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let now = Int64(Date().timeIntervalSince1970 * 1000) - 60_000
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            let tasks = [
                ("t1", "ov-1", "Ship the relay", "in_progress"),
                ("t2", "ov-2", "Polish the sidebar", "in_progress"),
                ("t3", "ov-3", "Title bar", "in_review"),
                ("t4", "ov-4", "Pick a name", "needs_decision"),
                ("t5", "ov-5", "Pick a color", "needs_decision"),
                ("t6", "ov-6", "Coordinator", "todo"),
                ("t7", "ov-7", "Old thing", "done"),
                ("t8", "ov-8", "Someday", "backlog"),
            ].map { id, key, title, status in
                #"{"id":"\#(id)","key":"\#(key)","title":"\#(title)","status":"\#(status)","status_since":\#(now),"created_at":\#(now),"updated_at":\#(now)}"#
            }
            return (Data(#"{"tasks":[\#(tasks.joined(separator: ","))]}"#.utf8), nil)
        }
        let store = TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
        await store.readIfNeverRead()
        return store
    }

    @Test("Running and in review are the board's In Progress and In Review tasks; need you is its waiting count")
    func countsAreTheBoards() async {
        let store = await Self.store()
        #expect(store.board.rows.count == 8)
        let source = TitleStatusSource(orchestrator: .working, status: .working, nowDoing: nil, board: store)
        let model = TitleStatus.model(source, board: store.board)
        #expect(model.running.map(\.key).sorted() == ["ov-1", "ov-2"])
        #expect(model.inReview.map(\.key) == ["ov-3"])
        // The board's own waiting count, passed through the window's rule.
        #expect(model.needYou == store.board.waitingOnYou)
        #expect(model.needYou == 2)
        let fromList = TitleStatusSource(
            orchestrator: nil, status: nil, nowDoing: nil, board: store, waiting: { column in column + 5 })
        #expect(TitleStatus.model(fromList, board: store.board).needYou == 7)
        // Each count is its own column's, not the other's.
        let counts = TitleStatus.counts(store.board)
        #expect(counts.running.allSatisfy { $0.status == .inProgress })
        #expect(counts.inReview.allSatisfy { $0.status == .inReview })
        #expect(TitleStatus.counts(.empty).running.isEmpty)
    }
}
