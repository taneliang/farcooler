import SwiftUI

// The two cards over a workspace's conversation (ov-616, ov-619), as the
// Dynamic Island holds two Live Activities: Now/Next, most of the width, and
// Agents beside it, compact. Same height, radius and material, a narrow gap
// between them, on the screen's margin. One tap target each, never two in one
// card, so a thumb can't mistake one for the other.
//
// - Now/Next: the orchestrator's state as a mark, the band's Now and Next one
//   line each, and what needs you as a dot and a count. It opens the Plan
//   sheet.
// - Agents: up to three status dots and how many agents run (Claude subagents
//   and Far Cooler's own alike, `RunningAgents`). It opens the agents view.
//   With none running it goes, and Now/Next takes the row, with a spring.
//
// At the accessibility sizes the two stack, Agents under Now/Next, rather
// than cut either one's words.

struct WorkspaceCards: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    /// Draw Now/Next. An iPad's chat column has the plan beside it, and
    /// carries the Agents card alone, the row's width (ov-619 review).
    var showsPlan = true

    @ObservedObject private var reads: PlanReads
    @Environment(\.phoneNavigator) private var navigator
    @Environment(\.screenMargin) private var margin
    @Environment(\.dynamicTypeSize) private var type
    @Environment(\.colorScheme) private var scheme

    init(connection: Connection, summary: WorkspaceSummary, place: PhoneWorkspace, showsPlan: Bool = true) {
        self.connection = connection
        self.summary = summary
        self.place = place
        self.showsPlan = showsPlan
        reads = connection.plans
    }

    var body: some View {
        let strip = connection.planStrip(summary)
        let agents = connection.runningAgents(summary, runner: place.runner)
        let layout = type.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: Spacing.group))
            : AnyLayout(HStackLayout(spacing: Spacing.group))
        layout {
            if showsPlan { nowNext(strip) }
            if agents.count > 0 {
                agentsCard(agents)
                    .transition(.scale(scale: 0.6, anchor: .trailing).combined(with: .opacity))
            }
        }
        // One height for both: the taller one's.
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, margin)
        .padding(.bottom, showsPlan || agents.count > 0 ? Spacing.group : 0)
        .animation(.spring(response: 0.4, dampingFraction: 0.8), value: agents.count > 0)
        // Nothing else over the conversation asks for the plan or the board.
        .task(id: connection.keepsPlan) {
            if connection.boards[summary.id] == nil { _ = await connection.readBoard(summary) }
            if connection.keepsPlan, reads.state(summary.id) == nil { await connection.readPlan(summary) }
        }
    }

    // MARK: Now/Next

    private func nowNext(_ strip: PlanStrip) -> some View {
        let glyph = Image(systemName: strip.orchestrator.glyph)
            .foregroundStyle(tone(strip.orchestrator.tone))
            .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
            .accessibilityHidden(true)
        let trailing = HStack(spacing: Spacing.group) {
            if strip.needsYou > 0 { badge(strip.needsYou) }
            Image(systemName: "chevron.up")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.tertiary)
                .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                .accessibilityHidden(true)
        }
        let words = Group {
            if let band = strip.band {
                // From `.xLarge` up, Now and Next in full, with no line limit:
                // cut, they hid what is happening, which matters more than the
                // one-line look (ov-685). The default size keeps ov-616's one
                // line each.
                PhoneGlanceBand(band: band, needsYou: 0, lines: type > .large ? nil : 1)
            } else if strip.isEmpty {
                Text("Plan").foregroundStyle(.quietInk)
            } else {
                self.words(strip).lineLimit(type > .large ? nil : 1).truncationMode(.tail)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        return Button { navigator?.openPlan(place) } label: {
            Group {
                if type.isAccessibilitySize {
                    // The glyph, the count and the chevron on a row of their
                    // own: beside the words they left them a third of the
                    // card's width (ov-685).
                    VStack(alignment: .leading, spacing: Spacing.group) {
                        HStack(spacing: Spacing.group) { glyph; Spacer(minLength: 0); trailing }
                        CappedScroll(share: 0.2) { words }
                    }
                } else {
                    HStack(spacing: Spacing.group) { glyph; words; trailing }
                }
            }
            .font(.subheadline)
            .padding(.horizontal, Spacing.inset)
            .padding(.vertical, Spacing.group)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .surface(.floating, in: .floating)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(strip.isEmpty && strip.band == nil ? "Plan" : strip.bandAccessibilityLabel)
        .accessibilityHint("Shows the plan")
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("plan-strip")
    }

    /// The count of what needs you, in an amber dot with the number: the dot
    /// carries the color and the number the meaning (review 17).
    private func badge(_ count: Int) -> some View {
        HStack(spacing: 5) {
            Circle().fill(Tint.attention(scheme)).frame(width: 7, height: 7)
            Text("\(count)").font(.subheadline.weight(.semibold).monospacedDigit())
        }
        .accessibilityHidden(true)
        .accessibilityIdentifier("plan-strip-needs-you")
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
        case .quiet: .quietInk
        }
    }

    // MARK: Agents

    /// The card the row's width, saying its count in words: stacked at the
    /// accessibility sizes, or alone over an iPad's chat.
    private var wide: Bool { type.isAccessibilitySize || !showsPlan }

    private func agentsCard(_ agents: RunningAgents) -> some View {
        Button { navigator?.openAgents(place) } label: {
            HStack(spacing: Spacing.group) {
                // A short stack of dots, overlapping as avatars do.
                HStack(spacing: -2) {
                    ForEach(Array(agents.dots.enumerated()), id: \.offset) { _, mark in
                        GlanceMarkView(mark.said(answering: connection.isAnswering), inAppDiameter: 8)
                    }
                }
                // Stacked, the card has the row's width, and says it in words.
                Text(wide ? agents.summary : "\(agents.count)")
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .foregroundStyle(.primary)
            }
            .padding(.horizontal, Spacing.inset)
            .padding(.vertical, Spacing.group)
            .frame(maxHeight: .infinity)
            .frame(maxWidth: wide ? .infinity : nil, alignment: .leading)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .surface(.floating, in: .floating)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(agents.summary)
        .accessibilityValue("\(agents.count)")
        .accessibilityHint("Shows the agents")
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("agents-card")
    }
}


/// Its content at its own height, up to `share` of the screen's, and scrolling
/// past that: the plan strip's sentences in full at the largest sizes would
/// otherwise cover the conversation and push the composer off (ov-685).
private struct CappedScroll<Content: View>: View {
    let share: CGFloat
    @ViewBuilder let content: Content
    @State private var height: CGFloat = 0

    var body: some View {
        ScrollView(.vertical) {
            content.onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height = $0 }
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(height: height == 0 ? nil : min(height, UIScreen.main.bounds.height * share))
    }
}
