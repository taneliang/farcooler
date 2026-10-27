import AgentKit
import SwiftUI

// A workspace's conversation column: its orchestrator, and every state around
// one (spec §4.10 and §8).
//
// The states and what each offers are worked out here as values, so
// `ConversationColumnTests` pins them; the view draws them and decides
// nothing.

enum ConversationColumn {
    /// How long a start may go unconfirmed before the column says so and
    /// offers Replace…: the seat can stick (CLI map §7.11).
    static let slowStart: TimeInterval = 30

    enum State: Equatable {
        /// No orchestrator runs this workspace, and none is starting.
        case none
        /// A start this app asked for or the runner is making; `slow` once
        /// it has gone unconfirmed for `slowStart`.
        case starting(slow: Bool)
        /// The orchestrator's pane was lost, or exited: its last screen,
        /// dimmed.
        case lost
        /// Running.
        case live
    }

    /// What the column can offer.
    enum Offer: Equatable, Hashable {
        case start(OrchestratorHarness)
        case restart
        case replace
    }

    /// The column's state, from the seat the runner names (`seat`), whether
    /// this app has a start in flight, and when a start began, if one did.
    static func state(seat: BoardPane?, isStarting: Bool, startedAt: Date?, now: Date) -> State {
        let slow = startedAt.map { now.timeIntervalSince($0) >= slowStart } ?? false
        guard let seat else { return isStarting ? .starting(slow: slow) : .none }
        switch StateKind.parse(seat.terminal.state) {
        case .starting: return .starting(slow: slow)
        case .lost, .exited, .error: return .lost
        default: return .live
        }
    }

    /// What `state` offers, in order. A read-only runner (`canAct` false)
    /// is offered nothing, and only the agents that are installed
    /// (`choices`) are offered to start.
    static func offers(_ state: State, canAct: Bool = true, choices: OrchestratorChoices = .all) -> [Offer] {
        guard canAct else { return [] }
        switch state {
        case .none: return choices.installed.map(Offer.start)
        case .starting(let slow): return slow ? [.replace] : []
        case .lost: return [.restart, .replace]
        case .live: return []
        }
    }

    /// Whether the orchestrator finished a turn nobody has seen: an unread
    /// dot on its workspace’s row and on the title bar’s mark, never an
    /// inbox item (ruling 10). `done` is finished-and-unseen, and seeing the
    /// pane turns it idle, which clears the dot.
    static func unread(_ seat: BoardPane?) -> Bool {
        seat?.terminal.agent == .done
    }

    /// What the column says with no orchestrator (spec §8, ov-519): the
    /// invitation, not a bare "No Orchestrator". The job first, then the two
    /// things a person does with one, in rows rather than a paragraph
    /// (ov-205); the button under it starts one.
    static let emptyCopy = EmptyStateCopy(
        lede: FirstRunCopy.Conversation.inviteLede,
        rows: [
            .init(symbol: "bubble.left", text: FirstRunCopy.Conversation.inviteRowTell),
            .init(symbol: "hand.raised", text: FirstRunCopy.Conversation.inviteRowAsk),
        ])
}

/// The orchestrator's menu: what the conversation column's header held
/// until the header went (ov-214), and now the title bar's status area's
/// orchestrator ⌄ (`TitleStatusView`). Replace Orchestrator, Show Charter
/// and Wake the Agent When You Answer, the chat switch, Restart and Stop
/// Being Orchestrator; with none running and a runner that can act, Start
/// Orchestrator.
///
/// The menu's items, not a menu: whoever draws the menu draws its label.
struct OrchestratorMenu: View {
    let seat: BoardPane?
    let charter: CharterAccess?
    let canAct: Bool
    var onReplace: (OrchestratorHarness) -> Void
    var onShowCharter: (URL) -> Void
    /// Show Charter on another machine's runner: its text in a read-only
    /// sheet (`CharterSheet`).
    var onReadCharter: () -> Void = {}
    /// The runner, as `DaemonClient.target` names it: empty for this Mac's,
    /// the only one whose panes get the conversation view.
    var target = ""
    var onRestart: () -> Void
    /// Stop Being Orchestrator: the terminal keeps running as an ordinary
    /// one (`OrchestratorAdoption.steppedDown`).
    var onStepDown: () -> Void = {}
    /// Whether answering a decision wakes the agent; nil from a runner that
    /// can't, which draws no switch. Here since the old sidebar's workspace
    /// row, its only home before, went (ov-178).
    var wakeOnAnswer: Bool? = nil
    var onSetWakeOnAnswer: (Bool) -> Void = { _ in }
    /// Whether this Mac may change the workspace's settings that act as
    /// nobody, the wake switch among them: the owner and a Maintainer
    /// (ov-784), whether or not they lead it. Nobody else gets the switch.
    /// Nil while who this Mac is isn't known: the switch goes with `canAct`,
    /// as it did before.
    var keepsSettings: Bool?
    /// With no orchestrator: the harnesses Start Orchestrator offers
    /// (`ConversationColumn.offers`), and what starting one does.
    var starts: [OrchestratorHarness] = []
    var onStart: (OrchestratorHarness) -> Void = { _ in }

    var body: some View {
        if canAct, seat == nil, !starts.isEmpty {
            Menu("Start Orchestrator") {
                ForEach(starts) { harness in Button(harness.title) { onStart(harness) } }
            }
        }
        // Drawn with no orchestrator too (ov-178): Show Charter and Wake
        // the Agent When You Answer are the workspace's, not the seat's.
        ForEach(
            Self.items(
                canAct: canAct, hasSeat: seat != nil, charter: charter, wakeOnAnswer: wakeOnAnswer,
                keepsSettings: keepsSettings),
            id: \.self
        ) { item in
            menuItem(item)
        }
        if canAct {
            if let seat {
                OrchestratorViewItem(terminal: seat.terminal, target: target)
            }
            if seat != nil {
                Divider()  // style-exempt: menu
                Button("Restart", action: onRestart)
                Button("Stop Being Orchestrator", action: onStepDown)
            }
        }
    }
}

extension OrchestratorMenu {
    /// What the menu draws: the workspace's items for a caller that can act,
    /// and for one that can't (a member can't restart, replace or step down
    /// anyone's orchestrator) only Show Charter, which every member reads
    /// (ov-755) and which opens as a sheet on another machine's runner. The
    /// wake switch is the owner's and a Maintainer's (`keepsSettings`,
    /// ov-784), whoever acts on the orchestrator: a lead who isn't one would
    /// be refused it, and a Maintainer who doesn't lead isn't. Not known
    /// (nil), it goes with `canAct`.
    nonisolated static func items(
        canAct: Bool, hasSeat: Bool, charter: CharterAccess?, wakeOnAnswer: Bool?, keepsSettings: Bool? = nil
    ) -> [WorkspaceMenu.Item] {
        let wake = (keepsSettings ?? canAct) ? wakeOnAnswer : nil
        if canAct { return menu(hasSeat: hasSeat, charter: charter, wakeOnAnswer: wake) }
        return (charter == .read ? [.showCharter] : []) + (wake == nil ? [] : [.wakeOnAnswer])
    }

    /// The workspace's items in the menu, in `WorkspaceMenu`'s
    /// order: Replace Orchestrator with one seated (starting one is the
    /// column's own placeholder's), Show Charter where this runner says
    /// where it is, and Wake the Agent When You Answer from a runner that
    /// said whether it's on.
    nonisolated static func menu(hasSeat: Bool, charter: CharterAccess?, wakeOnAnswer: Bool?) -> [WorkspaceMenu.Item] {
        WorkspaceMenu.items(hasBoard: false, hasOrchestrator: hasSeat, wakeOnAnswer: wakeOnAnswer).filter { item in
            switch item {
            case .showBoard, .startOrchestrator: return false
            case .showCharter: return charter != nil
            case .replaceOrchestrator, .wakeOnAnswer: return true
            }
        }
    }

    /// One of `menu`'s items, drawn.
    @ViewBuilder
    fileprivate func menuItem(_ item: WorkspaceMenu.Item) -> some View {
        switch item {
        case .replaceOrchestrator:
            // An ellipsis on each harness: choosing one asks first, because
            // the orchestrator running now closes.
            Menu(item.title) {
                ForEach(OrchestratorHarness.allCases) { harness in
                    Button("\(harness.title)…") { onReplace(harness) }
                }
            }
        case .showCharter:
            switch charter {
            case .open(let url)?: Button(item.title) { onShowCharter(url) }
            case .read?: Button(item.title, action: onReadCharter)
            // Disabled with the reason rather than left out: the item is
            // how anybody learns a charter exists.
            case .unavailable(let why)?: Button(item.title) {}.disabled(true).help(why)
            case nil: EmptyView()
            }
        case .wakeOnAnswer:
            Divider()  // style-exempt: menu section break
            Toggle(item.title, isOn: Binding(get: { wakeOnAnswer ?? false }, set: onSetWakeOnAnswer))
                .help("When you answer one of this board’s decisions, type the answer into the agent working that task, or else the orchestrator, once it’s idle.")
        case .showBoard, .startOrchestrator:
            EmptyView()
        }
    }

    /// The agent the navigator's orchestrator row names: its harness while
    /// one runs, and nothing otherwise. A lost pane reports no process, which
    /// `Terminal.name(of:)` calls "shell", and "Orchestrator · shell" named
    /// the one thing it wasn't (checklist O7).
    static func agentName(_ seat: BoardPane?) -> String? {
        guard let seat, seat.terminal.runsAgent else { return nil }
        return Terminal.name(of: seat.terminal.preset)
    }
}

/// A terminal that isn't the orchestrator, sharing its window, and the
/// one click that moves it out: see `WorkspaceScreen.sharers`.
struct SharedWindowNotice: View {
    let title: String
    let canAct: Bool
    let onMove: () -> Void

    static func sentence(_ title: String) -> String { "\(title) shares the orchestrator’s window." }

    var body: some View {
        HStack(spacing: 8) {
            Text(Self.sentence(title))
                .font(.system(size: 11))
                .foregroundStyle(.quietInk)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer(minLength: 6)
            if canAct {
                Button("Move to Its Own Window", action: onMove)
                    .controlSize(.small)
                    .fixedSize()
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
    }
}

/// Every state but a live orchestrator, centered in the column.
struct ConversationPlaceholder: View {
    let state: ConversationColumn.State
    let offers: [ConversationColumn.Offer]
    var onRestart: () -> Void
    var onReplace: () -> Void
    /// The workspace's running terminals Use a Running Terminal… lists,
    /// with no orchestrator: `OrchestratorAdoption.candidates`. Offered
    /// wherever Start Orchestrator is.
    var candidates: [BoardPane] = []
    var onUse: (BoardPane) -> Void = { _ in }
    /// Start Orchestrator's way to start, with the agents that are installed
    /// (ov-519): nil where this runner offers none.
    @Environment(\.orchestratorInvite) private var invite
    /// Drawn on its own card, as every destination is (ov-297). Off only over
    /// a lost orchestrator's dimmed last screen, which is cards already.
    var carded = true
    /// What it does first, under "Starting Orchestrator…", for one the
    /// Charter screens started (`CharterStart`); nil for any other start.
    var startingLine: String? = nil

    var body: some View {
        VStack(spacing: 10) {
            switch state {
            case .none:
                Text(FirstRunCopy.Conversation.inviteTitle).font(.headline)
                EmptyStateRows(copy: ConversationColumn.emptyCopy)
                    .font(.callout)
                    .foregroundStyle(.quietInk)
            case .starting(let slow):
                // An agent starting: the app's status mark, not a spinner
                // (ov-177).
                StatusGlyph(status: .starting, size: .lone)
                Text("Starting Orchestrator…").font(.headline)
                if let startingLine {
                    Text(startingLine)
                        .font(.callout)
                        .foregroundStyle(.quietInk)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: 320)
                        .identified("orchestrator-starting-line")
                }
                if slow {
                    Text("This is taking longer than usual.")
                        .font(.callout)
                        .foregroundStyle(.quietInk)
                }
            case .lost:
                Text("Orchestrator Stopped").font(.headline)
                Text("Restart picks its conversation up where it left off.")
                    .font(.callout)
                    .foregroundStyle(.quietInk)
            case .live:
                EmptyView()
            }
            if state == .none, let invite, invite.canAct {
                // Installed agents start; the rest are said, and with none the
                // fix is beside the button (`StartOrchestratorMenu`).
                StartOrchestratorMenu(invite: invite)
                // For the claude already running in a shell here: it
                // becomes the orchestrator without starting another.
                if !candidates.isEmpty, invite.choices != .none {
                    Menu("Use Running Terminal") {
                        ForEach(candidates, id: \.terminal.id) { pane in
                            Button(pane.terminal.label) { onUse(pane) }
                        }
                    }
                    .fixedSize()
                }
            }
            HStack(spacing: 8) {
                if offers.contains(.restart) { Button("Restart", action: onRestart) }
                if offers.contains(.replace) { Button("Replace…", action: onReplace) }
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .modifier(CardIf(on: carded))
    }
}

/// `contentCard()` when `on`.
struct CardIf: ViewModifier {
    let on: Bool

    @ViewBuilder func body(content: Content) -> some View {
        if on { content.contentCard() } else { content }
    }
}

/// The view switch the orchestrator's menu names (ov-411): the conversation
/// view where it's offered (`NativeAgents.offers`), the same switch the pane's
/// own button and ⌃⌘T make.
enum OrchestratorViewSwitch: Equatable {
    /// From the terminal to the conversation view, and back.
    case showConversation, showTerminalFromConversation

    var title: String {
        switch self {
        case .showConversation: return "Show Conversation"
        case .showTerminalFromConversation: return "Show Terminal"
        }
    }

    /// The one switch for a pane, or none where the view isn't offered.
    static func of(offersConversation: Bool, conversationShown: Bool) -> OrchestratorViewSwitch? {
        guard offersConversation else { return nil }
        return conversationShown ? .showTerminalFromConversation : .showConversation
    }
}

/// The orchestrator menu's view switch, drawn. Reads the pane's model itself,
/// so its title is true when the menu opens, whichever way the pane was
/// switched last.
struct OrchestratorViewItem: View {
    let terminal: Terminal
    let target: String
    @ObservedObject private var agents = NativeAgents.shared

    var body: some View {
        if agents.offers(terminal, target: target) {
            ConversationSwitchItem(
                model: agents.model(for: terminal.id, target: target, program: NativeAgents.agent(of: terminal)))
        } else if let reason = agents.unavailable(terminal, target: target), reason.isAboutTheRunner {
            // Dimmed, saying why (ov-443), rather than gone without a word:
            // on a line of its own, since a menu item's help shows only on a
            // hover nobody thinks to try (ov-830), and with the check it
            // failed, as `terminal conversation-check` names it.
            Button(OrchestratorViewSwitch.showConversation.title) {}
                .disabled(true)
                .help(reason.note)
            Text(reason.note)
        }
    }
}

/// The item once the view is offered: it watches the pane's model, so its
/// title follows the switch.
struct ConversationSwitchItem: View {
    @ObservedObject var model: NativePaneModel

    var body: some View {
        if let item = OrchestratorViewSwitch.of(offersConversation: true, conversationShown: model.showsNative) {
            Button(item.title) { model.showsNative.toggle() }
        }
    }
}
