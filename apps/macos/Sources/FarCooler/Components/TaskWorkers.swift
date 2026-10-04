import AgentKit
import SwiftUI

/// The subagents working a task, as the Mac says them (ov-213).
///
/// The words are AgentKit's, the phones' too: `TaskRow.startLine` composes the
/// sentence and `TaskRow.agentPresence` the control's name, against one shared
/// fixture. This file only decides where the Mac points the control and
/// lays the task view's section out; it writes no sentence of its own.
enum TaskWorkers {
    /// The pane the subagents on `row` live in, for the Subagent control to
    /// open: the orchestrator's, on the runner's own worktrees. Nil when the
    /// runner didn't say which pane, or it has closed or stopped since, so the
    /// control is plain text rather than a button that leads nowhere.
    static func orchestrator(of row: TaskRow, in worktrees: [Worktree]) -> BoardPane? {
        guard let id = row.orchestratorTerminalID else { return nil }
        for worktree in worktrees {
            guard let terminal = worktree.terminals.first(where: { $0.id == id }),
                TaskAgentLink.liveStates.contains(terminal.boardState)
            else { continue }
            return BoardPane(terminal: terminal, worktree: worktree)
        }
        return nil
    }

    /// `opus` as `Opus`: the model is a name, shown in the task view only.
    static func modelName(_ model: String) -> String {
        model.prefix(1).uppercased() + model.dropFirst()
    }

    /// One sentence per subagent, the ones still at it first, then the most
    /// recently closed. Each is `startLine`'s own worker sentence for that one
    /// worker, so "Claude subagent working, 12 min · Running cargo test" is
    /// said the same here as on a phone's card.
    static func lines(of row: TaskRow, at now: Date) -> [String] {
        let ordered = row.workers.sorted { a, b in
            if a.state.isOpen != b.state.isOpen { return a.state.isOpen }
            return (a.endedAt ?? a.startedAt ?? .distantPast) > (b.endedAt ?? b.startedAt ?? .distantPast)
        }
        return ordered.compactMap { worker in
            var alone = row
            alone.status = .inProgress
            alone.wait = nil
            alone.workers = [worker]
            guard let line = alone.startLine(at: now) else { return nil }
            return worker.model.isEmpty ? line : "\(line) · \(modelName(worker.model))"
        }
    }
}

/// The task view's Subagents section: what each is doing, and the way to the
/// orchestrator's pane. Quiet text; the one control is the button.
struct TaskWorkersSection: View {
    let row: TaskRow
    var orchestrator: BoardPane?
    var onGoTo: (BoardPane) -> Void = { _ in }

    var body: some View {
        if !row.workers.isEmpty {
            VStack(alignment: .leading, spacing: TaskTypography.labelGap) {
                Text(row.workers.count == 1 ? "Subagent" : "Subagents")
                    .font(TaskTypography.label)
                    .foregroundStyle(.secondary)
                    .accessibilityAddTraits(.isHeader)
                BoardTick { now in
                    VStack(alignment: .leading, spacing: ColumnGrid.rhythm / 2) {
                        ForEach(Array(TaskWorkers.lines(of: row, at: now).enumerated()), id: \.offset) { _, line in
                            Text(line)
                                .font(TaskTypography.body)
                                .fixedSize(horizontal: false, vertical: true)
                                .textSelection(.enabled)
                        }
                    }
                    .accessibilityIdentifier("task-workers")
                }
                if let orchestrator {
                    Button("Go to Orchestrator") { onGoTo(orchestrator) }
                        .help(orchestrator.title)
                        .accessibilityIdentifier("task-go-to-orchestrator")
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
