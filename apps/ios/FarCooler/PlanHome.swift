import SwiftUI

// The Plan's home on an iPad's plan column (ov-348), and the top of the Plan
// that the iPhone's Plan sheet shares with it (ov-300, ov-616): the band, the
// orchestrator's state and line, and the way to what needs you. The iPhone's
// Now/Next card is `WorkspaceCards`; its sheet, `PlanSheetScreen`.
//
// The rules are AgentKit's (`PlanStrip`, `PhoneTree`); this draws, in iOS's
// idiom, with color only for what needs you.

extension Connection {
    /// `summary`'s strip, from what this connection holds.
    func planStrip(_ summary: WorkspaceSummary) -> PlanStrip {
        // A member's strip is about their rep, which is who they talk to (ov-526).
        let terminal: Terminal?
        if case let .rep(_, _, existing) = talk.seat(for: summary, build: daemon) {
            terminal = existing.flatMap { rep in fleet.worktrees.lazy.flatMap(\.terminals).first { $0.id == rep.terminal } }
        } else {
            terminal = OrchestratorSegment.terminal(in: self, summary: summary)
        }
        let state = PhoneTree.orchestrator(terminal)
        let plan = plans.state(summary.id)?.plan ?? .empty
        return PlanStrip(
            plan: plan, needsYou: workspaceNeedsYou(summary), orchestrator: state,
            line: PhoneTree.line(terminal, state: state),
            statuses: Dictionary((boards[summary.id]?.rows ?? []).map { ($0.id, $0.status) }, uniquingKeysWith: { first, _ in first }))
    }

    /// The workspace's Needs You count: the Mac's title bar's number.
    func workspaceNeedsYou(_ summary: WorkspaceSummary) -> Int {
        PhoneTree.needsYouCount(
            summary: summary, board: boards[summary.id], plan: plans.state(summary.id)?.plan ?? .empty,
            items: needsYou, listRead: needsYouRead && !needsYouDerived,
            listServed: daemon?.can(.needsYou) == true)
    }
}

/// The Plan's home on an iPad's plan column (ov-348): the orchestrator's
/// state and line, what needs you, then the plan's sections as the Board's
/// Plan view draws them.
struct PlanHomeList: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let hook: PlanBoardHook
    /// The band says Now only, for a sheet too short for both lines.
    let compactBand: Bool
    /// The way to what needs you.
    let onNeedsYou: () -> Void

    init(
        connection: Connection, summary: WorkspaceSummary, hook: PlanBoardHook, compactBand: Bool = false,
        onNeedsYou: @escaping () -> Void
    ) {
        self.compactBand = compactBand
        self.connection = connection
        self.summary = summary
        self.hook = hook
        self.onNeedsYou = onNeedsYou
    }

    var body: some View {
        List {
            PlanHomeHeader(connection: connection, summary: summary, compactBand: compactBand, onNeedsYou: onNeedsYou)
            if connection.keepsPlan {
                PlanBoardSections(hook: hookBelowBand)
            } else {
                // A runner too old to keep a plan: said, once.
                Section {
                    PlanNotice(title: PlanWords.needsUpdate, detail: nil)
                        .accessibilityIdentifier("plan-needs-update")
                }
            }
        }
        .listStyle(.insetGrouped)
        .environment(\.defaultMinListRowHeight, PlanRowHeight.minimum)
        .refreshable { await hook.read() }
    }

    /// The hook for the sections under the band, which this list draws itself.
    private var hookBelowBand: PlanBoardHook {
        var below = hook
        below.showsBand = false
        return below
    }
}

/// The top of the Plan, as sections of a list: the band (Now and Next), the
/// orchestrator's state and line, and the way to what needs you. The
/// iPhone's Plan sheet and the iPad's plan column both start with it.
struct PlanHomeHeader: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    /// The band says Now only, for a sheet too short for both lines.
    var compactBand = false
    /// The way to what needs you.
    let onNeedsYou: () -> Void

    @ObservedObject private var reads: PlanReads
    @Environment(\.colorScheme) private var scheme

    init(connection: Connection, summary: WorkspaceSummary, compactBand: Bool = false, onNeedsYou: @escaping () -> Void) {
        self.connection = connection
        self.summary = summary
        self.compactBand = compactBand
        self.onNeedsYou = onNeedsYou
        reads = connection.plans
    }

    var body: some View {
        let strip = connection.planStrip(summary)
        // The band is the list's first row, above the orchestrator's; the
        // sections below don't draw it again.
        if let band = strip.band, reads.state(summary.id)?.plan?.isEmpty == false {
            Section {
                // The Needs You row under it says the count, so the band doesn't.
                PhoneGlanceBand(band: band, needsYou: 0, lines: 2, nowOnly: compactBand).padding(.vertical, 2)
            }
        }
        Section { orchestrator(strip) }
    }

    /// The orchestrator's state and line, and the workspace's count.
    @ViewBuilder
    private func orchestrator(_ strip: PlanStrip) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: strip.orchestrator.glyph)
                .foregroundStyle(tone(strip.orchestrator.tone))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text("\(connection.talk.seat(for: summary, build: connection.daemon).title) · \(strip.orchestrator.word)")
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
    ///
    /// `wide` is the iPad's plan column: its sidebar holds the Themes and Lanes
    /// lists that a phone pushes, and its themes grid is two tiles wide.
    func planHomeHook(
        _ summary: WorkspaceSummary, place: PhoneWorkspace, wide: Bool = false, onOpen: @escaping (PhonePlanPage) -> Void,
        onRoute: @escaping (PhoneRoute) -> Void = { _ in }, onRuling: @escaping @MainActor (PhoneRoute) -> Void
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
            rulingActions: rulingActions(summary, place: place, open: onRuling),
            facts: planFacts(summary), outline: wide ? nil : { connection.sidebarOutline(summary, filter: $0) },
            onRoute: onRoute, pushesLists: !wide, tileColumns: wide ? 2 : 1)
    }
}
