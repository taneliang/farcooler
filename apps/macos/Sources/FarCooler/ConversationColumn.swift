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
    /// is offered nothing.
    static func offers(_ state: State, canAct: Bool = true) -> [Offer] {
        guard canAct else { return [] }
        switch state {
        case .none: return OrchestratorHarness.allCases.map(Offer.start)
        case .starting(let slow): return slow ? [.replace] : []
        case .lost: return [.restart, .replace]
        case .live: return []
        }
    }

    /// Whether the orchestrator finished a turn nobody has seen: an unread
    /// dot on its workspace's row and in this column's header, never an
    /// inbox item (ruling 10). `done` is finished-and-unseen, and seeing the
    /// pane turns it idle, which clears the dot.
    static func unread(_ seat: BoardPane?) -> Bool {
        seat?.terminal.agent == .done
    }

    /// The sentence under "No Orchestrator" (spec §8).
    static let emptyExplanation =
        "An orchestrator runs this workspace’s board. It reads the charter, dispatches agents, and asks you when it needs a decision."
}

/// The conversation column's header: `Orchestrator`, its harness and status,
/// the unread dot, and its menu.
struct ConversationHeader: View {
    let seat: BoardPane?
    let charter: CharterAccess?
    let canAct: Bool
    var onReplace: (OrchestratorHarness) -> Void
    var onShowCharter: (URL) -> Void
    var onTogglePaneMode: () -> Void
    var onRestart: () -> Void
    /// Stop Being Orchestrator: the terminal keeps running as an ordinary
    /// one (`OrchestratorAdoption.steppedDown`).
    var onStepDown: () -> Void = {}
    /// Whether answering a decision wakes the agent; nil from a runner that
    /// can't, which draws no switch. Here since the old sidebar's workspace
    /// row, its only home before, went (ov-178).
    var wakeOnAnswer: Bool? = nil
    var onSetWakeOnAnswer: (Bool) -> Void = { _ in }

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: 8) {
            if let seat { StatusGlyph(status: seat.terminal.status) }
            Text("Orchestrator").font(ColumnHeader.font(.semibold))
            if let name = Self.agentName(seat) {
                Text(name)
                    .font(ColumnHeader.font())
                    .foregroundStyle(.secondary)
            }
            if ConversationColumn.unread(seat) {
                Circle()
                    .fill(GlancePalette.amber(scheme))
                    .frame(width: 6, height: 6)
                    .help("The orchestrator finished a turn you haven’t seen")
                    .accessibilityLabel("Unread")
            }
            Spacer(minLength: 0)
            // Drawn with no orchestrator too (ov-178): Show Charter and Wake
            // the Agent When You Answer are the workspace's, not the seat's.
            let items = Self.menu(hasSeat: seat != nil, charter: charter, wakeOnAnswer: wakeOnAnswer)
            if canAct, seat != nil || !items.isEmpty {
                Menu {
                    ForEach(items, id: \.self) { item in menuItem(item) }
                    if seat?.terminal.canSwitchPaneMode == true || seat?.terminal.isAgentPane == true {
                        Button(seat?.terminal.isAgentPane == true ? "Show as Terminal" : "Show as Chat", action: onTogglePaneMode)
                    }
                    if seat != nil {
                        Divider()
                        Button("Restart", action: onRestart)
                        Button("Stop Being Orchestrator", action: onStepDown)
                    }
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .help("Orchestrator actions")
            }
        }
        .padding(.horizontal, 12)
        .columnHeader()
    }
}

extension ConversationHeader {
    /// The workspace's items in the header's menu, in `WorkspaceMenu`'s
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
            // Disabled with the reason rather than left out: the item is
            // how anybody learns a charter exists.
            case .unavailable(let why)?: Button(item.title) {}.disabled(true).help(why)
            case nil: EmptyView()
            }
        case .wakeOnAnswer:
            Divider()
            Toggle(item.title, isOn: Binding(get: { wakeOnAnswer ?? false }, set: onSetWakeOnAnswer))
                .help("When you answer one of this board’s decisions, type the answer into the agent working that task, or else the orchestrator, once it’s idle.")
        case .showBoard, .startOrchestrator:
            EmptyView()
        }
    }

    /// The agent the header names beside "Orchestrator": its harness while
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
                .foregroundStyle(.secondary)
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
        .background(WorkspaceStyle.canvas)
    }
}

/// Every state but a live orchestrator, centered in the column.
struct ConversationPlaceholder: View {
    let state: ConversationColumn.State
    let offers: [ConversationColumn.Offer]
    var onStart: (OrchestratorHarness) -> Void
    var onRestart: () -> Void
    var onReplace: () -> Void
    /// The workspace's running terminals Use a Running Terminal… lists,
    /// with no orchestrator: `OrchestratorAdoption.candidates`. Offered
    /// wherever Start Orchestrator is.
    var candidates: [BoardPane] = []
    var onUse: (BoardPane) -> Void = { _ in }

    var body: some View {
        VStack(spacing: 10) {
            switch state {
            case .none:
                Text("No Orchestrator").font(.headline)
                Text(ConversationColumn.emptyExplanation)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 320)
            case .starting(let slow):
                // An agent starting: the app's status mark, not a spinner
                // (ov-177).
                StatusGlyph(status: .starting, size: .lone)
                Text("Starting Orchestrator…").font(.headline)
                if slow {
                    Text("This is taking longer than usual.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            case .lost:
                Text("The orchestrator stopped").font(.headline)
                Text("Restart picks its conversation up where it left off.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            case .live:
                EmptyView()
            }
            let starts = offers.compactMap { offer -> OrchestratorHarness? in
                if case .start(let harness) = offer { return harness }
                return nil
            }
            HStack(spacing: 8) {
                if !starts.isEmpty {
                    Menu("Start Orchestrator") {
                        ForEach(starts) { harness in Button(harness.title) { onStart(harness) } }
                    }
                    .fixedSize()
                    // For the claude already running in a shell here: it
                    // becomes the orchestrator without starting another.
                    if !candidates.isEmpty {
                        Menu("Use a Running Terminal…") {
                            ForEach(candidates, id: \.terminal.id) { pane in
                                Button(pane.terminal.label) { onUse(pane) }
                            }
                        }
                        .fixedSize()
                    }
                }
                if offers.contains(.restart) { Button("Restart", action: onRestart) }
                if offers.contains(.replace) { Button("Replace…", action: onReplace) }
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
