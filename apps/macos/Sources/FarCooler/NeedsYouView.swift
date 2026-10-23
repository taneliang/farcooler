import AgentKit
import SwiftUI

// Needs You on the Mac (spec §4.6): every item from every workspace, in rank
// order, answerable in place.
//
// What each row offers, and what a refusal says, are values here
// (`NeedsYouRowModel`), so `NeedsYouViewTests` pins them; the view draws them.

enum NeedsYouRowModel {
    /// One button on a row.
    enum Button: Equatable, Hashable {
        /// One of an ask's own options, sent with `terminal agent-answer`.
        case ask(option: String, title: String, destructive: Bool, primary: Bool)
        /// One of a decision's options, sent as the answer.
        case decide(String)
        /// A decision with no options: a field for the answer.
        case answerTyped
        /// Open the task's column with its changes. Never an approval: the
        /// charter says who lands work (ruling 2).
        case review
        /// Go where the item is, sending nothing.
        case open
        /// A Claude login line's button (ov-587): Runner Settings on that
        /// runner with the Claude sign-in sheet up. Sends nothing from here.
        case signIn(String)
        /// Open a pull request or a review thread on GitHub (ov-315), by its
        /// link. Sends nothing to the runner.
        case github(String)
        /// The same for somebody else's pull request, which is reviewed there
        /// (ov-317): "Review on GitHub", where a thread's is "Open on GitHub".
        case githubReview(String)
    }

    /// `link` when it is something this Mac may hand to the browser: an https
    /// URL on github.com, bare, and nothing else (`GitHubLink`). The runner
    /// sends one only at Control scope, and a link is text from a repository,
    /// so it is checked here too.
    static func githubLink(_ link: String?) -> URL? { GitHubLink.url(link) }

    /// The group a row is filed under: its workspace, else "Pull Request" for
    /// a review request, which belongs to no workspace, "Logins" for a
    /// login line, else "Unclaimed".
    static func group(_ item: NeedsYouItem) -> String {
        if !item.workspaceName.isEmpty { return item.workspaceName }
        // A login is the person's, in no workspace; Unclaimed would say
        // something is wrong with it (ov-587).
        if item.kind == .claudeLogin { return "Logins" }
        return item.kind == .prReview ? "Pull Request" : "Unclaimed"
    }

    /// The runner whose settings a login line's button opens (ov-587): its
    /// target is the runner key, and this Mac's own is "".
    static func settingsChoice(for item: NeedsYouItem) -> RunnerChoice {
        RunnerChoice(name: item.runner.isEmpty ? "This Mac" : item.runner, target: item.runner)
    }

    /// The most options drawn as buttons; the rest go in a More menu.
    static let visibleOptions = 3

    /// What `item` offers: its buttons, and a decision's options past the
    /// third. A connection that can't act (below Control, where the runner
    /// sends no actions) gets Open alone, as the board does.
    static func buttons(for item: NeedsYouItem, canAct: Bool) -> (buttons: [Button], more: [Button]) {
        let actionable = item.actions.filter { !$0.isOpen }
        // A login line's button is the member's own sheet, answered at any
        // scope. The owner's copy of the line has no action and so no button.
        if item.kind == .claudeLogin {
            return (item.actions.filter(\.isSignIn).map { .signIn($0.title) }, [])
        }
        // A review request is somebody else's pull request: the one thing to
        // do with it is read it on GitHub. Below Control there is no link,
        // and so no button; Open would go nowhere.
        if item.kind == .prReview {
            return (githubLink(item.link).map { [.githubReview($0.absoluteString)] } ?? [], [])
        }
        guard canAct else { return ([.open], []) }
        switch item.kind {
        case .ask:
            guard !actionable.isEmpty, item.askID != nil else { return ([.open], []) }
            return (actionable.map { .ask(option: $0.id, title: $0.title, destructive: $0.destructive, primary: $0.primary) }, [])
        case .decision:
            guard item.task != nil else { return ([.open], []) }
            let options = actionable.map { Button.decide($0.title) }
            if options.isEmpty { return ([.answerTyped], []) }
            return (Array(options.prefix(visibleOptions)), Array(options.dropFirst(visibleOptions)))
        case .prThread:
            // Answered as a decision is, beside a way to read the thread.
            let thread = githubLink(item.link).map { [Button.github($0.absoluteString)] } ?? []
            guard item.task != nil else { return (thread.isEmpty ? [.open] : thread, []) }
            let options = actionable.map { Button.decide($0.title) }
            if options.isEmpty { return (thread + [.answerTyped], []) }
            return (thread + options.prefix(visibleOptions), Array(options.dropFirst(visibleOptions)))
        case .review:
            return ([.review], [])
        case .blocked, .unknown, .prReview, .claudeLogin:
            return ([.open], [])
        }
    }

    /// Where a row is while it's being answered.
    enum Answering: Equatable {
        case idle
        /// A button has sent; its spinner shows in place.
        case sending
        /// The runner refused: the item stays, with this line.
        case refused(String)
    }

    /// The line an ask keeps when its answer is refused (spec §2.5), naming
    /// its agent.
    static func refused(_ refusal: DaemonClient.AskRefusal, item: NeedsYouItem) -> Answering {
        .refused(refusal.sentence(agent: item.terminal?.label))
    }

    /// How long a sent answer's spinner waits for its item to leave before
    /// the row comes back: a `needs_you_changed` that never arrives mustn't
    /// leave it spinning with no buttons.
    static let settleTimeout: Duration = .seconds(8)

    /// Where a row goes when `settleTimeout` passes: a row still sending
    /// gets its buttons back with a line; any other is left as it is.
    static func afterTimeout(_ state: Answering) -> Answering {
        state == .sending ? .refused("This hasn’t cleared yet. It may have gone through; check before answering again.") : state
    }

    /// The line a decision keeps when its answer wasn't written.
    static let decisionRefused = Answering.refused("Couldn’t send that answer. Try again.")

    /// What a row says it's about: the task's key, else the agent.
    static func subject(_ item: NeedsYouItem) -> String {
        if let task = item.task { return "\(task.key) \(task.title)" }
        if let terminal = item.terminal { return terminal.isOrchestrator ? "Orchestrator" : terminal.label }
        return ""
    }
}

/// The Needs You list, in the detail.
struct NeedsYouView: View {
    /// Under "Nothing Needs You": what the page is for, then what lands on
    /// it, one row each (ov-205).
    static let emptyCopy = EmptyStateCopy(
        lede: "Agents wait here when they need you.",
        rows: [
            .init(symbol: "questionmark.bubble", text: "Answer questions and make decisions"),
            .init(symbol: "eye", text: "Review finished work before you merge it"),
        ])

    /// What the empty page offers (ov-519): with no orchestrator running, a way
    /// to it, since nothing will ever wait here until one does.
    @Environment(\.orchestratorInvite) private var invite

    let items: [NeedsYouItem]
    /// Runners too old to send decisions and asks, by name: each gets its
    /// line (`NeedsYou.olderRunnerNote`).
    let olderRunners: [String]
    /// Runners that haven't said what needs a person, by name: under the list
    /// (or under "Nothing Needs You") the page says it may not be everything.
    var unanswered: [String] = []
    let canAct: (NeedsYouItem) -> Bool
    var onOpen: (NeedsYouItem) -> Void
    /// A Claude login line's button (ov-587).
    var onSignIn: (NeedsYouItem) -> Void = { _ in }
    var onAnswerAsk: (NeedsYouItem, String) async -> DaemonClient.AskRefusal?
    var onDecide: (NeedsYouItem, String) async -> Bool

    var body: some View {
        Group {
            if items.isEmpty {
                ContentUnavailableView {
                    EmptyStateTitle("Nothing Needs You", symbol: "tray")
                } description: {
                    VStack(spacing: Spacing.inset) {
                        EmptyStateRows(copy: Self.emptyCopy)
                        if let invite, !invite.seated {
                            Text(FirstRunCopy.NeedsYou.noAgents).identified("needs-you-no-agents")
                        }
                    }
                } actions: {
                    // Start Orchestrator, not "Show": with none running there is
                    // nothing to show, and the menu starts one in the first workspace.
                    if let invite, !invite.seated, invite.canAct {
                        StartOrchestratorMenu(invite: invite)
                    }
                }
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        // Yours, then Everyone Else, once anything is someone
                        // else's (ov-633); one list with no headers otherwise.
                        let groups = NeedsYou.grouped(items)
                        if groups.isSplit {
                            if !groups.yours.isEmpty {
                                groupTitle(NeedsYou.yoursTitle, first: true)
                                rows(groups.yours)
                            }
                            groupTitle(NeedsYou.othersTitle, first: groups.yours.isEmpty)
                            rows(groups.others)
                        } else {
                            rows(items)
                        }
                    }
                    .animation(.snappy(duration: 0.22), value: items.map(\.key))
                    .padding(16)
                    .frame(maxWidth: 720)
                    .frame(maxWidth: .infinity)
                }
            }
        }
        .safeAreaInset(edge: .bottom) {
            if !olderRunners.isEmpty || !unanswered.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    if let caveat = PhoneInbox.caveat(unanswered: unanswered) {
                        Text(caveat)
                    }
                    ForEach(olderRunners, id: \.self) { runner in
                        Text(NeedsYou.olderRunnerNote(runner: runner.isEmpty ? "this Mac" : runner))
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(10)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // One card on the window's frosted plane like every destination, with the
        // items on it as inset groups (ov-225, ov-297).
        .contentCard()
        .background { WindowPlane().ignoresSafeArea() }
        .navigationTitle("Needs You")
        .navigationSubtitle(items.isEmpty ? "" : (items.count == 1 ? "1 item" : "\(items.count) items"))
    }

    private func rows(_ items: [NeedsYouItem]) -> some View {
        ForEach(items) { item in
            NeedsYouItemRow(
                item: item, canAct: canAct(item), onOpen: { onOpen(item) },
                onSignIn: { onSignIn(item) },
                onAnswerAsk: { await onAnswerAsk(item, $0) },
                onDecide: { await onDecide(item, $0) })
                .transition(.opacity.combined(with: .move(edge: .leading)))
        }
    }

    /// A group's title: Yours, or Everyone Else.
    private func groupTitle(_ text: String, first: Bool) -> some View {
        SectionTitle(text: text)
            .padding(.top, first ? 0 : 12)
            .padding(.horizontal, 4)
            .accessibilityAddTraits(.isHeader)
            .identified("needs-you-group-\(text == NeedsYou.yoursTitle ? "yours" : "others")")
    }
}

/// One item: its workspace, its question, what it's about, and its
/// buttons.
struct NeedsYouItemRow: View {
    let item: NeedsYouItem
    let canAct: Bool
    var onOpen: () -> Void
    var onSignIn: () -> Void = {}
    var onAnswerAsk: (String) async -> DaemonClient.AskRefusal?
    var onDecide: (String) async -> Bool

    @State private var answering: NeedsYouRowModel.Answering = .idle
    @State private var typing = false
    @State private var typed = ""
    /// The answer field has the keyboard as soon as Answer… opens it: the
    /// first keystrokes went nowhere until it was clicked (checklist F4).
    @FocusState private var answerFocused: Bool

    var body: some View {
        let offered = NeedsYouRowModel.buttons(for: item, canAct: canAct)
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(NeedsYouRowModel.group(item))
                    .font(.footnote.weight(.semibold))
                    .textCase(.uppercase)
                    .foregroundStyle(.secondary)
                Text(NeedsYouRowModel.subject(item))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: 0)
                if let since = item.since {
                    Text(since, style: .relative)
                        .font(.footnote)
                        .foregroundStyle(.tertiary)
                }
            }
            Text(item.question)
                .font(.body.weight(.medium))
                .lineLimit(2)
            if let detail = item.detail, !detail.isEmpty {
                Text(detail)
                    .font(.subheadline.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .textSelection(.enabled)
            }
            HStack(spacing: 6) {
                // Whose it is, on someone else's (ov-633), in the room the
                // buttons leave.
                if let whose = item.othersName {
                    Label(whose, systemImage: "person")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .identified("needs-you-whose")
                }
                Spacer(minLength: 0)
                if answering == .sending {
                    ProgressView().controlSize(.small)
                } else {
                    ForEach(offered.buttons, id: \.self) { button($0) }
                    if !offered.more.isEmpty {
                        Menu("More") {
                            ForEach(offered.more, id: \.self) { choice in
                                if case .decide(let option) = choice {
                                    SwiftUI.Button(option) { decide(option) }
                                }
                            }
                        }
                        .fixedSize()
                    }
                }
            }
            if typing {
                HStack {
                    TextField("Your answer", text: $typed)
                        .textFieldStyle(.roundedBorder)
                        .focused($answerFocused)
                        .onSubmit { sendTyped() }
                        // Esc closes it, keeping what was typed for the
                        // next Answer… (checklist F4).
                        .onExitCommand { typing = false }
                        // Once it's in the window: focus set in the same
                        // update that inserts the field is dropped.
                        .task { answerFocused = true }
                    SwiftUI.Button("Send") { sendTyped() }
                        .disabled(typed.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            if case .refused(let line) = answering {
                Text(line)
                    .font(.subheadline)
                    .foregroundStyle(.red)
            }
        }
        .padding(Spacing.inset)
        // A group on the page's card: the inset fill is its edge, with no stroke.
        .surface(.inset, in: .card)
        .contentShape(Rectangle())
        .onTapGesture(perform: onOpen)
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder
    private func button(_ button: NeedsYouRowModel.Button) -> some View {
        switch button {
        case .ask(let option, let title, let destructive, let primary):
            if primary {
                SwiftUI.Button(title) { answer(option) }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
            } else {
                SwiftUI.Button(title, role: destructive ? .destructive : nil) { answer(option) }
                    .controlSize(.small)
            }
        case .decide(let option):
            SwiftUI.Button(option) { decide(option) }.controlSize(.small)
        case .answerTyped:
            SwiftUI.Button("Answer…") { typing = true }.controlSize(.small)
        case .review:
            SwiftUI.Button("Review", action: onOpen).controlSize(.small)
        case .open:
            SwiftUI.Button("Open", action: onOpen).controlSize(.small)
        case .signIn(let title):
            SwiftUI.Button(title, action: onSignIn)
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .identified("needs-you-sign-in")
        case .github(let link):
            SwiftUI.Button("Open on GitHub") {
                if let url = NeedsYouRowModel.githubLink(link) { NSWorkspace.shared.open(url) }
            }
            .controlSize(.small)
        case .githubReview(let link):
            SwiftUI.Button("Review on GitHub") {
                if let url = NeedsYouRowModel.githubLink(link) { NSWorkspace.shared.open(url) }
            }
            .controlSize(.small)
        }
    }

    private func answer(_ option: String) {
        answering = .sending
        Task {
            if let refusal = await onAnswerAsk(option) {
                answering = NeedsYouRowModel.refused(refusal, item: item)
            } else {
                // It leaves on the next `needs_you_changed`; until then it
                // keeps its spinner rather than offering the same answer
                // again, but not forever.
                await settle()
            }
        }
    }

    private func decide(_ body: String) {
        answering = .sending
        Task {
            answering = await onDecide(body) ? .sending : NeedsYouRowModel.decisionRefused
            if answering == .sending {
                typing = false
                await settle()
            }
        }
    }

    /// Wait for the item to leave, and bring the row back if it hasn't.
    private func settle() async {
        try? await Task.sleep(for: NeedsYouRowModel.settleTimeout)
        answering = NeedsYouRowModel.afterTimeout(answering)
    }

    private func sendTyped() {
        let body = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !body.isEmpty else { return }
        decide(body)
    }
}
