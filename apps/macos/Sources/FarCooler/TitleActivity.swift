import AgentKit
import SwiftUI

// What's happening in a workspace, as the title bar's activity panel says
// it (ov-214, slice 2): the orchestrator and what it last said, every agent
// at work with what it's doing, what's queued and why (ov-212), the
// subagents the orchestrator recorded (ov-213), anything that failed, and
// what agents spent today (ov-195). Worked out here as a value from what
// the window already holds, so `TitleActivityTests` can pin it; the panel
// draws it and decides nothing.
//
// Read when it's opened, never on a timer: the spend is one `farcooler
// report` call per opening, and nothing here ticks while the panel is shut
// or the window is hidden (ov-229).

/// One row of the panel: a task's agent, a recorded subagent, a queued
/// task or a failed one.
struct ActivityLine: Equatable, Identifiable {
    enum Kind: Equatable {
        /// A terminal at work: open it to go to its pane.
        case agent(terminal: String, worktree: String)
        /// A subagent the orchestrator recorded on a task.
        case subagent(task: String)
        /// A task waiting to start.
        case queued(task: String)
    }

    var id: String
    var title: String
    /// What it's doing, or why it's waiting; nil with nothing to say.
    var detail: String?
    /// Its status mark; nil for a queued task.
    var status: Status?
    var kind: Kind
    /// The task it opens, by its key, when it's one of the board's.
    var taskKey: String?
}

/// What today's spend reads as.
enum ActivitySpend: Equatable {
    case reading
    /// Nothing ended a turn today.
    case nothing
    /// "$3.20 · API-equivalent", and "1.2M tokens".
    case spent(cost: String, tokens: String)
    case needsUpdate
    case couldntRead

    /// What the panel says.
    var words: String {
        switch self {
        case .reading: "Reading…"
        case .nothing: "Nothing spent yet today"
        case .spent(let cost, let tokens): "\(cost) · \(tokens)"
        case .needsUpdate: TaskUsageFormat.needsUpdate
        case .couldntRead: "Far Cooler couldn’t read today’s spend."
        }
    }

    /// Today's spend from `farcooler report --json`'s output, or from the
    /// message a refused read came back with.
    static func read(data: Data?, message: String?) -> ActivitySpend {
        guard let data else {
            return (message ?? "").contains("older than reports") ? .needsUpdate : .couldntRead
        }
        struct Report: Decodable {
            struct Spend: Decodable { var total: TaskSpend }
            var spend: Spend?
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        guard let report = try? decoder.decode(Report.self, from: data) else { return .couldntRead }
        guard let total = report.spend?.total, !total.isEmpty else { return .nothing }
        return .spent(cost: TaskUsageFormat.cost(total), tokens: TaskUsageFormat.tokensLine(total))
    }
}

/// The panel's content.
struct TitleActivity: Equatable {
    struct Orchestrator: Equatable {
        var state: OrchestratorRow.State
        var status: Status?
        var nowDoing: String?
        /// The last thing it said, when that isn't already `nowDoing`.
        var lastSaid: String?
        /// The subagents it's running now, by name (the runner's `subagents`).
        var subagents: [String]
    }

    var orchestrator: Orchestrator?
    var working: [ActivityLine]
    var queued: [ActivityLine]
    var failed: [ActivityLine]

    var isQuiet: Bool { working.isEmpty && queued.isEmpty && failed.isEmpty }

    /// The statuses that count as at work, and as failed.
    static let atWork: [Status] = [.working, .blocked, .starting]
    static let failing: [Status] = [.failedRun, .failedTurn, .failed, .lost]

    /// The panel's content from what the window holds: the orchestrator's
    /// seat and state, the workspace's other agent panes, its board and its
    /// tasks' starts.
    static func make(
        orchestrator state: OrchestratorRow.State?, seat: Terminal?, panes: [BoardPane], board: TaskBoardModel,
        starts: [String: TaskStart], now: Date = Date()
    ) -> TitleActivity {
        let byID = Dictionary(board.rows.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        func line(_ pane: BoardPane) -> ActivityLine {
            let row = pane.terminal.taskId.flatMap { byID[$0] }
            let status = pane.terminal.status
            let doing = OrchestratorRow.nowDoing(
                pane.terminal, state: status == .blocked ? .needsYou : .working, now: now)
            return ActivityLine(
                id: pane.terminal.id, title: row.map { "\($0.key) \($0.title)" } ?? pane.terminal.label,
                detail: failing.contains(status) ? status.label : doing, status: status,
                kind: .agent(terminal: pane.terminal.id, worktree: pane.worktree.id), taskKey: row?.key)
        }
        let agents = panes.filter { $0.terminal.runsAgent && !$0.terminal.isOrchestrator }
        var working = agents.filter { atWork.contains($0.terminal.status) }.map(line)
        let failed = agents.filter { failing.contains($0.terminal.status) }.map(line)

        // Recorded subagents still at work, after the panes: lane B doesn't
        // observe them yet, so most say when they started, not what they do.
        let keys = starts.keys.sorted()
        for key in keys {
            guard let start = starts[key] else { continue }
            for worker in start.workers where worker.isActive {
                let detail: String? = {
                    if !worker.doing.isEmpty { return worker.doing }
                    guard let ms = worker.startedAt else { return nil }
                    let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
                    return "Started at \(date.formatted(date: .omitted, time: .shortened))"
                }()
                let name = worker.label.isEmpty ? [worker.harness, "subagent"].filter { !$0.isEmpty }.joined(separator: " ") : worker.label
                working.append(
                    ActivityLine(
                        id: "worker:\(worker.id)", title: "\(key) · \(name)", detail: detail,
                        status: worker.state == "running" ? .working : .running, kind: .subagent(task: key),
                        taskKey: key))
            }
        }

        let queued = keys.compactMap { key -> ActivityLine? in
            guard let start = starts[key], start.isQueued else { return nil }
            return ActivityLine(
                id: "queued:\(key)", title: "\(key) \(start.title)", detail: TaskStarts.why(start, now: now),
                status: nil, kind: .queued(task: key), taskKey: key)
        }

        let orchestrator = state.map { state -> Orchestrator in
            let doing = OrchestratorRow.nowDoing(seat, state: state, now: now)
            let said = seat?.lastSaid?.trimmingCharacters(in: .whitespacesAndNewlines)
            return Orchestrator(
                state: state, status: seat?.status, nowDoing: doing,
                lastSaid: said.flatMap { $0.isEmpty || $0 == doing ? nil : $0 },
                subagents: seat?.subagents ?? [])
        }
        return TitleActivity(orchestrator: orchestrator, working: working, queued: queued, failed: failed)
    }

    /// How many are queued, for the status area's count.
    static func queuedCount(_ starts: [String: TaskStart]) -> Int { starts.values.filter(\.isQueued).count }
}

/// The activity panel: the popover under the status area's activity line,
/// and, with slice 4, what the title bar's field shows while it's empty.
struct TitleActivityPanel: View {
    let activity: TitleActivity
    let spend: ActivitySpend
    /// Open a row: its task, or its pane.
    let onOpen: (ActivityLine) -> Void
    var onOrchestrator: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let orchestrator = activity.orchestrator { orchestratorBlock(orchestrator) }
            section("Working", activity.working, empty: activity.orchestrator == nil ? nil : "No agents at work")
            section("Queued", activity.queued)
            section("Failed", activity.failed)
            VStack(alignment: .leading, spacing: 2) {
                Text("Spent Today").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                Text(spend.words)
                    .font(.callout)
                    .foregroundStyle(spend == .reading ? .secondary : .primary)
                    .accessibilityIdentifier("activity-spend")
                if case .spent = spend {
                    Text("API-equivalent, across this repository").font(.caption).foregroundStyle(.tertiary)
                }
            }
        }
        .padding(Spacing.inset)
        .frame(width: 360, alignment: .leading)
    }

    private func orchestratorBlock(_ o: TitleActivity.Orchestrator) -> some View {
        Button(action: onOrchestrator) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                OrchestratorMark(state: o.state, status: o.status).frame(width: 12)
                VStack(alignment: .leading, spacing: 2) {
                    Text(o.state == .none ? OrchestratorRow.word(.none) : "Orchestrator · \(OrchestratorRow.word(o.state))")
                        .font(.callout.weight(.semibold))
                    if let doing = o.nowDoing { Text(doing).font(.callout).lineLimit(2) }
                    if let said = o.lastSaid {
                        Text(said).font(.caption).foregroundStyle(.secondary).lineLimit(3)
                    }
                    if !o.subagents.isEmpty {
                        Text("Subagents: " + o.subagents.joined(separator: ", "))
                            .font(.caption).foregroundStyle(.secondary).lineLimit(2)
                    }
                }
                Spacer(minLength: 0)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(TitleStatus.orchestratorLabel(
            TitleStatus.Model(orchestrator: o.state, status: o.status, nowDoing: o.nowDoing, needYou: 0, running: [], inReview: []))
            ?? "Orchestrator")
        .accessibilityHint("Shows the orchestrator")
        .accessibilityIdentifier("activity-orchestrator")
    }

    @ViewBuilder
    private func section(_ title: String, _ lines: [ActivityLine], empty: String? = nil) -> some View {
        if !lines.isEmpty || empty != nil {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    .padding(.top, Spacing.tight)
                if lines.isEmpty, let empty {
                    Text(empty).font(.callout).foregroundStyle(.secondary)
                }
                ForEach(lines) { line in row(line) }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(title)
        }
    }

    private func row(_ line: ActivityLine) -> some View {
        Button { onOpen(line) } label: {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                Group {
                    if let status = line.status { StatusGlyph(status: status) } else { Image(systemName: "clock").font(.caption) }
                }
                .frame(width: 12)
                VStack(alignment: .leading, spacing: 1) {
                    Text(line.title).font(.callout).lineLimit(1).truncationMode(.tail)
                    if let detail = line.detail {
                        Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.tail)
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 3)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(line.detail ?? line.title)
        .probed("activity-row")
        .accessibilityLabel([line.title, line.detail].compactMap { $0 }.joined(separator: ", "))
    }
}
