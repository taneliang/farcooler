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
//
// Glass the system's way (ov-794): each card's glass is interactive, so a
// press or a drag swells and lights it as the pills in Messages and Maps do;
// both are one glass container, so Agents comes out of and goes back into
// Now/Next as one piece; held, each shows a live preview with its quick
// actions (`WorkspaceCardMenus`); and a tap grows its sheet out of the card
// (`PhoneZoom`) rather than sliding one up from the foot of the screen. The
// glass is the button's own (`GlassCardStyle`), so a hold lifts the card
// whole.

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
    @Environment(\.phoneZoom) private var zoom
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The two cards' glass, for Agents to morph out of Now/Next.
    @Namespace private var glass
    /// Read for the card's cap: a turn of the phone changes it, so the cap,
    /// a share of the screen's height, is worked out again (a phone on its
    /// side kept its upright cap, two thirds of the screen).
    @Environment(\.verticalSizeClass) private var verticalSize
    /// A line of the card's words at the reader's size, for its floor.
    @ScaledMetric(relativeTo: .subheadline) private var line: CGFloat = 20

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
        // One container: closer than the gap between the cards, so at rest
        // they stay two, and pressed, a card's swell reaches toward the other.
        FloatingGroup(spacing: Spacing.tight) {
            layout {
                if showsPlan { nowNext(strip) }
                if agents.count > 0 {
                    // Morphs out of Now/Next's glass and back into it; with
                    // Reduce Motion, fades in and out where it stands.
                    agentsCard(agents)
                        .glassEffectTransition(reduceMotion ? .identity : .matchedGeometry)
                        .transition(.opacity)
                }
            }
            // One height for both: the taller one's.
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, margin)
        .padding(.bottom, showsPlan || agents.count > 0 ? Spacing.group : 0)
        .animation(
            reduceMotion ? .easeInOut(duration: 0.2) : .spring(response: 0.4, dampingFraction: 0.8), value: agents.count > 0)
        // Nothing else over the conversation asks for the plan or the board.
        .task(id: connection.keepsPlan) {
            if connection.boards[summary.id] == nil { _ = await connection.readBoard(summary) }
            if connection.keepsPlan, reads.state(summary.id) == nil { await connection.readPlan(summary) }
        }
    }

    // MARK: Now/Next

    /// The most of the screen's height the Now/Next card takes (ov-423).
    static let cardShare: CGFloat = 0.2

    /// The card's content's most height: its share of the screen as the
    /// phone is held now, less its insets, and never less than its top row
    /// and two lines' room for the words, which scroll. A phone on its side
    /// at the largest size has a fifth of 390 points, which the row alone
    /// nearly fills: without the floor the words ran out of the card.
    private var cardCap: CGFloat {
        _ = verticalSize
        let floor = line * 2 + Spacing.group + 4
        return max(UIScreen.main.bounds.height * Self.cardShare - Spacing.group * 2, floor)
    }

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
                    // The whole card, row and words and its insets, stays
                    // within a fifth of the screen, as ov-423 held it; the
                    // words scroll inside it past that. Capping the words
                    // alone at a fifth made the card 29% (the Oct 22 nightly,
                    // DynamicTypeTests). One layout pass, no measured state:
                    // the row takes its height, the words what's left, whole
                    // if they fit and in a scroll if not, so the first frame
                    // is the settled one.
                    VStack(alignment: .leading, spacing: Spacing.group) {
                        HStack(spacing: Spacing.group) { glyph; Spacer(minLength: 0); trailing }
                        ViewThatFits(in: .vertical) {
                            words
                            ScrollView(.vertical) { words }
                                .scrollBounceBehavior(.basedOnSize)
                        }
                    }
                    .frame(maxHeight: cardCap, alignment: .top)
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
        // The glass and the sheet's source, inside the button; the menu on it
        // (`GlassCardStyle`).
        .buttonStyle(GlassCardStyle(glassID: "plan", in: glass, zoomID: PhoneZoom.plan(place), zoom: zoom))
        .contentShape(.contextMenuPreview, .floating)
        .contextMenu {
            Button("Open Plan", systemImage: "list.bullet.rectangle") { navigator?.openPlan(place) }
                .accessibilityIdentifier("plan-strip-open")
            if strip.needsYou > 0 {
                Button("Show Needs You", systemImage: "exclamationmark.circle") { navigator?.go([]) }
                    .accessibilityIdentifier("plan-strip-needs-you-action")
            }
        } preview: {
            PlanCardPreview(connection: connection, summary: summary, strip: strip)
        }
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
        .buttonStyle(GlassCardStyle(glassID: "agents", in: glass, zoomID: PhoneZoom.agents(place), zoom: zoom))
        .contentShape(.contextMenuPreview, .floating)
        .contextMenu {
            AgentsCardActions(agents: agents, showAll: { navigator?.openAgents(place) }, navigator: navigator)
        } preview: {
            AgentsCardPreview(agents: agents, answering: connection.isAnswering)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(agents.summary)
        .accessibilityValue("\(agents.count)")
        .accessibilityHint("Shows the agents")
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("agents-card")
    }
}

/// A card's glass, and its press left to that glass: `.plain` dimmed the
/// words under a finger as the glass swelled, two answers to one touch.
///
/// The glass is drawn here, inside the button, with the sheet's zoom source
/// under it, and the card's context menu goes on the button, outside the
/// glass: the order is measured, not taste (ov-805, frames of a hold read
/// from a screen recording).
/// - The menu inside the glass: a hold made the words jump right. The glass
///   swells about its middle, and the words with it; as the menu took them
///   for its lift they were redrawn about 7 pt to the right, as if swollen
///   from their leading edge, and stayed there for the hold.
/// - The glass on the button from outside, as a modifier, with the menu
///   outside that: a hold never showed the menu, and its release was a tap
///   (ov-794).
/// - The zoom source outside the glass: the sheet grew from the middle of
///   the screen, its source never found (ov-794).
///
/// Drawn by the style, the glass is the button's own, as `.glass` draws it,
/// and the menu lifts the whole card.
private struct GlassCardStyle: ButtonStyle {
    /// The card's glass, for Agents to morph out of Now/Next.
    let glassID: String
    let glass: Namespace.ID
    /// The sheet the card opens, to grow out of it (`PhoneZoom`).
    let zoomID: String
    let zoom: Namespace.ID?

    init(glassID: String, in glass: Namespace.ID, zoomID: String, zoom: Namespace.ID?) {
        self.glassID = glassID
        self.glass = glass
        self.zoomID = zoomID
        self.zoom = zoom
    }

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .zoomSource(zoomID, in: zoom)
            .surface(.floating, in: .floating, interactive: true)
            .glassEffectID(glassID, in: glass)
    }
}
