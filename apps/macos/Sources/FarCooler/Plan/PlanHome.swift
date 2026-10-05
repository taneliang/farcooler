import AgentKit
import SwiftUI

// The plan as the canvas's home (ov-298, Concept A of
// .claude/agent/reports/ov-298/research-layout.md, the owner's pick): beside
// the orchestrator's chat column, the plan fills the canvas until something
// is opened in its place. Needs You first, this workspace's only and answered
// in place (the toolbar's tray stays every workspace's), then the overview:
// Now, Next Up, Themes, Pages and Landed Today.
//
// A window too narrow for the canvas folds it away: the chat carries the
// plan's one-line strip (Concept C, 3.3), and a click on it or ⌥⌘P peeks the
// plan over the chat without resizing the terminal.

/// This workspace's Needs You, and how its rows answer.
struct PlanNeedsYou {
    var items: [NeedsYouItem] = []
    var canAct: (NeedsYouItem) -> Bool = { _ in false }
    var onOpen: (NeedsYouItem) -> Void = { _ in }
    var onAnswerAsk: (NeedsYouItem, String) async -> DaemonClient.AskRefusal? = { _, _ in nil }
    var onDecide: (NeedsYouItem, String) async -> Bool = { _, _ in false }
}

/// The canvas's home: the plan, on its card.
struct PlanHome: View {
    /// The board, watched for its statuses (whether a lane waits on the
    /// owner); its plan is `board.plan`.
    @ObservedObject var board: TaskBoardStore
    let needsYou: PlanNeedsYou
    let onOpen: (PlanPage) -> Void

    var body: some View {
        ScrollView {
            PlanHomeContent(plan: board.plan, statuses: board.board.statuses, needsYou: needsYou, onOpen: onOpen)
                .padding(.horizontal, ColumnGrid.a + ColumnGrid.step)
                .padding(.vertical, 2 * ColumnGrid.rhythm)
                .frame(maxWidth: .infinity, alignment: .topLeading)
        }
        .scrollBounceBehavior(.basedOnSize)
        .clipShape(.card)
        .surface(.content, in: .card, fill: WorkspaceStyle.paper)
        .padding(Gutter.window)
        .identified("plan-home")
    }
}

/// The plan's sections, for the home and the peek alike.
struct PlanHomeContent: View {
    @ObservedObject var plan: PlanStore
    let statuses: [String: TaskStatus]
    let needsYou: PlanNeedsYou
    let onOpen: (PlanPage) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.section) {
            if !needsYou.items.isEmpty { needs }
            if plan.available {
                PlanOverviewView(plan: plan, statuses: statuses, selected: nil, onOpen: onOpen)
            } else if needsYou.items.isEmpty {
                // A runner too old to keep a plan: said, once.
                PlanNotice(title: PlanWords.needsUpdate, detail: nil)
                    .identified("plan-needs-update")
            }
        }
    }

    /// Needs You: this workspace's questions, decisions and reviews, each
    /// answered here, most urgent first.
    private var needs: some View {
        CollapsibleSection(
            "Needs You", id: "plan.needs", style: .navigator, tone: .attention,
            key: "board.plan.section.needs.\(plan.host).\(plan.workspace.id)", count: needsYou.items.count
        ) {
            VStack(alignment: .leading, spacing: Spacing.group) {
                ForEach(needsYou.items) { item in
                    NeedsYouItemRow(
                        item: item, canAct: needsYou.canAct(item), onOpen: { needsYou.onOpen(item) },
                        onAnswerAsk: { await needsYou.onAnswerAsk(item, $0) },
                        onDecide: { await needsYou.onDecide(item, $0) })
                    .changeWashed(item.key)
                }
            }
            .padding(.top, NavigatorRhythm.air)
            .listChanges(needsYou.items.map { ListChangeRow(id: $0.key, signature: "\($0.kind)\u{1}\($0.question)") })
        }
        .identified("plan-needs-you")
    }
}

/// The plan in one line, over the chat's foot while the canvas is folded:
/// "2 need you · mac-ux In review · next: close-cards". A click peeks the
/// plan; a change washes it, as a changed row does.
struct PlanStrip: View {
    @ObservedObject var plan: PlanStore
    let needsYou: Int
    let onPeek: () -> Void

    /// The strip's words, or nil with nothing to say.
    static func words(_ model: PlanModel, needsYou: Int) -> String? {
        var parts: [String] = []
        if needsYou > 0 { parts.append(needsYou == 1 ? "1 needs you" : "\(needsYou) need you") }
        for lane in model.working.prefix(2) { parts.append("\(lane.name) \(PlanWords.state(lane.state))") }
        if let next = model.nextUp.first { parts.append("next: \(next.name)") }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    var body: some View {
        if plan.planned || needsYou > 0, let words = Self.words(plan.plan, needsYou: needsYou) {
            Button(action: onPeek) {
                HStack(spacing: Spacing.tight + 2) {
                    Image(systemName: "map").accessibilityHidden(true)
                    Text(words).lineLimit(1).truncationMode(.tail)
                }
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .padding(.horizontal, Spacing.inset)
                .padding(.vertical, Spacing.tight + 2)
                .changeWashed("strip")
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .surface(.floating, in: Capsule())
            .listChanges([ListChangeRow(id: "strip", signature: words)])
            .help("Peek at the Plan (⌥⌘P)")
            .accessibilityLabel("Plan: \(words)")
            .accessibilityHint("Peeks at the plan")
            .padding(.bottom, 2 * Spacing.inset)
            .identified("plan-strip")
        }
    }
}

/// The plan peeked over the chat: a card over its middle, the terminal
/// under it the size it was. Esc, ⌥⌘P or a click beside it puts it away.
struct PlanPeek: View {
    @ObservedObject var board: TaskBoardStore
    let needsYou: PlanNeedsYou
    let onOpen: (PlanPage) -> Void
    let onClose: () -> Void

    var body: some View {
        ZStack {
            // A click beside the card puts it away; it draws nothing.
            Color.clear.contentShape(Rectangle()).onTapGesture(perform: onClose)
            ScrollView {
                PlanHomeContent(plan: board.plan, statuses: board.board.statuses, needsYou: needsYou) { page in
                    onClose()
                    onOpen(page)
                }
                .padding(Spacing.inset + Spacing.group)
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(maxWidth: 560)
            .clipShape(.card)
            .surface(.content, in: .card, fill: WorkspaceStyle.paper)
            .padding(.horizontal, 3 * Spacing.inset)
            .padding(.vertical, 4 * Spacing.inset)
        }
        .onExitCommand(perform: onClose)
        .identified("plan-peek")
    }
}
