import SwiftUI

// What the cards over a conversation show when held (ov-794), as the pills in
// Messages, Mail and Maps do: a live preview of where a tap goes, and the
// quick ways there, without opening anything first.
//
// - Now/Next: the orchestrator, its line and the band in full, over Open Plan
//   and, when something waits, Needs You.
// - Agents: who is at work and on what, over Show All Agents and each agent.

/// A held card's preview's box: the window's width less a margin either side,
/// no wider than a phone's on an iPad, and at most half the window's height.
/// The window's, not the screen's (`WindowBounds`).
@MainActor
enum CardPreview {
    static let widest: CGFloat = 400
    static var width: CGFloat { min(WindowBounds.current.width - 2 * Spacing.section, widest) }
    static var tallest: CGFloat { WindowBounds.current.height * 0.5 }
    /// The most agents the preview lists, and the menu offers one by one.
    static let mostAgents = 5
}

extension View {
    /// The preview whole when it fits the box, and otherwise its top in a
    /// scroll the box's height: at the accessibility sizes the lines ran past
    /// the menu's room.
    func cardPreviewBox() -> some View {
        ViewThatFits(in: .vertical) {
            self
            ScrollView(.vertical) { self }
                .scrollBounceBehavior(.basedOnSize)
        }
        .frame(width: CardPreview.width, alignment: .leading)
        .frame(maxHeight: CardPreview.tallest, alignment: .top)
    }
}

/// The Now/Next card held: the orchestrator and the plan, as the sheet's top
/// says them, every line of Now and Next.
struct PlanCardPreview: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let strip: PlanStrip
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.inset) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                Image(systemName: strip.orchestrator.glyph)
                    .foregroundStyle(tone)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 3) {
                    Text("\(connection.talk.seat(for: summary, build: connection.daemon).title) · \(strip.orchestrator.word)")
                        .font(.headline)
                    if let line = strip.line {
                        Text(line)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(3)
                    }
                }
            }
            if let band = strip.band {
                PhoneGlanceBand(band: band, needsYou: strip.needsYou, lines: 3)
            } else if !strip.parts.isEmpty {
                Text(strip.parts.joined(separator: " · ")).font(.subheadline)
            }
        }
        .padding(Spacing.section)
        .frame(width: CardPreview.width, alignment: .leading)
        .cardPreviewBox()
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("plan-strip-preview")
    }

    private var tone: Color {
        switch strip.orchestrator.tone {
        case .attention: Tint.attention(scheme)
        case .failure: Tint.failure
        case .quiet: .quietInk
        }
    }
}

/// The Agents card held: each agent's mark, name and what it's doing.
struct AgentsCardPreview: View {
    let agents: RunningAgents
    let answering: Bool

    var body: some View {
        let shown = agents.agents.filter { $0.kind != .orchestrator }
        VStack(alignment: .leading, spacing: Spacing.inset) {
            Text(agents.summary).font(.headline)
            ForEach(shown.prefix(CardPreview.mostAgents)) { agent in
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    GlanceMarkView(agent.mark.said(answering: answering), inAppDiameter: 8)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(agent.name).font(.subheadline.weight(.semibold))
                        if !agent.activity.isEmpty {
                            Text(agent.activity)
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                        }
                    }
                }
            }
            if shown.count > CardPreview.mostAgents {
                Text("\(shown.count - CardPreview.mostAgents) more")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(Spacing.section)
        .frame(width: CardPreview.width, alignment: .leading)
        .cardPreviewBox()
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("agents-card-preview")
    }
}

/// The Agents card's quick actions: the whole view, then each agent, which
/// opens as its row in the view does.
struct AgentsCardActions: View {
    let agents: RunningAgents
    let showAll: () -> Void
    let navigator: PhoneNavigator?

    var body: some View {
        Button("Show All Agents", systemImage: "list.bullet", action: showAll)
            .accessibilityIdentifier("agents-card-show-all")
        Section {
            ForEach(agents.agents.filter { $0.kind != .orchestrator }.prefix(CardPreview.mostAgents)) { agent in
                Button {
                    if agent.opensOwnConversation {
                        AgentsOpening.openOwn(agent)
                    } else {
                        AgentsOpening.openPane(agent, navigator: navigator)
                    }
                } label: {
                    Text(agent.name)
                    if !agent.activity.isEmpty { Text(agent.activity) }
                }
            }
        }
    }
}
