import SwiftUI

// The plan's strip on the orchestrator, and the Plan sheet it peeks
// (ov-300; Concept C of .claude/agent/reports/ov-298/research-layout.md,
// 3.3, the phone form). One line pinned under the segment control while the
// orchestrator is up: its state as a mark, what needs you, what's moving and
// what's next. A tap raises the Plan as a sheet at the medium detent, over
// the pane, which keeps its size; dragged up, it's the whole plan.
//
// The rules are AgentKit's (`PlanStrip`, `PhoneTree`); this draws, in iOS's
// idiom: a capsule of the system's glass, and color only for what needs you.
// See .claude/agent/reports/phones-tree/design.md.

extension Connection {
    /// `summary`'s strip, from what this connection holds.
    func planStrip(_ summary: WorkspaceSummary) -> PlanStrip {
        let terminal = OrchestratorSegment.terminal(in: self, summary: summary)
        let state = PhoneTree.orchestrator(terminal)
        let plan = plans.state(summary.id)?.plan ?? .empty
        return PlanStrip(
            plan: plan, needsYou: workspaceNeedsYou(summary), orchestrator: state,
            line: PhoneTree.line(terminal, state: state))
    }

    /// The workspace's Needs You count: the Mac's title bar's number.
    func workspaceNeedsYou(_ summary: WorkspaceSummary) -> Int {
        PhoneTree.needsYouCount(
            summary: summary, board: boards[summary.id], plan: plans.state(summary.id)?.plan ?? .empty,
            items: needsYou, listRead: needsYouRead && !needsYouDerived,
            listServed: daemon?.can(.needsYou) == true)
    }
}

/// The strip, and the sheet it opens.
struct PhonePlanStrip: View {
    @ObservedObject var connection: Connection
    @ObservedObject private var reads: PlanReads
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    /// The phone's own Light or Dark, which the Plan sheet keeps whatever the
    /// strip's surroundings say. The strip sits under the orchestrator's
    /// ground, which forces the terminal theme's scheme on everything in it
    /// (`WorkspaceScreen`), and a sheet inherits that environment value while
    /// its bars, grabber and background follow the system. Half the sheet
    /// came out in one scheme and half in the other, and the mix changed with
    /// the detent (ov-444).
    let appearance: ColorScheme

    @State private var peeking = false
    /// Where something chosen in the sheet goes (a page, the orchestrator's
    /// pane after a ruling's Discuss, or Needs You): gone to once the sheet
    /// has gone, so it lands on the stack rather than under a sheet going away.
    @State private var chosen: PlanSheetExit?
    @Environment(\.phoneNavigator) private var navigator
    @Environment(\.colorScheme) private var scheme

    init(connection: Connection, summary: WorkspaceSummary, place: PhoneWorkspace, appearance: ColorScheme) {
        self.connection = connection
        reads = connection.plans
        self.summary = summary
        self.place = place
        self.appearance = appearance
    }

    var body: some View {
        let strip = connection.planStrip(summary)
        // A stack, not a group: with nothing to say it still stands, so the
        // read below still runs.
        VStack(spacing: 0) {
            if !strip.isEmpty {
                Button { peeking = true } label: { label(strip) }
                    .buttonStyle(.plain)
                    .surface(.floating, in: Capsule())
                    .padding(.horizontal, 16)
                    .padding(.bottom, 8)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(strip.accessibilityLabel)
                    .accessibilityHint("Shows the plan")
                    .accessibilityAddTraits(.isButton)
                    .accessibilityIdentifier("plan-strip")
            } else {
                Color.clear.frame(height: 0).accessibilityHidden(true)
            }
        }
        // Nothing else on the orchestrator asks for the plan or the board.
        .task(id: connection.keepsPlan) {
            if connection.boards[summary.id] == nil { _ = await connection.readBoard(summary) }
            if connection.keepsPlan, reads.state(summary.id) == nil { await connection.readPlan(summary) }
        }
        .sheet(isPresented: $peeking, onDismiss: pushChosen) {
            PlanSheet(connection: connection, summary: summary, place: place) { exit in
                chosen = exit
                peeking = false
            }
            .environment(\.colorScheme, appearance)
            .preferredColorScheme(appearance)
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
        }
    }

    private func label(_ strip: PlanStrip) -> some View {
        HStack(spacing: 8) {
            Image(systemName: strip.orchestrator.glyph)
                .foregroundStyle(tone(strip.orchestrator.tone))
                .accessibilityHidden(true)
            words(strip)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            Image(systemName: "chevron.up")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.tertiary)
                .accessibilityHidden(true)
        }
        .font(.subheadline)
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
        .contentShape(Capsule())
    }

    /// The parts, what needs you first and in weight, behind an amber dot.
    /// Not amber text: over the glass on a dark pane, and on plain white,
    /// the amber measured 1.4:1 and 3.6:1, under the 4.5:1 text needs
    /// (review 17). The dot carries the color; the words carry the meaning.
    private func words(_ strip: PlanStrip) -> Text {
        var rest = strip.parts
        var text = Text("")
        if let needs = strip.needsYouWords, rest.first == needs {
            rest.removeFirst()
            text = Text(Image(systemName: "circle.fill")).font(.system(size: 7)).foregroundStyle(Tint.attention(scheme))
                + Text(" ") + Text(needs).fontWeight(.semibold)
            if !rest.isEmpty { text = text + Text(" · ") }
        }
        return text + Text(rest.joined(separator: " · "))
    }

    private func tone(_ tone: PlanStripTone) -> Color {
        switch tone {
        case .attention: Tint.attention(scheme)
        case .failure: Tint.failure
        case .quiet: .secondary
        }
    }

    private func pushChosen() {
        guard let exit = chosen else { return }
        chosen = nil
        switch exit {
        case .open(let route): navigator?.open(route)
        case .needsYou: navigator?.go([])
        }
    }
}

/// The Plan, as a sheet over the orchestrator: what the orchestrator is doing
/// and what needs you, then the plan's sections as the Board's Plan view
/// draws them.
struct PlanSheet: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    /// Somewhere to go: the sheet goes first.
    let onExit: (PlanSheetExit) -> Void

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            PlanHomeList(connection: connection, summary: summary, hook: hook) { onExit(.needsYou) }
                .navigationTitle("Plan")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { dismiss() }
                            .accessibilityIdentifier("plan-sheet-done")
                    }
                }
        }
        .accessibilityIdentifier("plan-sheet")
    }

    private var hook: PlanBoardHook {
        connection.planHomeHook(
            summary, place: place, onOpen: { onExit(.open(.plan(place, page: $0))) },
            // Keep, Keep All, Reverse and Discuss, as on the Board (review 5).
            onRuling: { onExit(.open($0)) })
    }
}

/// The Plan's home: the orchestrator's state and line, what needs you, then
/// the plan's sections as the Board's Plan view draws them. The phone's
/// sheet draws it, and so does the iPad's plan column (ov-348).
struct PlanHomeList: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let hook: PlanBoardHook
    /// The way to what needs you.
    let onNeedsYou: () -> Void

    @ObservedObject private var reads: PlanReads
    @Environment(\.colorScheme) private var scheme

    init(connection: Connection, summary: WorkspaceSummary, hook: PlanBoardHook, onNeedsYou: @escaping () -> Void) {
        self.connection = connection
        self.summary = summary
        self.hook = hook
        self.onNeedsYou = onNeedsYou
        reads = connection.plans
    }

    var body: some View {
        let strip = connection.planStrip(summary)
        List {
            Section { orchestrator(strip) }
            if connection.keepsPlan {
                PlanBoardSections(hook: hook)
            } else {
                // A runner too old to keep a plan: said, once.
                Section {
                    PlanNotice(title: PlanWords.needsUpdate, detail: nil)
                        .accessibilityIdentifier("plan-needs-update")
                }
            }
        }
        .listStyle(.insetGrouped)
        .refreshable { await hook.read() }
    }

    /// The orchestrator's state and line, and the workspace's count.
    @ViewBuilder
    private func orchestrator(_ strip: PlanStrip) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: strip.orchestrator.glyph)
                .foregroundStyle(tone(strip.orchestrator.tone))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text("Orchestrator · \(strip.orchestrator.word)")
                    .font(.headline)
                if let line = strip.line {
                    Text(line)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .accessibilityIdentifier("plan-sheet-line")
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("plan-sheet-orchestrator")
        if let needs = strip.needsYouWords {
            // The way to what needs you (review 12): the sheet goes, and the
            // app's Needs You opens. The flag carries the color, the words
            // stay the text's own (review 17).
            Button { onNeedsYou() } label: {
                HStack {
                    Label {
                        Text(needs).foregroundStyle(.primary)
                    } icon: {
                        Image(systemName: OneTreeGlyph.needsYou).foregroundStyle(Tint.attention(scheme))
                    }
                    Spacer()
                    Image(systemName: "chevron.forward")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.tertiary)
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("plan-sheet-needs-you")
        }
    }

    private func tone(_ tone: PlanStripTone) -> Color {
        switch tone {
        case .attention: Tint.attention(scheme)
        case .failure: Tint.failure
        case .quiet: .secondary
        }
    }
}

extension Connection {
    /// What the Plan's home reads and where it goes: `onOpen` for a theme's,
    /// a lane's or a page's page, `onRuling` for where a ruling's Reverse or
    /// Discuss sends the owner after it.
    func planHomeHook(
        _ summary: WorkspaceSummary, place: PhoneWorkspace, onOpen: @escaping (PhonePlanPage) -> Void,
        onRuling: @escaping @MainActor (PhoneRoute) -> Void
    ) -> PlanBoardHook {
        let connection = self
        return PlanBoardHook(
            summary: summary, place: place, reads: plans, keeps: keepsPlan,
            read: {
                await connection.readPlan(summary)
                if connection.keepsPages { await connection.readPages(summary) }
            },
            onOpen: onOpen,
            statuses: Dictionary(
                (boards[summary.id]?.rows ?? []).map { ($0.id, $0.status) },
                uniquingKeysWith: { first, _ in first }),
            pages: keepsPages ? pages : nil,
            keepsRulings: keepsRulings,
            rulingActions: rulingActions(summary, place: place, open: onRuling))
    }
}

/// Where the Plan sheet sends the phone once it has gone.
enum PlanSheetExit: Equatable {
    case open(PhoneRoute)
    case needsYou
}
