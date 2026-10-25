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
    /// The runner whose Add Repository sheet is open (a runner with none).
    @State private var addingRepository: Runner?
    /// The runner whose settings a Claude login line opened, with the
    /// sign-in sheet up (ov-587).
    @State private var signingIn: SignInRunner?
    /// The extra read-only folder open in Files, and the runner it's on (ov-259).
    @State private var browsingFolder: BrowsedFolder?
    /// Whether this device is signed in: the push note shows until it is.
    @ObservedObject private var account = Account.shared
    /// The Unclaimed and Hidden groups open right now, by runner and title.
    @State private var openGroups: Set<String> = []
    /// The mark column, which grows with the text so the mark never touches the name.
    @ScaledMetric private var markWidth: CGFloat = 14

    var body: some View {
        List {
            trouble
            items
            allAgents
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
                onRemove: { hosts.remove($0) },
                onSwitchRoute: { hosts.switchRoute($0) })
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
        .sheet(item: $browsingFolder) { folder in
            if let connection = fleet.connection(for: folder.runner) {
                FilesSheet(connection: connection, root: FilesLocation(place: .folder(folder.name)))
            }
        }
        .sheet(item: $signingIn) { runner in
            if let connection = fleet.connection(for: runner.id) {
                NavigationStack {
                    RunnerSettingsView(name: connection.hostLabel, connection: connection, signInOnOpen: true)
                }
            }
        }
        .sheet(item: $addingRepository) { runner in
            if let connection = fleet.connection(for: runner.id) {
                AddRepositorySheet(connection: connection) { _ in }
            }
        }
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
                        onAuthorize: { authorizing = true },
                        onConnectDirectly: { hosts.switchRoute(runner.host) })
                }
            }
        }
    }

    // MARK: - All Agents

    /// All Agents (ov-617): every agent at work on every runner, one row
    /// above the workspaces that opens them all, with the first few dots and
    /// how many run.
    private var allAgents: some View {
        let all = fleet.runners.map {
            (agents: RunningAgents.runner($0.host.id.uuidString, fleet: $0.connection.fleet), answering: $0.connection.isAnswering)
        }
        let count = all.reduce(0) { $0 + $1.agents.count }
        // Each runner's marks as it can say them: dimmed where it isn't answering.
        let dots = Array(all.flatMap { runner in runner.agents.dots.map { $0.said(answering: runner.answering) } }.prefix(RunningAgents.dotLimit))
        return Section {
            Button { open([.agents]) } label: {
                HStack(spacing: Spacing.group) {
                    Label(AgentsWords.allTitle, systemImage: "person.2")
                        .foregroundStyle(.primary)
                    Spacer(minLength: Spacing.group)
                    HStack(spacing: -2) {
                        ForEach(Array(dots.enumerated()), id: \.offset) { _, mark in
                            GlanceMarkView(mark, inAppDiameter: 8)
                        }
                    }
                    Text(count == 0 ? "None" : "\(count)")
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                    Image(systemName: "chevron.forward")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.tertiary)
                        .accessibilityHidden(true)
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(AgentsWords.allTitle)
            .accessibilityValue(RunningAgents.summary(count))
            .accessibilityAddTraits(.isButton)
            .accessibilityIdentifier("all-agents-row")
        }
    }

    // MARK: - The items

    private func rows(_ items: [NeedsYouItem]) -> some View {
        ForEach(items) { item in
            // A thread's Reply is the owner's (ov-712): its row redraws when
            // the runner says who this phone is, which comes after the list.
            if item.kind == .prThread, let connection = UUID(uuidString: item.runner).flatMap({ fleet.connection(for: $0) }) {
                WatchingViewer(talk: connection.talk, connection: connection) { row(item) }
            } else {
                row(item)
            }
        }
    }

    private func row(_ item: NeedsYouItem) -> NeedsYouRow {
        NeedsYouRow(
            item: item,
            mayAnswer: mayAnswer(item),
            workspace: workspaceName(item),
            runnerLabel: fleet.runners.count > 1
                ? fleet.runner(item.runner)?.label : nil,
            onOpen: { open(stack(for: item)) },
            onSignIn: { signingIn = UUID(uuidString: item.runner).map(SignInRunner.init) },
            onAnswer: { action in await answer(item, with: action) },
            onAnswerDecision: { text in await answerDecision(item, with: text) },
            replies: replies(item),
            teamRunners: NeedsYou.teamRunners(in: fleet.needsYou),
            listIsSplit: NeedsYou.grouped(fleet.needsYou).isSplit)
    }

    @ViewBuilder
    private var items: some View {
        if !fleet.needsYou.isEmpty {
            NeedsYouSections(items: fleet.needsYou, rows: rows) { footer }
        } else if fleet.needsYouReadings.contains(.read) {
            Section {
                VStack(alignment: .leading, spacing: 4) {
                    Label(FirstRunCopy.Phone.nothingNeedsYou, systemImage: "checkmark.circle")
                        .foregroundStyle(.secondary)
                    if let copy = PhoneFirstRun.noOrchestratorCopy(sections: allSections, working: workingAgents) {
                        PhoneEmptyRows(copy: copy, compact: true)
                            .padding(.top, 4)
                            .accessibilityIdentifier("needs-you-no-orchestrator")
                        // The step the words name (ov-519): Start Orchestrator opens the
                        // workspace, whose Plan leads with it, where the agent is chosen.
                        if let place = PhoneFirstRun.workspaceToStart(sections: allSections, working: workingAgents) {
                            // Centered under the centered words above it.
                            HStack {
                                Spacer(minLength: 0)
                                Button(FirstRunCopy.Phone.start) { open([.workspace(place)]) }
                                .buttonStyle(.bordered)
                                .accessibilityIdentifier("needs-you-start-orchestrator")
                                Spacer(minLength: 0)
                            }
                            .padding(.top, 6)
                        }
                    }
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
                footer
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

    /// Under the items, or under the empty state: the older-runner notes and,
    /// until this device is signed in, why notifications stop with the app.
    @ViewBuilder
    private var footer: some View {
        VStack(alignment: .leading, spacing: 8) {
            olderRunners
            if !account.isSignedIn {
                VStack(alignment: .leading, spacing: 4) {
                    Text(FirstRunCopy.Phone.pushBody)
                    Button(FirstRunCopy.Phone.signIn) {
                        Task {
                            await Account.shared.signIn()
                            await AccountSection.afterSignIn?()
                        }
                    }
                    .font(.footnote.weight(.semibold))
                    .accessibilityIdentifier("needs-you-sign-in")
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("needs-you-push")
            }
        }
    }

    /// The agents working on any runner, whoever started them.
    private var workingAgents: Int {
        fleet.runners
            .flatMap { $0.connection.fleet.worktrees }
            .flatMap(\.terminals)
            .filter { $0.agent == .working }
            .count
    }

    /// Every runner's sections, as the workspaces below are drawn from them.
    private var allSections: [PhoneRepositorySection] {
        fleet.runners.filter { $0.connection.hasFleet }.flatMap { runner in
            let id = runner.host.id.uuidString
            return runner.connection.fleet.phoneSections(
                runner: id, names: runner.connection.repositoryNames,
                items: fleet.needsYou.filter { $0.runner == id })
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
            // An orchestrator's pane is its workspace's conversation, which
            // is what the workspace opens on.
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

    /// What an item's workspace is called: its repository's for a main
    /// checkout, else the repository and the workspace, else, on a
    /// runner without workspaces, its repository's (the row it's counted
    /// under), else Unclaimed.
    private func workspaceName(_ item: NeedsYouItem) -> String {
        let connection = UUID(uuidString: item.runner).flatMap { fleet.connection(for: $0) }
        if !item.workspaceName.isEmpty {
            // Named as All Agents names it, so two repositories' Main differ (ov-676).
            guard let connection, let id = item.workspaceID, let summary = connection.workspace(id) else {
                return item.workspaceName
            }
            return item.listedWorkspace(
                in: summary, repositoryName: connection.repositoryNames[summary.repository ?? summary.id])
        }
        if let connection,
            connection.fleet.workspaces == nil,
            let name = item.repositoryID.flatMap({ connection.repositoryNames[$0] })
        {
            return name
        }
        // A review request is in no workspace, and Unclaimed would say
        // something is wrong with it.
        // A login is the person's, in no workspace (ov-587).
        if item.kind == .claudeLogin { return "Logins" }
        return item.kind == .prReview ? "Pull Request" : "Unclaimed"
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

    /// Replying to an escalated thread (ov-317), where this phone may: the runner
    /// has `pr_actions`, this device holds Control, and it's the owner's
    /// (ov-712). Nil otherwise, and the row draws no Reply.
    private func replies(_ item: NeedsYouItem) -> ThreadReplies? {
        guard item.kind == .prThread, let task = item.task?.id,
            let connection = UUID(uuidString: item.runner).flatMap({ fleet.connection(for: $0) }),
            connection.mayActOnPullRequests
        else { return nil }
        return ThreadReplies(
            task: task,
            preview: { await connection.previewReply(task: task) },
            send: { body, shown in await connection.sendReply(task: task, body: body, shown: shown) })
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
            if PhoneFirstRun.hasNoRepositories(sections) {
                noRepositories(runner.host)
            }
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
            folders(of: runner.host, connection: connection)
        }
    }

    /// One runner's extra read-only folders, by name (ov-232): the logs and
    /// notes its owner chose to share, never addable from here (ov-259).
    /// Nothing at all from a runner without any, or too old to have them.
    @ViewBuilder
    private func folders(of runner: Runner, connection: Connection) -> some View {
        let names = connection.knownBuild?.sharedFolders ?? []
        if !names.isEmpty {
            Section {
                ForEach(names, id: \.self) { name in
                    Button {
                        browsingFolder = BrowsedFolder(runner: runner.id, name: name)
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: "folder")
                                .foregroundStyle(.secondary)
                                .frame(width: markWidth)
                            Text(name)
                            Spacer(minLength: 0)
                            Image(systemName: "chevron.right")
                                .font(.footnote.weight(.semibold))
                                .foregroundStyle(.tertiary)
                        }
                        .frame(minHeight: PaneMetrics.target)
                    }
                    .foregroundStyle(.primary)
                    .accessibilityIdentifier("folder-row-\(name)")
                }
            } header: {
                Text(fleet.runners.count > 1 ? "Folders on \(runner.label)" : "Folders")
            }
        }
    }

    /// A runner that's answering but lists no repository: what to add, and the
    /// button for it. Phones had no door to this before (ov-205).
    private func noRepositories(_ runner: Runner) -> some View {
        Section {
            VStack(spacing: 8) {
                Text(FirstRunCopy.Phone.noRepositoriesTitle(runner.label))
                    .font(.headline)
                    .multilineTextAlignment(.center)
                    .accessibilityIdentifier("no-repositories-title")
                PhoneEmptyRows(copy: PhoneEmptyStates.noRepositories, compact: true)
                    .accessibilityIdentifier("no-repositories-body")
                if fleet.connection(for: runner.id)?.daemon?.mayAct ?? true {
                    Button(FirstRunCopy.Phone.addRepository) { addingRepository = runner }
                        .buttonStyle(.borderedProminent)
                        .padding(.top, 4)
                        .accessibilityIdentifier("no-repositories-add")
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("no-repositories")
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
            // Where it was left: a task, a plan page, a tree level pushed
            // over it, as long as that is still on its board (ov-442).
            let board = connection.boards[row.place.workspace]
            open(
                WorkspacePlaceMemory.opening(row.place) { route in
                    guard case .task(_, let task) = route, let board else { return true }
                    return board.rows.contains { $0.id == task }
                })
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
            GlanceMarkView(ShellFleetMap.mark(of: terminal).said(answering: connection.isAnswering), inAppDiameter: 8)
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

/// A Needs You list's sections: Yours, then Everyone Else, once anything is
/// someone else's (ov-633); one section with no header otherwise, as it always
/// was. The iPhone's screen and the iPad's plan column both draw it (ov-705),
/// so the two can't file an item differently. `footer` goes under the last.
struct NeedsYouSections<Rows: View, Footer: View>: View {
    let items: [NeedsYouItem]
    let rows: ([NeedsYouItem]) -> Rows
    @ViewBuilder let footer: () -> Footer

    var body: some View {
        let groups = NeedsYou.grouped(items)
        if groups.isSplit {
            if !groups.yours.isEmpty {
                Section {
                    rows(groups.yours)
                } header: {
                    Text(NeedsYou.yoursTitle).accessibilityIdentifier("needs-you-group-yours")
                }
            }
            Section {
                rows(groups.others)
            } header: {
                Text(NeedsYou.othersTitle).accessibilityIdentifier("needs-you-group-others")
            } footer: {
                footer()
            }
        } else {
            Section {
                rows(items)
            } footer: {
                footer()
            }
        }
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
    /// A Claude login line's button (ov-587): Runner Settings with the sign-in
    /// sheet up.
    var onSignIn: () -> Void = {}
    let onAnswer: (NeedsYouAction) async -> String?
    let onAnswerDecision: (String) async -> String?
    /// Replying to a review thread on GitHub, for a PR_THREAD on a runner that
    /// allows it; nil for every other row.
    var replies: ThreadReplies?
    /// The runners whose rows say who each item is for (`NeedsYou.teamRunners`).
    var teamRunners: Set<String> = []
    /// Whether the list is split under Yours and Everyone Else, whose header
    /// says "You" already.
    var listIsSplit = false

    /// What an answer to someone else's item does once the person confirms
    /// it (ov-737); nil while nothing waits on them.
    @State private var confirming: Confirmation?
    /// The answer on its way, by action id, until the item leaves or the
    /// runner refuses it.
    @State private var sending: String?
    /// Why the last answer didn't land.
    @State private var failure: String?
    @State private var writing = false
    @State private var replying = false
    @State private var draft = ""

    @Environment(\.colorScheme) private var scheme
    @Environment(\.openURL) private var openURL
    @Environment(\.dynamicTypeSize) private var typeSize

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
        .probesShown()
        .sheet(isPresented: $replying) {
            if let replies {
                ThreadReplySheet(item: item, replies: replies)
            }
        }
        // An alert, not a confirmation dialog: on an iPhone the dialog is drawn
        // as a popover with no Cancel button, and this one has both choices on
        // screen (ov-737).
        .alert(
            item.answerForOthersPrompt ?? "",
            isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } })
        ) {
            Button("Cancel", role: .cancel) { confirming = nil }
            Button("Answer") {
                let step = confirming?.step
                confirming = nil
                step?()
            }
            .accessibilityIdentifier("needs-you-confirm-\(item.itemID)")
        }
        // Someone else's item asks in this alert: its title is the question
        // and Answer sends, so typing and confirming are one step.
        .alert(item.answerForOthersPrompt ?? "Answer", isPresented: $writing) {
            TextField("Your answer", text: $draft)
            Button("Cancel", role: .cancel) { draft = "" }
            Button(item.answerForOthersPrompt == nil ? "Send" : "Answer") {
                send(id: "answer") { await onAnswerDecision(draft) }
            }
        } message: {
            Text(item.question)
        }
    }

    private var words: some View {
        VStack(alignment: .leading, spacing: 3) {
            if typeSize.isAccessibilitySize {
                // The age goes under the context, not beside it: beside it the
                // two shared a line too narrow for either, and the context was
                // cut to "Billing · b…" (ov-422).
                VStack(alignment: .leading, spacing: 2) {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        mark
                        contextText(lineLimit: 3)
                    }
                    age
                }
            } else {
                HStack(spacing: 6) {
                    mark
                    contextText(lineLimit: 1)
                    Spacer(minLength: 4)
                    age
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
            // Whose it is (ov-633, ov-737): wraps rather than cutting a long name.
            if let whose = item.whose(teamRunners: teamRunners, listIsSplit: listIsSplit) {
                // Not a Label: in a list its icon takes a row icon's width.
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Image(systemName: "person").imageScale(.small)
                    Text(whose).fixedSize(horizontal: false, vertical: true)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("needs-you-whose")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
    }

    private var mark: some View {
        Image(systemName: symbol)
            .foregroundStyle(tint)
            .accessibilityHidden(true)
    }

    private func contextText(lineLimit: Int) -> some View {
        Text(context)
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(lineLimit)
            // Its own height for its lines: a decision's row, with its
            // question and answers under it, was given one line at the
            // largest size once the repository joined the words (ov-676),
            // and "overnight · Billing · bil-7" was cut.
            .fixedSize(horizontal: false, vertical: true)
            .textFitProbe("needs-you-context", text: context, font: .caption)
    }

    @ViewBuilder
    private var age: some View {
        if let since = item.since {
            BoardTick { now in
                let ago = TaskRow.ago(now.timeIntervalSince(since))
                Text(ago)
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .fixedSize()  // never "2m / ago"
                    .textFitProbe("needs-you-age", text: ago, font: .caption2)
            }
        }
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
        case .decision, .prThread, .review: "Opens the task"
        // Somebody else's pull request opens nothing here: no hint says so.
        case .prReview: ""
        case .claudeLogin: ""
        case .unknown: "Opens its workspace"
        }
    }

    private var symbol: String {
        switch item.kind {
        case .ask: "hand.raised.fill"
        case .blocked: "exclamationmark.bubble.fill"
        case .decision: "questionmark.circle.fill"
        case .review: "eye"
        case .prThread: "text.bubble.fill"
        case .prReview: "arrow.triangle.pull"
        case .claudeLogin: "person.crop.circle.badge.exclamationmark"
        case .unknown: "circle"
        }
    }

    private var tint: Color {
        item.kind == .review || item.kind == .prReview ? GlancePalette.review(scheme) : GlancePalette.amber(scheme)
    }

    /// The answers the runner offered, else the one move there is. Below
    /// Control scope an item has no actions, and gets Open alone.
    @ViewBuilder
    private var buttons: some View {
        let answers = item.actions.filter { !$0.isOpen }
        VStack(alignment: .leading, spacing: 8) {
            if item.kind == .prThread, github != nil || replies != nil {
                // Beside the answers the card offers: read the thread where it is,
                // and reply to the reviewer in the person's own words (ov-317).
                HStack(spacing: 8) {
                    githubButton
                    if replies != nil {
                        Button(PhonePrActions.reply) { replying = true }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                            .disabled(sending != nil)
                            .accessibilityIdentifier("needs-you-reply-\(item.itemID)")
                    }
                }
            }
            HStack(spacing: 8) { answerButtons(answers) }
        }
    }

    /// The pull request or thread's page, only when it is a github.com link.
    private var github: URL? { GitHubLink.url(item.link) }

    @ViewBuilder private var githubButton: some View {
        if let github {
            // Somebody else's pull request is reviewed there; a thread is opened there.
            Button(item.kind == .prReview ? PhonePrActions.reviewOnGitHub : PhonePrActions.openOnGitHub) { openURL(github) }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .accessibilityIdentifier("needs-you-github-\(item.itemID)")
        }
    }

    @ViewBuilder
    private func answerButtons(_ answers: [NeedsYouAction]) -> some View {
        Group {
            switch item.kind {
            case .ask where !answers.isEmpty && mayAnswer:
                ForEach(answers, id: \.id) { action in answerButton(action) }
            case _ where item.kind.isDecision && !answers.isEmpty && mayAnswer:
                ForEach(answers.prefix(TaskQuestion.buttonLimit), id: \.id) { action in
                    answerButton(action)
                }
                if answers.count > TaskQuestion.buttonLimit {
                    Menu("More") {
                        ForEach(answers.dropFirst(TaskQuestion.buttonLimit), id: \.id) { action in
                            Button(action.title) {
                                confirmed {
                                    send(id: action.id) { await onAnswerDecision(action.id) }
                                }
                            }
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                }
            case _ where item.kind.isDecision && item.task != nil && mayAnswer:
                Button("Answer…") {
                    // No separate confirmation: for someone else's item the
                    // text alert itself asks (below), so there is no second
                    // alert to raise as the first goes (ov-737).
                    draft = ""
                    writing = true
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .disabled(sending != nil)
                .accessibilityIdentifier("needs-you-answer-\(item.itemID)")
            case .prReview:
                // Somebody else's pull request is read on GitHub (ov-317): the
                // button is there when the runner sent a github.com link, and
                // absent below Control, where it sends none.
                githubButton
            case .claudeLogin:
                // Answered at any scope (the sheet makes its own calls), so
                // `mayAnswer` isn't asked; the owner's copy has no action.
                ForEach(answers.filter(\.isSignIn), id: \.id) { action in
                    Button(action.title, action: onSignIn)
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                        .accessibilityIdentifier("needs-you-sign-in-\(item.itemID)")
                }
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
            confirmed {
                send(id: action.id) {
                    if item.kind.isDecision {
                        await onAnswerDecision(action.id)
                    } else {
                        await onAnswer(action)
                    }
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

    /// What waits on a person's yes to answering for someone else.
    private struct Confirmation {
        let step: () -> Void
    }

    /// Run `step` now, or after the person confirms when the item is for
    /// someone else (ov-737). Yours and nobody's go straight through.
    private func confirmed(_ step: @escaping () -> Void) {
        if item.answerForOthersPrompt == nil { step() } else { confirming = Confirmation(step: step) }
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

/// A folder a runner shares, picked from Needs You.
private struct BrowsedFolder: Identifiable {
    let runner: UUID
    let name: String
    var id: String { "\(runner)/\(name)" }
}

/// The runner a Claude login line opened settings on (ov-587).
struct SignInRunner: Identifiable {
    let id: UUID
}

/// Redraws `content` when the runner says who this phone is (`TalkSeats`):
/// what a row gated on the viewer watches, on a screen that otherwise
/// watches only the fleet (ov-712). It asks, too, if nothing has yet: the
/// list can come before the fleet's poll does.
private struct WatchingViewer<Content: View>: View {
    @ObservedObject var talk: TalkSeats
    let connection: Connection
    @ViewBuilder let content: () -> Content

    var body: some View {
        content().task { await talk.load(connection) }
    }
}
