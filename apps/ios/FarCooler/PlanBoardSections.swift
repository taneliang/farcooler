import FarCoolerClient
import SwiftUI

// The Plan view on a phone (ov-268 design 6.5): what the board's list shows
// when Plan is chosen. Next Up first, because it's what the owner can't see
// anywhere else; then the tickets working now, the themes, and what landed
// today. A ticket or a theme opens its page, pushed.
//
// EXPERIMENTAL and opt-in, as the layer is: Tasks is the default, the control
// only appears on a runner that advertises `board_plan`, and it switches only
// the task list's sections. The Unread strip, the waiting count, the pull to
// refresh and everything above and below stay where they are.
//
// Every rule is AgentKit's (`PlanModel`, `PlanWords`, `PlanReadState`); this
// draws, in iOS's own idiom: title case, system type, and color only for a
// ticket that's stale or waiting on you.

/// What `WorkspaceBoardList` needs to draw a board's plan.
struct PlanBoardHook {
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    let reads: PlanReads
    /// Whether this runner advertises `board_plan`.
    let keeps: Bool
    /// Read the plan, once or again.
    let read: () async -> Void
    let onOpen: (PhonePlanPage) -> Void
    /// The board's statuses by task id: a ticket waits on you when its
    /// card needs a decision.
    let statuses: [String: TaskStatus]
    /// The board's orchestrator pages (ov-285), read with the plan. Nil on a
    /// runner without `board_pages`, which shows no Pages section.
    var pages: PageReads?
    /// Whether this runner advertises `board_rulings` (ov-304): without it,
    /// no Decided For You.
    var keepsRulings = false
    /// What the owner's Keep, Keep All, Reverse and Discuss do (ov-333).
    var rulingActions = PhoneRulingActions()
    /// What the sections are decided from besides the plan (ov-503).
    var facts = PlanFacts()
    /// The sidebar's two lists for a filter, for the pages Themes and Tickets
    /// push; nil where there are none to push.
    var outline: ((OneTreeFilter) -> SidebarOutline)?
    /// A card, ticket or pane chosen in a list the Plan pushes: the phone's
    /// stack opens it.
    var onRoute: (PhoneRoute) -> Void = { _ in }
    /// Themes and Tickets as two rows that push lists, which a phone has in
    /// place of a sidebar. The iPad's sidebar holds both.
    var pushesLists = true
    /// How many tiles wide the themes grid is: one on a phone, two on an
    /// iPad's plan column.
    var tileColumns = 1
    /// Draw the glance band over the sections. A caller that draws it
    /// itself, above something else, turns this off.
    var showsBand = true
    /// The sections drawn, nil for all of them. The iPhone's Plan sheet,
    /// whose tree lists every theme and card, leaves Themes out (ov-616).
    var kinds: Set<WireSectionKind>?
}

/// What the Plan's sections are decided from besides the plan itself: the
/// workspace's Needs You count, when each card last moved, and whether the
/// runner lists agents (`board_line`).
struct PlanFacts {
    var needsYou = 0
    var activity: [String: Int64] = [:]
    var listsAgents = false
}

/// The Rust core's plan view (ov-1016), asked through the client library this
/// app links: AgentKit can't link it, since the watch and the widgets compile
/// it without, so the app hands it the call.
enum PlanViewBridge {
    private static let installed: Void = PlanViewPresenter.install { PlanViewPresenter.call(farcooler_client_plan_view, $0) }

    static func install() { _ = installed }

    static func view(plan: PlanModel, facts: PlanFacts, statuses: [String: TaskStatus]) -> WirePlanView? {
        install()
        return WirePlanView.build(
            plan: plan, needsYou: facts.needsYou, statuses: statuses, activity: facts.activity,
            agentsSupported: facts.listsAgents)
    }

    /// The plan column (ov-874), from the same view.
    static func column(
        plan: PlanModel, needsYou: Int = 0, statuses: [String: TaskStatus], titles: [String: String]
    ) -> WireColumn? {
        install()
        return WireColumn.of(plan, needsYou: needsYou, statuses: statuses, titles: titles)
    }
}

extension Connection {
    /// Whether the runner lists the agents working for a board (`board_line`);
    /// without it the Agents section isn't drawn.
    var listsAgents: Bool { knownBuild?.can(.boardLine) == true }

    func planFacts(_ summary: WorkspaceSummary) -> PlanFacts {
        PlanFacts(
            needsYou: workspaceNeedsYou(summary), activity: boards[summary.id]?.activity ?? [:], listsAgents: listsAgents)
    }
}

/// The shortest a row is in the Plan's lists. A list's default is 44 pt, which
/// is right for a tappable row and wrong for a group label, whose words sat in
/// a row twice their height (ov-515); every row of the Plan's has content
/// taller than this, so only the labels change.
enum PlanRowHeight {
    static let minimum: CGFloat = 24
    static let standard: CGFloat = 44
}

/// The Tasks | Plan control, a segmented picker as Apple's own lists have it.
struct PlanSwitch: View {
    @Binding var showsPlan: Bool

    var body: some View {
        Picker("Board", selection: $showsPlan) {
            Text("Tasks").tag(false)
            Text("Plan").tag(true)
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .accessibilityIdentifier("plan-switch")
    }
}

/// The Plan view's sections, in a board's list.
struct PlanBoardSections: View {
    let hook: PlanBoardHook
    @ObservedObject var reads: PlanReads
    @ObservedObject private var pages: PageReads
    @State private var landedOpen = false
    @State private var pastRulingsOpen = false

    init(hook: PlanBoardHook) {
        self.hook = hook
        reads = hook.reads
        pages = hook.pages ?? PageReads()
    }

    var body: some View {
        switch reads.state(hook.summary.id) {
        case .none, .loading:
            Section {
                ProgressView()
                    .frame(maxWidth: .infinity, alignment: .center)
                    .accessibilityLabel("Reading the plan")
                    .accessibilityIdentifier("plan-loading")
            }
        case .needsUpdate:
            Section { PlanNotice(title: PlanWords.needsUpdate, detail: nil).accessibilityIdentifier("plan-needs-update") }
        case .reconnecting:
            // The runner isn't answering yet (ov-631): not a broken plan.
            // The resync that follows the link reads again; Try Again asks now.
            Section {
                HStack(spacing: 8) {
                    ProgressView()
                    Text(PlanWords.reconnecting).foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("plan-reconnecting")
                Button(PlanWords.tryAgain) { Task { await hook.read() } }
                    .accessibilityIdentifier("plan-retry")
            }
        case .unavailable:
            Section {
                PlanNotice(title: PlanWords.couldntRead, detail: nil)
                    .accessibilityIdentifier("plan-unavailable")
                Button(PlanWords.tryAgain) { Task { await hook.read() } }
                    .accessibilityIdentifier("plan-retry")
            }
        case .loaded(let plan):
            sections(plan)
        }
    }

    @ViewBuilder
    private func sections(_ plan: PlanModel) -> some View {
        if plan.isEmpty {
            // Rulings alone plan nothing (review 1005a F1), but they're
            // something to show, so the notice stands down for them.
            if !(hook.keepsRulings && !plan.rulings.isEmpty) {
                Section {
                    PlanNotice(title: PlanWords.nothingPlanned, detail: PlanWords.nothingPlannedDetail)
                        .accessibilityIdentifier("plan-empty")
                }
            }
            pagesSection(plan)
            PlanRulingsSection(plan: plan, keeps: hook.keepsRulings, actions: hook.rulingActions, pastOpen: $pastRulingsOpen)
        } else {
            // The Rust core's view decides the sections and their words
            // (ov-1016); a plan the runner sent always has one.
            let view = PlanViewBridge.view(plan: plan, facts: hook.facts, statuses: hook.statuses)
            if let view {
                if hook.showsBand {
                    Section {
                        PhoneGlanceBand(band: PlanGlanceSentence(view: view), needsYou: hook.facts.needsYou)
                            .padding(.vertical, 2)
                    }
                }
                PlanSectionsList(view: view, plan: plan, hook: hook)
            } else {
                // The core couldn't read the plan: say so, as Android and the web do.
                Section { PlanNotice(title: PlanWords.couldntRead, detail: nil).accessibilityIdentifier("plan-unreadable") }
            }
            // Behind `board_cost`: a runner without it sends no cost (ov-307).
            if let cost = plan.cost, cost.isWorthShowing {
                Section {
                    PlanCostBlock(cost: cost)
                } header: {
                    PlanHeader(title: "Cost", count: nil).accessibilityIdentifier("plan-cost-header")
                }
            }
            pagesSection(plan)
            if let view, let landed = view.section(.landedToday) {
                Section {
                    if landedOpen {
                        ForEach(view.landedToday) { card in PlanCardRow(card: card) { hook.onOpen(.ticket(card.id)) } }
                    }
                } header: {
                    Button { withAnimation { landedOpen.toggle() } } label: {
                        PlanHeader(title: landed.title, count: Int(landed.count), open: landedOpen)
                    }
                    .buttonStyle(.plain)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("\(landed.title), \(landed.count)")
                    .accessibilityValue(landedOpen ? "Expanded" : "Collapsed")
                    .accessibilityIdentifier("plan-landed-header")
                }
            }
            // Last, as on the Mac: rulings ask nothing, and stand until the
            // owner says otherwise.
            PlanRulingsSection(plan: plan, keeps: hook.keepsRulings, actions: hook.rulingActions, pastOpen: $pastRulingsOpen)
        }
    }
}

extension PlanBoardSections {
    /// After Themes, as on the Mac (design 6.1): pages of their own, and
    /// anchored ones whose theme is gone. Nothing on a runner without pages.
    @ViewBuilder
    fileprivate func pagesSection(_ plan: PlanModel) -> some View {
        if hook.pages != nil {
            PlanPagesSection(state: pages.state(hook.summary.id), plan: plan, read: hook.read, onOpen: hook.onOpen)
        }
    }
}

/// A section's title and count, as the board's status headers read: the name
/// and its count, trailing and tertiary, with a chevron when it opens.
struct PlanHeader: View {
    let title: String
    /// Nil when it isn't known, as over a read that failed: no count is
    /// drawn, never a zero that says there's nothing.
    let count: Int?
    /// Nil when the section doesn't open.
    var open: Bool?
    /// A button drawn before the count, as Decided For You's Keep All
    /// (ov-637): before it, so the count isn't read as the button's.
    var action: Action?
    /// The header's identifier, for a header with an `action`: put on its
    /// words and count, not around the button, whose own identifier an
    /// outer one would replace (ov-637).
    var id: String?

    struct Action {
        let title: String
        let id: String
        let run: () -> Void
    }

    var body: some View {
        HStack(spacing: PaneMetrics.tight) {
            Text(title).identified(id)
            Spacer()
            if let action {
                Button(action.title, action: action.run)
                    .font(.subheadline)
                    .fontWeight(.regular)
                    .tint(.secondary)
                    .accessibilityIdentifier(action.id)
                    .padding(.trailing, PaneMetrics.step)
            }
            if let count {
                Text("\(count)")
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(.tertiary)
                    .identified(id)
            }
            if let open {
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .rotationEffect(.degrees(open ? 90 : 0))
                    .foregroundStyle(.tertiary)
            }
        }
        .textCase(nil)
        .font(.subheadline.weight(.semibold))
        .foregroundStyle(.secondary)
    }
}

private extension View {
    /// `accessibilityIdentifier(id)`, or nothing when there's none.
    @ViewBuilder func identified(_ id: String?) -> some View {
        if let id { accessibilityIdentifier(id) } else { self }
    }
}

/// "Nothing is planned on this board yet.", and why there would be.
struct PlanNotice: View {
    let title: String
    let detail: String?

    var body: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.tight) {
            Text(title).font(.subheadline.weight(.semibold))
            if let detail {
                Text(detail).font(.footnote).foregroundStyle(.secondary)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A ticket stage's glyph: neutral, a shape to scan by.
enum PlanGlyph {
    static func name(_ stage: TicketStage, fixing: Bool = false) -> String {
        switch stage {
        case .waiting: "clock"
        case .building: fixing ? "wrench.and.screwdriver" : "hammer"
        case .reviewing: "eye"
        case .needsYou: "person.crop.circle.badge.exclamationmark"
        case .landing: "arrow.down.to.line"
        case .landed: "checkmark.circle"
        case .dropped: "xmark.circle"
        case .unknown: "questionmark.circle"
        }
    }
}

/// "● Needs you: …": the one colored mark a theme has, with words.
struct PlanAsk: View {
    let text: String
    var lines: Int?
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Circle()
                .fill(GlancePalette.amber(scheme))
                .frame(width: 6, height: 6)
                .accessibilityHidden(true)
            Text("\(Text("Needs you: ").fontWeight(.medium).foregroundStyle(GlancePalette.amber(scheme)))\(text)")
                .font(.footnote)
                .lineLimit(lines)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.top, PaneMetrics.tight)
        .accessibilityIdentifier("plan-ask")
    }
}

/// A theme's cards by status, left to right, in neutral fills: done, in
/// review, in progress, not started. Canceled cards aren't drawn
/// (`PlanWords.segments`).
struct PlanProgressBar: View {
    let counts: PlanCounts

    var body: some View {
        let parts = PlanWords.segments(counts)
        let total = max(1, PlanWords.total(counts))
        GeometryReader { proxy in
            HStack(spacing: 1) {
                ForEach(Array(parts.enumerated()), id: \.offset) { _, part in
                    Rectangle()
                        .fill(Self.style(part.kind))
                        .frame(width: max(0, proxy.size.width * CGFloat(part.count) / CGFloat(total) - 1))
                }
                Spacer(minLength: 0)
            }
            .clipShape(Capsule())
        }
        .frame(height: 4)
        .background(Capsule().fill(.quinary))
        .accessibilityElement()
        .accessibilityLabel(PlanWords.breakdown(counts))
    }

    static func style(_ kind: PlanSegment.Kind) -> AnyShapeStyle {
        switch kind {
        case .done: AnyShapeStyle(.secondary)
        case .inReview: AnyShapeStyle(.tertiary)
        case .inProgress: AnyShapeStyle(.quaternary)
        case .notStarted: AnyShapeStyle(.quinary)
        }
    }
}
