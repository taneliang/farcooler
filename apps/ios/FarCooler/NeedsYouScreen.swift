import SwiftUI

// The phone's front door (spec §6.1): everything on every runner a person has
// to act on, answerable in place, then the Workspaces list.
//
// What an item IS is the daemon's (`needs_you.list`), merged across runners
// by `FleetStore`; what a runner too old to send a list gets is
// `NeedsYou.derived`. This screen draws the items, sends their answers, and
// lists the workspaces, whose sections are `Fleet.phoneSections`.

struct NeedsYouScreen: View {
    @ObservedObject var fleet: FleetStore
    @ObservedObject var hosts: RunnerStore
    /// Replace the stack over this screen.
    let open: ([PhoneRoute]) -> Void

    @State private var editingRunner: Runner?
    @State private var authorizing = false
    @State private var showSettings = false
    @State private var showAdd = false
    /// The Unclaimed and Hidden groups open right now, by runner and title.
    @State private var openGroups: Set<String> = []
    /// The mark column, which grows with the text so the mark never touches the name.
    @ScaledMetric private var markWidth: CGFloat = 14

    var body: some View {
        List {
            trouble
            items
            workspaces
        }
        .listStyle(.insetGrouped)
        .navigationTitle("Needs You")
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button {
                    showSettings = true
                } label: {
                    Image(systemName: "gearshape")
                }
                .accessibilityLabel("Settings")
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showAdd = true
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityLabel("Add Runner")
            }
        }
        .refreshable {
            await fleet.refreshAll()
            for connection in fleet.active { await connection.loadNeedsYou() }
        }
        .sheet(item: $editingRunner) { runner in
            HostEditorView(
                existing: runner,
                onSave: { hosts.update($0) },
                onRemove: { hosts.remove($0) })
        }
        .sheet(isPresented: $authorizing) {
            NavigationStack { AuthorizeView(runners: hosts) }
        }
        .sheet(isPresented: $showSettings) {
            NavigationStack {
                SettingsView(connection: fleet.runners.first?.connection, runners: hosts)
            }
        }
        .sheet(isPresented: $showAdd) { AddView(runners: hosts) }
        .accessibilityIdentifier("needs-you")
    }

    // MARK: - Runners in trouble

    /// A row for each runner that isn't simply answering, above everything
    /// it's about.
    @ViewBuilder
    private var trouble: some View {
        let troubled = fleet.runners.filter { $0.connection.phase != .connected }
        if !troubled.isEmpty {
            Section {
                ForEach(troubled, id: \.host.id) { runner in
                    RunnerStatusRow(
                        connection: runner.connection,
                        host: runner.host,
                        onRetry: { fleet.retry(runner.host.id) },
                        onReconnectNow: { runner.connection.reconnectNow() },
                        onTrust: { hosts.trust(runner.host, fingerprint: $0) },
                        onReviewKey: { hosts.forgetKey(runner.host) },
                        onNotNow: { runner.connection.declineHostKey(runner.host) },
                        onEdit: { editingRunner = runner.host },
                        onAuthorize: { authorizing = true })
                }
            }
        }
    }

    // MARK: - The items

    @ViewBuilder
    private var items: some View {
        if !fleet.needsYou.isEmpty {
            Section {
                ForEach(fleet.needsYou) { item in
                    NeedsYouRow(
                        item: item,
                        mayAnswer: mayAnswer(item),
                        workspace: workspaceName(item),
                        runnerLabel: fleet.runners.count > 1
                            ? fleet.runner(item.runner)?.label : nil,
                        onOpen: { open(stack(for: item)) },
                        onAnswer: { action in await answer(item, with: action) },
                        onAnswerDecision: { text in await answerDecision(item, with: text) })
                }
            } footer: {
                olderRunners
            }
        } else if fleet.needsYouReadings.contains(.read) {
            Section {
                VStack(alignment: .leading, spacing: 4) {
                    Label("Nothing needs you", systemImage: "checkmark.circle")
                        .foregroundStyle(.secondary)
                    if let caveat = PhoneInbox.caveat(unanswered: unanswered) {
                        Text(caveat)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .accessibilityIdentifier("needs-you-caveat")
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("needs-you-nothing")
            } footer: {
                olderRunners
            }
        } else if !fleet.runners.isEmpty {
            Section {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Checking what needs you…").foregroundStyle(.secondary)
                }
            }
        }
    }

    /// The runners whose items were derived from their fleet, each told to
    /// update (spec §2.6).
    @ViewBuilder
    private var olderRunners: some View {
        if !fleet.olderRunners.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(fleet.olderRunners) { runner in
                    Text(NeedsYou.olderRunnerNote(runner: runner.label))
                }
            }
            .accessibilityIdentifier("needs-you-older")
        }
    }

    /// The runners that haven't answered what needs them, by name.
    private var unanswered: [String] {
        fleet.runners.filter { !$0.connection.needsYouRead || !$0.connection.isAnswering }
            .map(\.host.label)
    }

    /// Where an item opens (spec §2.5): an ask or a block, its agent, with
    /// its workspace and task under it; a decision or a review, its task.
    private func stack(for item: NeedsYouItem) -> [PhoneRoute] {
        guard let connection = UUID(uuidString: item.runner).flatMap({ fleet.connection(for: $0) })
        else { return [] }
        if let terminal = item.terminal, item.kind == .ask || item.kind == .blocked,
            let link = connection.fleet.phoneLink(toTerminal: terminal.id, runner: item.runner)
        {
            if let segment = link.segment, case .workspace(let place)? = link.stack.last {
                segment.remember(for: place)
            }
            return link.stack
        }
        let workspace =
            item.workspaceID
            ?? (connection.fleet.workspaces == nil ? item.repositoryID : nil)
        guard let workspace else {
            if let worktree = item.worktree?.id ?? item.terminal?.worktreeID {
                return [.worktree(runner: item.runner, worktree: worktree, landing: .resume)]
            }
            return []
        }
        let place = PhoneWorkspace(runner: item.runner, workspace: workspace)
        if let task = item.task {
            return [.workspace(place), .task(place, task: task.id)]
        }
        return [.workspace(place)]
    }

    /// What an item's workspace is called: its own name, else, on a
    /// runner without workspaces, its repository's (the row it's counted
    /// under), else Unclaimed.
    private func workspaceName(_ item: NeedsYouItem) -> String {
        if !item.workspaceName.isEmpty { return item.workspaceName }
        if let connection = UUID(uuidString: item.runner).flatMap({ fleet.connection(for: $0) }),
            connection.fleet.workspaces == nil,
            let name = item.repositoryID.flatMap({ connection.repositoryNames[$0] })
        {
            return name
        }
        return "Unclaimed"
    }

    /// Whether this phone may answer on the item's runner: not on a Read
    /// grant, where the only button is Open (spec §2.5).
    private func mayAnswer(_ item: NeedsYouItem) -> Bool {
        UUID(uuidString: item.runner).flatMap { fleet.connection(for: $0) }?.daemon?.mayAct ?? true
    }

    private func answer(_ item: NeedsYouItem, with action: NeedsYouAction) async -> String? {
        guard let connection = UUID(uuidString: item.runner).flatMap({ fleet.connection(for: $0) })
        else { return "Far Cooler isn’t talking to this runner right now." }
        return await connection.answer(item, with: action)
    }

    private func answerDecision(_ item: NeedsYouItem, with text: String) async -> String? {
        guard let connection = UUID(uuidString: item.runner).flatMap({ fleet.connection(for: $0) }),
            let task = item.task
        else { return "Far Cooler isn’t talking to this runner right now." }
        return await connection.answerDecision(task: task.id, with: text)
    }

    // MARK: - The workspaces

    @ViewBuilder
    private var workspaces: some View {
        let runners = fleet.runners.filter { $0.connection.hasFleet }
        ForEach(runners, id: \.host.id) { runner in
            let connection = runner.connection
            let id = runner.host.id.uuidString
            let sections = connection.fleet.phoneSections(
                runner: id, names: connection.repositoryNames,
                items: fleet.needsYou.filter { $0.runner == id })
            ForEach(sections) { section in
                Section {
                    ForEach(section.workspaces) { row in
                        workspaceRow(row, connection: connection)
                    }
                    if !section.unclaimed.isEmpty {
                        worktreeGroup(
                            "Unclaimed", ids: section.unclaimed, count: section.unclaimedCount,
                            runner: id, connection: connection)
                    }
                    if !section.hidden.isEmpty {
                        worktreeGroup(
                            "Hidden", ids: section.hidden, count: 0, runner: id,
                            connection: connection)
                    }
                } header: {
                    Text(heading(section, runner: runner.host, sections: sections.count))
                }
            }
        }
    }

    /// "Workspaces", naming the repository when a runner has several, and
    /// the runner when there's more than one.
    private func heading(_ section: PhoneRepositorySection, runner: Runner, sections: Int) -> String {
        var words = "Workspaces"
        if sections > 1,
            let name = fleet.connection(for: runner.id)?.repositoryNames[section.repository]
        {
            words += " in \(name)"
        }
        if fleet.runners.count > 1 { words += " on \(runner.label)" }
        return words
    }

    private func workspaceRow(_ row: PhoneWorkspaceRow, connection: Connection) -> some View {
        Button {
            open([.workspace(row.place)])
        } label: {
            HStack(spacing: 10) {
                orchestratorMark(row, connection: connection)
                Text(row.name)
                    .foregroundStyle(.primary)
                if row.unread {
                    Circle()
                        .fill(Color.accentColor)
                        .frame(width: 7, height: 7)
                        .accessibilityHidden(true)
                }
                Spacer()
                CountBadge(count: row.count)
                Image(systemName: "chevron.forward")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(row.name)
        .accessibilityValue(spokenCount(row.count) + (row.unread ? ", Orchestrator finished" : ""))
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("workspace-row-\(row.name)")
    }

    /// The orchestrator's own mark, or a hollow circle for a workspace with
    /// none running.
    @ViewBuilder
    private func orchestratorMark(_ row: PhoneWorkspaceRow, connection: Connection) -> some View {
        if let id = row.orchestrator,
            let terminal = connection.fleet.worktrees.lazy.flatMap(\.terminals).first(where: {
                $0.id == id
            })
        {
            ShellMarkView(mark: ShellFleetMap.mark(of: terminal, now: Date()), size: 8)
                .frame(width: markWidth)
        } else {
            Image(systemName: "circle.dashed")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .frame(width: markWidth)
        }
    }

    /// Unclaimed and Hidden: a row that opens in place, drawn as the workspace
    /// rows above it are. It was a `DisclosureGroup`, whose bold primary
    /// chevron and unmarked, differently indented title sat beside the
    /// workspaces' gray ones. Same mark column, same chevron, same count.
    private func worktreeGroup(
        _ title: String, ids: [String], count: Int, runner: String, connection: Connection
    ) -> some View {
        let key = "\(runner)/\(title)"
        let isOpen = openGroups.contains(key)
        return Group {
            Button {
                if isOpen { openGroups.remove(key) } else { openGroups.insert(key) }
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: title == "Hidden" ? "eye.slash" : "tray")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                        .frame(width: markWidth)
                    Text(title)
                        .foregroundStyle(.primary)
                    Spacer()
                    CountBadge(count: count)
                    Image(systemName: "chevron.forward")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(isOpen ? 90 : 0))
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityValue(isOpen ? "Expanded" : "Collapsed")
            .accessibilityAddTraits(.isButton)
            .accessibilityIdentifier("worktrees-\(title.lowercased())")
            if isOpen {
                ForEach(ids, id: \.self) { id in
                    if let worktree = connection.fleet.worktrees.first(where: { $0.id == id }) {
                        WorktreeRow(worktree: worktree, inbox: connection.inbox[id]) {
                            open([.worktree(runner: runner, worktree: id, landing: .resume)])
                        }
                    }
                }
            }
        }
    }

    private func spokenCount(_ count: Int) -> String {
        switch count {
        case 0: "Nothing needs you"
        case 1: "1 thing needs you"
        default: "\(count) things need you"
        }
    }
}

/// A workspace's needs-you count, in amber, and nothing at zero.
struct CountBadge: View {
    let count: Int
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        if count > 0 {
            Text("\(count)")
                .font(.caption.weight(.semibold))
                .monospacedDigit()
                .foregroundStyle(.black)
                .padding(.horizontal, 7)
                .padding(.vertical, 2)
                .background(GlancePalette.amber(scheme), in: Capsule())
        }
    }
}

/// One worktree, as a row that opens it: its name, its branch, the tasks
/// working in it, and its changes.
struct WorktreeRow: View {
    let worktree: Worktree
    let inbox: InboxRow?
    let onOpen: () -> Void

    var body: some View {
        Button(action: onOpen) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(worktree.task)
                            .foregroundStyle(.primary)
                        ForEach(worktree.openTasks ?? [], id: \.id) { task in
                            Text(task.key)
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                        }
                    }
                    if !worktree.branch.isEmpty {
                        Text(worktree.branch)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer()
                if let inbox, inbox.hasDiff {
                    Text("+\(inbox.insertions) −\(inbox.deletions)")
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                }
                Image(systemName: "chevron.forward")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("worktree-row-\(worktree.task)")
    }
}

/// One thing that needs you: what it's about, what it asks, and its answers.
struct NeedsYouRow: View {
    let item: NeedsYouItem
    /// Whether this phone may answer it. False on a Read grant, which gets
    /// Open alone.
    let mayAnswer: Bool
    /// What its workspace is called, as the row it's counted under is.
    let workspace: String
    /// The runner's name, when the list holds more than one runner's items.
    let runnerLabel: String?
    let onOpen: () -> Void
    let onAnswer: (NeedsYouAction) async -> String?
    let onAnswerDecision: (String) async -> String?

    /// The answer on its way, by action id, until the item leaves or the
    /// runner refuses it.
    @State private var sending: String?
    /// Why the last answer didn't land.
    @State private var failure: String?
    @State private var writing = false
    @State private var draft = ""

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button(action: onOpen) { words }
                .buttonStyle(.plain)
                .accessibilityElement(children: .combine)
                .accessibilityHint(opens)
                .accessibilityIdentifier("needs-you-item-\(item.itemID)")
            buttons
            if let failure {
                Text(failure)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("needs-you-failure-\(item.itemID)")
            }
        }
        .padding(.vertical, 2)
        .alert("Answer", isPresented: $writing) {
            TextField("Your answer", text: $draft)
            Button("Cancel", role: .cancel) { draft = "" }
            Button("Send") { send(id: "answer") { await onAnswerDecision(draft) } }
        } message: {
            Text(item.question)
        }
    }

    private var words: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Image(systemName: symbol)
                    .foregroundStyle(tint)
                    .accessibilityHidden(true)
                Text(context)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: 4)
                if let since = item.since {
                    BoardTick { now in
                        Text(TaskRow.ago(now.timeIntervalSince(since)))
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                    }
                }
            }
            Text(item.question)
                .font(.subheadline)
                .foregroundStyle(.primary)
                .lineLimit(3)
            if let detail = item.distinctDetail {
                Text(detail)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
    }

    /// Where it is and what it's about: "Billing · bil-7", or the agent.
    private var context: String {
        let subject: String? =
            if let task = item.task {
                task.key
            } else if let terminal = item.terminal {
                terminal.isOrchestrator ? "Orchestrator" : terminal.label
            } else {
                nil
            }
        var words = [workspace]
        if let subject { words.append(subject) }
        if let runnerLabel { words.append(runnerLabel) }
        return words.joined(separator: " · ")
    }

    /// Where tapping it goes, for VoiceOver: an ask or a block opens its
    /// agent, a decision or a review its task.
    private var opens: String {
        switch item.kind {
        case .ask, .blocked: item.terminal?.isOrchestrator == true ? "Opens the orchestrator" : "Opens the agent"
        case .decision, .review: "Opens the task"
        case .unknown: "Opens its workspace"
        }
    }

    private var symbol: String {
        switch item.kind {
        case .ask: "hand.raised.fill"
        case .blocked: "exclamationmark.bubble.fill"
        case .decision: "questionmark.circle.fill"
        case .review: "eye"
        case .unknown: "circle"
        }
    }

    private var tint: Color {
        item.kind == .review ? GlancePalette.review(scheme) : GlancePalette.amber(scheme)
    }

    /// The answers the runner offered, else the one move there is. Below
    /// Control scope an item has no actions, and gets Open alone.
    @ViewBuilder
    private var buttons: some View {
        let answers = item.actions.filter { !$0.isOpen }
        HStack(spacing: 8) {
            switch item.kind {
            case .ask where !answers.isEmpty && mayAnswer:
                ForEach(answers, id: \.id) { action in answerButton(action) }
            case .decision where !answers.isEmpty && mayAnswer:
                ForEach(answers.prefix(TaskQuestion.buttonLimit), id: \.id) { action in
                    answerButton(action)
                }
                if answers.count > TaskQuestion.buttonLimit {
                    Menu("More") {
                        ForEach(answers.dropFirst(TaskQuestion.buttonLimit), id: \.id) { action in
                            Button(action.title) {
                                send(id: action.id) { await onAnswerDecision(action.id) }
                            }
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                }
            case .decision where item.task != nil && mayAnswer:
                Button("Answer…") {
                    draft = ""
                    writing = true
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .disabled(sending != nil)
                .accessibilityIdentifier("needs-you-answer-\(item.itemID)")
            case .review:
                Button("Review", action: onOpen)
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                    .accessibilityIdentifier("needs-you-review-\(item.itemID)")
            default:
                Button("Open", action: onOpen)
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                    .accessibilityIdentifier("needs-you-open-\(item.itemID)")
            }
            if sending != nil {
                ProgressView().controlSize(.small)
            }
        }
    }

    private func answerButton(_ action: NeedsYouAction) -> some View {
        Button(role: action.destructive ? .destructive : nil) {
            send(id: action.id) {
                if item.kind == .decision {
                    await onAnswerDecision(action.id)
                } else {
                    await onAnswer(action)
                }
            }
        } label: {
            Text(action.title).lineLimit(1)
        }
        .buttonStyle(.bordered)
        .tint(action.primary ? .accentColor : nil)
        .controlSize(.small)
        .disabled(sending != nil)
        .accessibilityIdentifier("needs-you-action-\(item.itemID)-\(action.id)")
    }

    /// Send one answer. The spinner stays while it's out; on success the
    /// item leaves with the runner's next list, and a refusal stays under it
    /// in words (spec §2.5).
    private func send(id: String, _ answer: @escaping () async -> String?) {
        guard sending == nil else { return }
        sending = id
        failure = nil
        Task {
            let refused = await answer()
            sending = nil
            failure = refused
        }
    }
}
