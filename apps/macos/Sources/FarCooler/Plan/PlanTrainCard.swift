import AgentKit
import SwiftUI

// A train in Shipping (ov-503, design 2): a card with its title and what it
// carries, a ladder of where it stands, the CI line, and its lanes with how
// each stands relative to the train. Amber only for CI that failed, a train
// that stalled or is held, and a lane waiting on the owner, each with words.
//
//   Train 86 · The team directory; the iPhone composer
//   ▬▬▬▬▬ ▬▬▬▬▬▬▬▬▬▬ ▬▬▬▬▬ ▬▬▬▬▬▬▬▬▬▬▬▬▬ ▬▬▬▬▬▬
//   Picked Integrated Gated Waiting for CI Landed
//   3f78754f · 2 of 5 jobs done · 12 min · Integrator · Opus · 1.4M
//   Holding for ios-overwrite · 2 h
//   ✓ team-dir          Ready
//   ⚠ ios-overwrite     Fixing · round 2             [Take Off Train]

/// The ladder a train climbs: five steps, and which one a phase is on.
enum PlanLadder {
    static let steps = ["Picked", "Integrated", "Gated", "Waiting for CI", "Landed"]

    /// The step a phase is standing on, 0 to 4; 5 once everything is done.
    static func step(_ phase: TrainPhase) -> Int {
        switch phase {
        case .waitingForLanes, .unknown: 0
        case .integrating, .stalled: 1
        case .gatingLocally: 2
        case .waitingForCi, .ciFailed: 3
        case .readyToLand: 4
        case .landed, .dropped: 5
        }
    }

    /// What step `index` says while a train stands on `current` in `phase`:
    /// the phase's own word on the step it's on.
    static func label(_ index: Int, current: Int, phase: TrainPhase) -> String {
        index == current ? phase.word : steps[index]
    }
}

struct PlanTrainCard: View {
    let card: PlanSections.TrainCard
    let ci: PlanCIRead?
    let now: Int64
    /// Whether a held train's lane can be taken off from here.
    let canMove: Bool
    /// Lanes being moved now, by id: their button waits.
    let moving: Set<String>
    let selected: PlanPage?
    let keyed: Bool
    let onOpen: (PlanPage) -> Void
    let onMove: (PlanLane) -> Void
    @Environment(\.colorScheme) private var scheme
    @Environment(\.openURL) private var openURL

    private var train: PlanTrain { card.train }
    private var run: URL? { ci.flatMap { PageLinks.https($0.url) } }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            heading
            PlanLadderView(phase: card.phase)
            facts
            if let held = card.held {
                Text(held)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                    .foregroundStyle(Tint.attention(scheme))
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("plan-train-\(train.name)-held")
            }
            if !card.lanes.isEmpty {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(card.lanes) { lane in
                        PlanTrainLaneRow(
                            lane: lane, now: now, canMove: canMove, moving: moving.contains(lane.id),
                            selected: selected == .lane(lane.id), keyed: keyed, onOpen: { onOpen(.lane(lane.id)) },
                            onMove: { onMove(lane.lane) })
                    }
                }
            }
        }
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.inset)
        .frame(maxWidth: .infinity, alignment: .leading)
        .surface(.inset, in: .card)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(accessibility)
        .identified("plan-train-\(train.name)")
    }

    private var heading: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
            Text(train.heading)
                .font(.system(size: WorkspaceStyle.PaneText.title, weight: .semibold))
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
            if let carries = train.carries {
                Text(carries)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .identified("plan-train-\(train.name)-carries")
            }
        }
        // The slug is for a person who needs it: here, and nowhere in the text.
        .help(train.slug ?? "")
    }

    /// "3f78754f · 2 of 5 jobs done · 12 min · Integrator · Opus · 1.4M": the
    /// commit, how CI stands, how long it has been here and who integrates.
    /// The commit opens its run, the one place a train goes.
    private var facts: some View {
        let words = Self.facts(card, ci: ci, now: now)
        return Group {
            if words.isEmpty {
                EmptyView()
            } else if let run {
                Button { openURL(run) } label: { line(words) }
                    .buttonStyle(.plain)
                    .help("Open CI on \(run.host() ?? "GitHub")")
            } else {
                line(words)
            }
        }
        .identified("plan-train-\(train.name)-words")
    }

    private func line(_ words: String) -> some View {
        Text(words)
            .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: card.phase.needsAttention ? .medium : .regular))
            .foregroundStyle(card.phase.needsAttention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
    }

    /// The facts line's words, apart so a test reads them.
    static func facts(_ card: PlanSections.TrainCard, ci: PlanCIRead?, now: Int64) -> String {
        var parts: [String] = []
        if let sha = card.train.pushedSha, !sha.isEmpty { parts.append(String(sha.prefix(8))) }
        if let ci {
            parts.append(PlanWords.ciSummary(ci))
            if let stale = PlanWords.ciStale(ci, now: now) { parts.append(stale) }
        } else if card.train.pushedSha != nil {
            parts.append("CI not read yet")
        }
        if card.since > 0 { parts.append(PlanWords.span(now - card.since)) }
        if let agent = card.train.agent { parts.append(Self.integrator(agent)) }
        return parts.joined(separator: " \u{00B7} ")
    }

    /// "Integrator · Opus · 1.4M", or "Integrator · Opus · done".
    static func integrator(_ agent: PlanTrainAgent) -> String {
        var parts = ["Integrator"]
        let model = PlanWords.model(agent.model)
        if !model.isEmpty { parts.append(model) }
        if agent.spend.totalTokens > 0 {
            parts.append(TaskUsageFormat.tokens(agent.spend.totalTokens))
        } else if !agent.isWorking {
            parts.append("done")
        }
        return parts.joined(separator: " \u{00B7} ")
    }

    private var accessibility: String {
        let step = PlanLadder.step(card.phase)
        let place = step >= PlanLadder.steps.count ? "" : ", step \(step + 1) of \(PlanLadder.steps.count)"
        return [train.heading, train.carries, card.phase.word + place, card.held].compactMap { $0 }.joined(separator: ", ")
    }
}

/// Where a train stands, as five segments and the words under them. Roomy, the
/// five steps are named; narrow, the phase alone is.
struct PlanLadderView: View {
    let phase: TrainPhase
    @Environment(\.colorScheme) private var scheme

    private var current: Int { PlanLadder.step(phase) }
    private var tint: Color? { phase.needsAttention ? Tint.attention(scheme) : nil }

    var body: some View {
        ViewThatFits(in: .horizontal) {
            named
            compact
        }
        .accessibilityHidden(true)
        .identified("plan-ladder")
    }

    private func segment(_ index: Int) -> some View {
        Capsule()
            .fill(index < current ? AnyShapeStyle(.secondary) : index == current ? AnyShapeStyle(tint ?? .primary) : AnyShapeStyle(.quaternary))
            .frame(height: PlanMetrics.bar)
    }

    private var named: some View {
        HStack(alignment: .top, spacing: Spacing.group) {
            ForEach(0..<PlanLadder.steps.count, id: \.self) { index in
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    segment(index)
                    Text(PlanLadder.label(index, current: current, phase: phase))
                        .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: index == current ? .medium : .regular))
                        .foregroundStyle(
                            index == current
                                ? AnyShapeStyle(tint ?? Color.primary) : index < current ? AnyShapeStyle(.secondary) : AnyShapeStyle(.tertiary)
                        )
                        .lineLimit(1)
                        .fixedSize()
                }
                .fixedSize(horizontal: true, vertical: false)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var compact: some View {
        VStack(alignment: .leading, spacing: Spacing.tight) {
            HStack(spacing: Spacing.tight) {
                ForEach(0..<PlanLadder.steps.count, id: \.self) { segment($0) }
            }
            Text(phase.word)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                .foregroundStyle(tint ?? Color.primary)
                .lineLimit(1)
        }
    }
}

/// One lane inside a train: its title and how it stands, its cards, and, on the
/// one holding the train, the button that takes it off.
struct PlanTrainLaneRow: View {
    let lane: PlanSections.TrainLane
    let now: Int64
    let canMove: Bool
    let moving: Bool
    let selected: Bool
    let keyed: Bool
    let onOpen: () -> Void
    let onMove: () -> Void
    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false

    private var warning: String? { lane.waitsOnOwner ? "Needs you" : PlanWords.stale(lane.lane, now: now) }
    private var amber: Bool { lane.holding || warning != nil }

    /// "Move to Next Train" for a ready lane, which boards the next; "Take Off
    /// Train" for any other, which goes back to waiting.
    static func button(for standing: PlanSections.Standing) -> String {
        if case .ready = standing { return "Move to Next Train" }
        return "Take Off Train"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button(action: onOpen) {
                HStack(alignment: .firstTextBaseline, spacing: 0) {
                    Image(systemName: glyph)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(amber ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                        .glyphColumn()
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                            Text(lane.lane.heading)
                                .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                                .lineLimit(2)
                                .fixedSize(horizontal: false, vertical: true)
                                .layoutPriority(1)
                            Spacer(minLength: 0)
                            Text(lane.standing.word)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .fixedSize()
                                .layoutPriority(2)
                                .probed("plan-train-lane-\(lane.name)-standing")
                        }
                        if !lane.lane.cards.isEmpty {
                            TaskKeyText(keysIn: PlanWords.cardKeys(lane.lane))
                                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                        if let warning {
                            Text(warning)
                                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                                .foregroundStyle(Tint.attention(scheme))
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .navigatorRow(selected: selected, keyed: keyed, leading: 0)
                .background {
                    if hovering && !selected { RoundedRectangle.control.fill(Fill.hover).boxOutset() }
                }
            }
            .buttonStyle(.plain)
            .onHover { hovering = $0 }
            .accessibilityElement(children: .combine)
            .accessibilityLabel([lane.lane.heading, lane.standing.word, warning].compactMap { $0 }.joined(separator: ", "))
            .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            .help(lane.lane.slug ?? "")
            .identified("plan-train-lane-\(lane.name)")
            if lane.holding && canMove {
                Button(Self.button(for: lane.standing), action: onMove)
                    .controlSize(.small)
                    .disabled(moving)
                    .padding(.leading, NavigatorGrid.textInset)
                    .padding(.bottom, NavigatorRhythm.air)
                    .identified("plan-train-lane-\(lane.name)-move")
            }
        }
    }

    private var glyph: String {
        if lane.holding { return "exclamationmark.triangle" }
        switch lane.standing {
        case .ready: return "checkmark.circle"
        case .fixing: return "wrench.and.screwdriver"
        case .landed: return "checkmark.circle.fill"
        case .notReady(let state): return PlanGlyph.name(state)
        }
    }
}
