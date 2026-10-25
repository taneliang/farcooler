import AgentKit
import AppKit
import SwiftUI

// The plan as the canvas's home (ov-298, Concept A of
// .claude/agent/reports/ov-298/research-layout.md, the owner's pick): beside
// the orchestrator's chat column, the plan fills the canvas until something
// is opened in its place. Needs You first, this workspace's only and answered
// in place (the toolbar's tray stays every workspace's), then the overview:
// Now, Next Up, Themes, Pages and Landed Today, then Decided For You.
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
        // No pinned Now / Next band (ov-522): the sections and the title bar's
        // counts say the same, and its "N need you" repeated Needs You just
        // below. The sentence stays in the folded strip, on the phones and on
        // the watch.
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                PlanHomeContent(
                    plan: board.plan, statuses: board.board.statuses, activity: board.board.activity, needsYou: needsYou,
                    onOpen: onOpen
                )
                // No reading width since the sections are columns and tiles
                // (ov-503): the 680-pt cap went; only a very wide display stops.
                .frame(maxWidth: PlanMetrics.widest, alignment: .leading)
                .padding(.horizontal, PlanMetrics.gutter)
                .padding(.vertical, 2 * ColumnGrid.rhythm)
                .frame(maxWidth: .infinity, alignment: .topLeading)
            }
            .scrollBounceBehavior(.basedOnSize)
        }
        // The one card every destination sits on (ov-297), so a change to
        // its gutter or radius moves the canvas's home with the rest.
        .contentCard()
        // The chat column is the canvas's neighbor: it holds Start Orchestrator.
        .environment(\.besideOrchestratorColumn, true)
        .identified("plan-home")
    }
}

/// The plan's sections, for the home and the peek alike.
struct PlanHomeContent: View {
    @ObservedObject var plan: PlanStore
    let statuses: [String: TaskStatus]
    /// Each card's last move on the board, by task id.
    var activity: [String: Int64] = [:]
    let needsYou: PlanNeedsYou
    let onOpen: (PlanPage) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.section) {
            if !needsYou.items.isEmpty || !asks.isEmpty { needs }
            if plan.available {
                PlanOverviewView(
                    plan: plan, statuses: statuses, selected: nil, activity: activity,
                    needsYou: needsYou.items.count + asks.count, onOpen: onOpen)
                // The calls made for the owner (ov-304), last: they ask
                // nothing, and stand until the owner says otherwise.
                PlanRulingsSection(plan: plan)
            } else if needsYou.items.isEmpty {
                // A runner too old to keep a plan: said, once.
                PlanNotice(title: PlanWords.needsUpdate, detail: nil)
                    .identified("plan-needs-update")
            }
        }
    }

    /// The themes asking for the owner.
    private var asks: [PlanTheme] { plan.plan.shownThemes.filter { !$0.ownerAsk.isEmpty } }

    /// Needs You: this workspace's questions, decisions and reviews, each
    /// answered here, most urgent first.
    private var needs: some View {
        CollapsibleSection(
            "Needs You", id: "plan.needs", style: .navigator, tone: .attention,
            key: "board.plan.section.needs.\(plan.host).\(plan.workspace.id)", count: needsYou.items.count + asks.count
        ) {
            VStack(alignment: .leading, spacing: Spacing.group) {
                // A theme's ask, counted here as in the title bar and the
                // tree's Needs You (ov-321 review M3).
                ForEach(asks) { theme in
                    Button { onOpen(.theme(theme.id)) } label: {
                        VStack(alignment: .leading, spacing: Spacing.tight) {
                            Text(theme.name)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.quietInk)
                            PlanAsk(text: theme.ownerAsk)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .identified("plan-home-theme-ask-\(theme.name)")
                }
                ForEach(needsYou.items) { item in
                    NeedsYouItemRow(
                        item: item, canAct: needsYou.canAct(item), onOpen: { needsYou.onOpen(item) },
                        onAnswerAsk: { await needsYou.onAnswerAsk(item, $0) },
                        onDecide: { await needsYou.onDecide(item, $0) },
                        teamRunners: NeedsYou.teamRunners(in: needsYou.items))
                    .changeWashed(item.key)
                }
            }
            .padding(.top, NavigatorRhythm.air)
            .listChanges(needsYou.items.map { ListChangeRow(id: $0.key, signature: "\($0.kind)\u{1}\($0.question)") })
        }
        .identified("plan-needs-you")
    }
}

/// Now/Next, the card at the top of the chat while the canvas is folded:
/// what's moving and what's next, a line each, and what needs you as a count.
/// A click peeks the plan; a change washes it, as a changed row does. The
/// words are AgentKit's `PlanStrip` and its band, the model the phones draw
/// too (ov-343, ov-641), so a person with both reads one sentence. Named for
/// the row, because a type of this module called `PlanStrip` would hide
/// AgentKit's.
struct PlanStripRow: View {
    @ObservedObject var plan: PlanStore
    let needsYou: Int
    var statuses: [String: TaskStatus] = [:]
    let onPeek: () -> Void
    @Environment(\.colorScheme) private var scheme

    /// The strip's words, or nil with nothing to say. The orchestrator's state
    /// isn't one of them (this row has no mark for it), so any state gives the
    /// same words.
    static func words(_ model: PlanModel, needsYou: Int, statuses: [String: TaskStatus] = [:]) -> String? {
        let strip = PlanStrip(plan: model, needsYou: needsYou, orchestrator: .idle, statuses: statuses)
        return strip.isEmpty ? nil : strip.text
    }

    var body: some View {
        content
            // Folded with the navigator put away, nothing else asks for the
            // plan: the strip reads it, and follows it as it moves.
            .task(id: [plan.available ? 1 : 0, plan.generation]) {
                guard plan.available else { return }
                await plan.reloadIfMoved()
                await plan.readIfNeverRead()
            }
    }

    /// The strip as the phones draw it (`WorkspaceCards`): the band's Now and
    /// Next, a line each, and what needs you as an amber dot with its count.
    /// Always drawn, so an empty board says "No lanes moving" rather than
    /// leaving the row bare.
    @ViewBuilder private var content: some View {
        let strip = PlanStrip(plan: plan.plan, needsYou: needsYou, orchestrator: .idle, statuses: statuses)
        let words = strip.isEmpty ? nil : strip.text
        Button(action: onPeek) {
            HStack(spacing: Spacing.group) {
                Group {
                    if let band = strip.band {
                        VStack(alignment: .leading, spacing: 3) {
                            bandLine("Now", band.nowLine, quiet: band.now == nil, id: "plan-band-now")
                            bandLine("Next", band.nextLine, quiet: band.next == nil, id: "plan-band-next")
                        }
                    } else if let words {
                        Text(words).lineLimit(1).truncationMode(.tail)
                    } else {
                        Text("Plan").foregroundStyle(.quietInk)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if needsYou > 0 { badge(needsYou) }
                Image(systemName: "chevron.up")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.quietInk)
                    .accessibilityHidden(true)
            }
            .font(.system(size: WorkspaceStyle.PaneText.secondary))
            .padding(.horizontal, Spacing.inset)
            .padding(.vertical, Spacing.group)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            .changeWashed("strip")
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .surface(.floating, in: .floating)
        .listChanges([ListChangeRow(id: "strip", signature: strip.band?.text ?? words ?? "")])
        .help("Peek at the Plan (⌥⌘P)")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(strip.band != nil ? strip.bandAccessibilityLabel : "Plan: \(words ?? "nothing yet")")
        .accessibilityHint("Peeks at the plan")
        .accessibilityAddTraits(.isButton)
        .identified("plan-strip")
    }

    private func bandLine(_ label: String, _ words: String, quiet: Bool, id: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Text(label)
                .font(.system(size: 9, weight: .semibold))
                .textCase(.uppercase)
                .kerning(0.4)
                .foregroundStyle(.quietInk)
                .frame(width: 34, alignment: .leading)
            Text(words)
                .fontWeight(quiet ? .regular : .medium)
                .foregroundStyle(quiet ? AnyShapeStyle(.quietInk) : AnyShapeStyle(.primary))
                .lineLimit(1)
                .truncationMode(.tail)
        }
        .identified(id)
    }

    /// What needs you, as an amber dot with the number: the dot carries the
    /// color and the number the meaning.
    private func badge(_ count: Int) -> some View {
        HStack(spacing: 5) {
            Circle().fill(GlancePalette.amber(scheme)).frame(width: 7, height: 7)
            Text("\(count)").fontWeight(.semibold).monospacedDigit()
        }
        .accessibilityHidden(true)
        .identified("plan-strip-needs-you")
    }
}

/// What a bare Esc puts away before anything else in the window hears it
/// (ov-298): the plan peeked over the chat, else a navigator floated over
/// the canvas. Nil when there's neither, or for any other key.
enum OverlayEscape: Equatable {
    case peek
    case navigator

    static func puts(keyCode: UInt16, modifiers: NSEvent.ModifierFlags, peeking: Bool, floating: Bool) -> OverlayEscape? {
        guard keyCode == 53, modifiers.intersection(.deviceIndependentFlagsMask).isEmpty else { return nil }
        if peeking { return .peek }
        return floating ? .navigator : nil
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
                PlanHomeContent(plan: board.plan, statuses: board.board.statuses, activity: board.board.activity, needsYou: needsYou) { page in
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
