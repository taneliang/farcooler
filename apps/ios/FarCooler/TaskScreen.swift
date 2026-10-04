import SwiftUI

// One task, pushed from its board, from Needs You, or from a pane's task chip
// (spec §6.1): the card, answerable in place while it waits on a decision,
// then rows for its agent, its changes and its worktree, each of which pushes.
// Back from any of them returns here, which is what the board sheet's "Go to
// Agent" jump could never do.

struct TaskScreen: View {
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace
    let task: String

    /// The task's record, read on arrival and after an answer.
    @State private var record: TaskDetailModel?
    /// The question still waiting, with its options, from the same read.
    @State private var question: TaskQuestion?
    /// The last read of the record didn't come back, so the notes below may be
    /// missing or old (ov-179).
    @State private var recordUnread = false
    /// What its agents spent, read on arrival (ov-195).
    @State private var usage: TaskUsageState = .loading
    /// The answer on its way, by option.
    @State private var sending: String?
    /// Why the last answer didn't land.
    @State private var failure: String?
    @State private var writing = false
    @State private var draft = ""

    @Environment(\.phoneNavigator) private var navigator
    @Environment(\.colorScheme) private var scheme
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        Group {
            if let row {
                list(row)
            } else if connection.boards[place.workspace] == nil {
                ProgressView()
            } else {
                ContentUnavailableView {
                    Label("Not on This Board", systemImage: "checklist")
                } description: {
                    Text("This task isn’t on the board anymore.")
                }
            }
        }
        // The key is in the heading card; saying it here as well put `bil-9`
        // twice, one line apart.
        .navigationTitle("Task")
        .navigationBarTitleDisplayMode(.inline)
        .task { await load() }
        .refreshable { await load() }
        .alert("Answer", isPresented: $writing) {
            TextField("Your answer", text: $draft)
            Button("Cancel", role: .cancel) { draft = "" }
            Button("Send") { answer(draft) }
        } message: {
            Text(question?.body ?? "")
        }
        .accessibilityIdentifier("task-screen")
    }

    private var row: TaskRow? {
        connection.boards[place.workspace]?.rows.first { $0.id == task }
    }

    private func load() async {
        if let summary = connection.workspace(place.workspace) {
            await connection.readBoard(summary)
        }
        let read = await connection.taskRecord(task)
        recordUnread = read == nil
        if let read {
            record = read.detail
            question = read.question
            // Opened and read: its finish no longer keeps it in Done's short
            // list (ov-103).
            if let row { PhoneReads.open(row, latest: read.detail.notes.map(\.at).max(), place: place) }
        }
        await readUsage()
    }

    /// What its agents spent; Try Again reads it once more.
    private func readUsage() async {
        usage = await connection.taskUsage(task)
    }

    private func list(_ row: TaskRow) -> some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: PaneMetrics.tight) {
                    Text(row.key)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(.secondary)
                    Text(row.title)
                        .font(.headline)
                    Text(row.status.title)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    CardStartLines(
                        row: row,
                        speaksOfAgents: TaskAgentLink.speaksOfAgents(
                            connected: connection.isAnswering, build: connection.daemon))
                    CardTimeLines(row: row, timeFont: .caption)
                    if let progress = row.acceptanceProgress {
                        AcceptanceLine(progress: progress)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("task-heading")

                if row.status == .needsDecision, let question {
                    decision(question)
                }
            }

            places(row)

            if !row.intent.isEmpty {
                // Markdown, as the Mac draws it (ov-98): the same
                // `TaskProse` blocks, with the document's spacing.
                Section("Intent") {
                    MarkdownText(text: row.intent, spacing: .document)
                        .accessibilityIdentifier("task-intent")
                }
            }

            if !row.acceptance.isEmpty {
                Section("Acceptance") {
                    ForEach(row.acceptance) { line in
                        let linked = linker.linked(TaskProse.acceptance(line.text, met: line.met))
                        HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.step) {
                            // Monochrome, as on the Mac: met lines ticked
                            // and struck through, quietly.
                            Image(systemName: line.met ? "checkmark.square" : "square")
                                .foregroundStyle(.secondary)
                            Text(linked)
                                .font(.body)
                                .foregroundStyle(line.met ? Color.secondary : Color.primary)
                        }
                        .accessibilityElement(children: .ignore)
                        // What it says, not its markup.
                        .accessibilityLabel(TaskProse.plain(line.text))
                        .accessibilityValue(line.met ? "Met" : "Not met")
                        // Its task links, which the one element hides
                        // (ov-196): "Open ov-190" in the actions rotor.
                        .taskKeyActions(linked, linker: linker)
                    }
                }
            }

            TaskUsageSection(state: usage) {
                usage = .loading
                Task { await readUsage() }
            }

            if recordUnread {
                Section {
                    Text("Couldn’t read this task’s record. Pull down to try again.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            }

            if let notes = record?.notes, !notes.isEmpty {
                Section("Record") {
                    ForEach(TaskNoteFeed.newestFirst(notes)) { note in
                        // A block: the quiet "Kind · byline · time" line,
                        // then the body as Markdown (ov-98).
                        VStack(alignment: .leading, spacing: PaneMetrics.tight) {
                            BoardTick { now in
                                Text(
                                    TaskProse.noteLine(
                                        kind: note.kind.title, byline: note.byline,
                                        ago: TaskRow.ago(now.timeIntervalSince(note.at))))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            // As the Mac draws it: a status change in words,
                            // not wire ids.
                            MarkdownText(text: note.displayBody, spacing: .document)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        // Links in task text open on the web or in mail, or a task key's
        // task, pushed (ov-196), nothing else (`Markdown.opens`).
        .environment(\.openURL, Markdown.openGuard(linker))
        .environment(\.taskKeyLinker, linker)
    }

    /// What "ov-190" in this task's text links to.
    private var linker: TaskKeyLinker { connection.taskKeyLinker(navigator) }

    /// The question it's waiting on, and its answers: the options as
    /// buttons (a menu past three), else Answer… for words of your own.
    @ViewBuilder
    private func decision(_ question: TaskQuestion) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(question.body)
                .font(.subheadline.weight(.medium))
                .foregroundStyle(GlancePalette.amber(scheme))
                .accessibilityIdentifier("task-question")
            // A Read grant sees the question and not the answers (spec §2.5).
            if connection.daemon?.mayAct ?? true {
                answers(question)
            }
            if let failure {
                Text(failure)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
    }

    private func answers(_ question: TaskQuestion) -> some View {
            HStack(spacing: 8) {
                ForEach(question.buttons, id: \.self) { option in
                    Button(option) { answer(option) }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                        .disabled(sending != nil)
                        .accessibilityIdentifier("task-answer-\(option)")
                }
                if !question.overflow.isEmpty {
                    Menu("More") {
                        ForEach(question.overflow, id: \.self) { option in
                            Button(option) { answer(option) }
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                }
                if question.options.isEmpty {
                    Button("Answer…") {
                        draft = ""
                        writing = true
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                    .disabled(sending != nil)
                    .accessibilityIdentifier("task-answer")
                }
                if sending != nil { ProgressView().controlSize(.small) }
            }
    }

    /// Agent, Changes and Worktree: where the task's work is, each pushed
    /// over this screen. Reached through the task's own `worktree_id`, so a
    /// task in review with no live agent still reaches its changes.
    @ViewBuilder
    private func places(_ row: TaskRow) -> some View {
        let agents = connection.boardAgents(for: row)
        // Subagents have no pane of their own; they live in the orchestrator's.
        let orchestrator =
            agents.isEmpty && !row.openWorkers.isEmpty ? connection.orchestratorAgent(for: row) : nil
        let worktree = row.worktreeID.flatMap { id in
            connection.fleet.worktrees.first { $0.id == id }
        }
        if !agents.isEmpty || orchestrator != nil || worktree != nil {
            Section {
                if let orchestrator {
                    link(
                        "Orchestrator", systemImage: "sparkle", detail: "Where its subagents work",
                        id: "task-orchestrator"
                    ) {
                        guard let home = connection.fleet.worktrees.first(where: {
                            $0.terminals.contains { $0.id == orchestrator.id }
                        }) else { return }
                        navigator?.open(
                            .worktree(
                                runner: place.runner, worktree: home.id,
                                landing: .terminal(orchestrator.id)))
                    }
                }
                ForEach(agents) { agent in
                    link(
                        agents.count == 1 ? "Agent" : agent.title, systemImage: "sparkle",
                        detail: agents.count == 1 ? agent.title : nil, id: "task-agent"
                    ) {
                        guard let home = connection.fleet.worktrees.first(where: {
                            $0.terminals.contains { $0.id == agent.id }
                        }) else { return }
                        navigator?.open(
                            .worktree(
                                runner: place.runner, worktree: home.id,
                                landing: .terminal(agent.id)))
                    }
                }
                if let worktree {
                    link("Changes", systemImage: "plusminus", detail: nil, id: "task-changes") {
                        navigator?.open(
                            .worktree(runner: place.runner, worktree: worktree.id, landing: .changes))
                    }
                    link(
                        "Worktree", systemImage: "arrow.triangle.branch", detail: worktree.task,
                        id: "task-worktree"
                    ) {
                        navigator?.open(
                            .worktree(runner: place.runner, worktree: worktree.id, landing: .resume))
                    }
                }
            }
        }
    }

    private func link(
        _ title: String, systemImage: String, detail: String?, id: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            // One line at the ordinary sizes, with a long value cut in the
            // middle rather than pushing the row onto two lines. The value
            // goes under the label only at the accessibility sizes, where
            // there is no room beside it.
            Group {
                if dynamicTypeSize.isAccessibilitySize {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Label(title, systemImage: systemImage)
                                .foregroundStyle(.primary)
                            if let detail {
                                Text(detail)
                                    .font(.subheadline)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        Spacer()
                        chevron
                    }
                } else {
                    HStack {
                        Label(title, systemImage: systemImage)
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                            .fixedSize()
                        Spacer(minLength: 8)
                        if let detail {
                            Text(detail)
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                        chevron
                    }
                }
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(id)
    }

    private var chevron: some View {
        Image(systemName: "chevron.forward")
            .font(.footnote.weight(.semibold))
            .foregroundStyle(.tertiary)
    }

    /// Send an answer note, then read the record again: the question closes
    /// once it's answered, and the task stays in Needs Decision for its
    /// orchestrator to move.
    private func answer(_ text: String) {
        guard sending == nil else { return }
        sending = text
        failure = nil
        Task {
            let refused = await connection.answerDecision(task: task, with: text)
            sending = nil
            failure = refused
            await load()
        }
    }
}
