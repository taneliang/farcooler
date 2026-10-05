import AgentKit
import AppKit
import SwiftUI

// A theme in the canvas, as a short brief (ov-331, design 2). The owner, Oct 5:
// "the Themes list in the content view should probably be longer form instead
// of just replicating the sidebar's short bullets … the themes are really
// important because they give me an overview of what both me and the agent
// are trying to accomplish. It's also intended to help the user stay on
// track."
//
// It answers three questions in order: what are we trying to achieve (the
// outcome), where does it stand (the bar, the story with its age) and is it
// moving, and what's in the way (the track line, the ask, Next, the lanes, what
// landed and what was decided). What a theme's page has beyond that (its
// cards, spend, trend, agents, timeline) stays on the page. The sidebar's row
// (`PlanThemeRow`) finds a theme; this one says what it's for.
//
// The head is one button that opens the theme's page; the chevron folds the
// body, and ⌥-click folds every theme. Amber is the owner's alone: an ask, and
// a budget gone over. The track line is neutral words with a glyph.

/// The Plan view's theme entries are open until the owner closes one, and
/// that is kept per theme, per board (design 4, ruling D2).
enum PlanThemeFold {
    static func key(theme: String, host: String, workspace: String) -> String {
        "board.plan.theme.\(theme).\(host).\(workspace)"
    }

    /// Open unless the owner closed it; a paused or done theme, which sits in
    /// the fold at the end, is closed unless the owner opened it.
    static func isExpanded(
        theme: PlanTheme, host: String, workspace: String, in defaults: UserDefaults
    ) -> Bool {
        let key = key(theme: theme.id, host: host, workspace: workspace)
        return defaults.object(forKey: key).map { _ in defaults.bool(forKey: key) } ?? (theme.state == "active")
    }

    static func set(_ expanded: Bool, theme: String, host: String, workspace: String, in defaults: UserDefaults) {
        defaults.set(expanded, forKey: key(theme: theme, host: host, workspace: workspace))
    }

    /// What a click on an entry's chevron does: it folds the entry or opens
    /// it, and with ⌥ held, every theme to match it, as the Finder does.
    /// Returns whether the clicked entry is now open.
    @discardableResult
    static func toggle(
        _ theme: PlanTheme, all: Bool, among themes: [PlanTheme], host: String, workspace: String,
        in defaults: UserDefaults
    ) -> Bool {
        let open = !isExpanded(theme: theme, host: host, workspace: workspace, in: defaults)
        for target in all ? themes : [theme] {
            set(open, theme: target.id, host: host, workspace: workspace, in: defaults)
        }
        return open
    }
}

struct PlanThemeEntry: View {
    let theme: PlanTheme
    let plan: PlanModel
    /// The board's own word on its cards' last moves, by task id.
    var activity: [String: Int64] = [:]
    let expanded: Bool
    let selected: Bool
    let keyed: Bool
    /// Fold or unfold this entry; true when ⌥ was held, for every entry.
    let onToggle: (Bool) -> Void
    let onOpen: (PlanPage) -> Void
    var copy: @MainActor (String) -> Void = PlanRulingsList.toPasteboard

    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.boardMotionSlowdown) private var slowdown
    @State private var hovering = false

    private var brief: PlanThemeBrief { PlanThemeBrief(theme, in: plan, activity: activity) }

    var body: some View {
        let brief = brief
        VStack(alignment: .leading, spacing: 0) {
            head(brief)
            if expanded {
                body(brief)
                    .transition(CollapsibleSection<EmptyView, EmptyView, EmptyView>.contentTransition(
                        reduceMotion: reduceMotion, slowedBy: slowdown))
            } else {
                collapsed(brief)
            }
        }
        .accessibilityElement(children: .contain)
        .identified("plan-theme-entry-\(theme.name)")
    }

    // MARK: Head

    private func head(_ brief: PlanThemeBrief) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            Button {
                onToggle(NSEvent.modifierFlags.contains(.option))
            } label: {
                DisclosureChevron(expanded: expanded)
                    .frame(width: NavigatorGrid.mark, alignment: .center)
                    .padding(.trailing, NavigatorGrid.gap)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(expanded ? "Collapse \(theme.name)" : "Expand \(theme.name)")
            .accessibilityValue(expanded ? "Expanded" : "Collapsed")
            .probed("plan-theme-entry-\(theme.name)-chevron")
            Button { onOpen(.theme(theme.id)) } label: {
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    Text(theme.name)
                        .font(.system(size: WorkspaceStyle.PaneText.title, weight: .semibold))
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .layoutPriority(1)
                    Spacer(minLength: SidebarGrid.gap)
                    Text(PlanWords.progress(theme.counts))
                        .font(.system(size: WorkspaceStyle.PaneText.secondary).monospacedDigit())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .fixedSize()
                        .probed("plan-theme-entry-\(theme.name)-progress")
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(headAccessibility(brief))
            .accessibilityHint("Opens the theme")
            .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            .probed("plan-theme-entry-\(theme.name)-head")
        }
        .padding(.trailing, NavigatorGrid.trailingInset)
        .padding(.vertical, NavigatorRhythm.air)
        .background {
            if selected {
                RoundedRectangle.control.fill(Fill.selection(active: keyed)).boxOutset()
            } else if hovering {
                RoundedRectangle.control.fill(Fill.hover).boxOutset()
            }
        }
        .onHover { hovering = $0 }
    }

    private func headAccessibility(_ brief: PlanThemeBrief) -> String {
        var parts = [theme.name, PlanWords.progress(theme.counts), PlanWords.trackSpoken(brief.track, now: plan.nowMs)]
        if let ask = brief.ask { parts.append("Needs you: \(ask)") }
        return parts.joined(separator: ", ")
    }

    // MARK: Folded

    /// Closed: the head's second line, the track, and one line of the outcome.
    private func collapsed(_ brief: PlanThemeBrief) -> some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
            trackLine(brief)
            if let outcome = brief.outcome {
                Text(outcome)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .probed("plan-theme-entry-\(theme.name)-outcome")
            }
            if let ask = brief.ask { PlanAsk(text: ask, lines: 1) }
        }
        .padding(.leading, NavigatorGrid.textInset)
        .padding(.trailing, NavigatorGrid.trailingInset)
    }

    // MARK: Open

    private func body(_ brief: PlanThemeBrief) -> some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            VStack(alignment: .leading, spacing: Spacing.tight) {
                if let outcome = brief.outcome {
                    // Up to three lines, the owner's ruling (ov-273): an
                    // outcome is one sentence, and two lines cut most of them.
                    TaskKeyText(keysIn: outcome)
                        .font(Self.secondaryFont)
                        .foregroundStyle(.secondary)
                        .lineLimit(PlanThemeBrief.outcomeLines)
                        .fixedSize(horizontal: false, vertical: true)
                        .probed("plan-theme-entry-\(theme.name)-outcome")
                }
                PlanBar(counts: theme.counts)
                    .padding(.vertical, Spacing.tight)
                trackLine(brief)
            }
            if let story = brief.story {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    TaskKeyText(keysIn: story)
                        .font(.system(size: WorkspaceStyle.PaneText.body))
                        .lineLimit(PlanThemeBrief.storyLines)
                        .fixedSize(horizontal: false, vertical: true)
                        .probed("plan-theme-entry-\(theme.name)-story")
                    if let age = brief.storyAge {
                        Text(age)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(.tertiary)
                            .probed("plan-theme-entry-\(theme.name)-story-age")
                    }
                }
            }
            if brief.ask != nil || brief.next != nil {
                VStack(alignment: .leading, spacing: Spacing.tight) {
                    if let ask = brief.ask { PlanAsk(text: ask, lines: 3).padding(.top, 0) }
                    if let next = brief.next {
                        (Text("Next: ").foregroundStyle(.secondary) + Text(next))
                            .font(Self.secondaryFont)
                            .lineLimit(3)
                            .fixedSize(horizontal: false, vertical: true)
                            .probed("plan-theme-entry-\(theme.name)-next")
                    }
                }
            }
            if !brief.moving.isEmpty || brief.landed != nil || !brief.decided.isEmpty { details(brief) }
            if let budget = brief.budget {
                PlanBudgetLine(budget: budget, font: Self.secondaryFont)
            }
        }
        .padding(.leading, NavigatorGrid.textInset)
        .padding(.trailing, NavigatorGrid.trailingInset)
        .padding(.top, NavigatorRhythm.air)
        .padding(.bottom, NavigatorRhythm.air)
    }

    static let secondaryFont = Font.system(size: WorkspaceStyle.PaneText.secondary)

    /// "Moving it", "Landed this week" and "Decided": a label column and
    /// what's under it.
    private func details(_ brief: PlanThemeBrief) -> some View {
        Grid(alignment: .topLeading, horizontalSpacing: Spacing.group, verticalSpacing: Spacing.tight) {
            if !brief.moving.isEmpty {
                GridRow {
                    label("Moving it")
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(brief.moving) { lane in laneRow(lane) }
                        if brief.movingMore > 0 { more(brief.movingMore) }
                    }
                }
            }
            if let landed = brief.landed {
                GridRow {
                    label("Landed this week")
                    Text(landed)
                        .font(Self.secondaryFont)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                        .probed("plan-theme-entry-\(theme.name)-landed")
                }
            }
            if !brief.decided.isEmpty {
                GridRow {
                    label("Decided")
                    VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                        ForEach(brief.decided) { ruling in decidedRow(ruling) }
                        if brief.decidedMore > 0 { more(brief.decidedMore) }
                    }
                }
            }
        }
    }

    private func label(_ text: String) -> some View {
        Text(text)
            .font(.system(size: WorkspaceStyle.PaneText.secondary))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .fixedSize()
            .gridColumnAlignment(.leading)
    }

    private func more(_ n: Int) -> some View {
        Text("+\(n) more")
            .font(Self.secondaryFont)
            .foregroundStyle(.tertiary)
    }

    private func laneRow(_ lane: PlanLane) -> some View {
        Button { onOpen(.lane(lane.id)) } label: {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.tight + 2) {
                Image(systemName: PlanGlyph.name(lane.state))
                    .font(Self.secondaryFont)
                    .foregroundStyle(.secondary)
                    .frame(width: NavigatorGrid.mark, alignment: .center)  // one column, whatever the glyph's width
                    .accessibilityHidden(true)
                Text(lane.name)
                    .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                    .lineLimit(1)
                    .layoutPriority(1)
                Text(PlanWords.status(lane))
                    .font(Self.secondaryFont)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: 0)
            }
            .padding(.vertical, NavigatorRhythm.lineGap)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(lane.name), \(PlanWords.status(lane))")
        .accessibilityHint("Opens the lane")
        .probed("plan-theme-entry-\(theme.name)-lane-\(lane.name)")
    }

    private func decidedRow(_ ruling: PlanRuling) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Text(ruling.short)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold).monospacedDigit())
                .foregroundStyle(.secondary)
                .fixedSize()
            TaskKeyText(keysIn: ruling.decision)
                .font(Self.secondaryFont)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
        }
        .contentShape(Rectangle())
        .contextMenu { Button(PlanWords.copyReference) { copy(ruling.reference) } }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.rulingAccessibility(ruling))
        .accessibilityAction(named: PlanWords.copyReference) { copy(ruling.reference) }
        .probed("plan-theme-entry-\(theme.name)-ruling-\(ruling.short)")
    }

    // MARK: The track line

    /// The glyph and the words: neutral, and amber only for a budget gone
    /// over, which says so in words and a filled glyph.
    private func trackLine(_ brief: PlanThemeBrief) -> some View {
        let track = brief.track
        let words = PlanWords.track(track, now: plan.nowMs)
        return HStack(alignment: .firstTextBaseline, spacing: Spacing.tight + 2) {
            Image(systemName: track.symbol)
                .font(Self.secondaryFont)
                .foregroundStyle(track.needsAttention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                .accessibilityHidden(true)
            Text(words)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: track.needsAttention ? .medium : .regular))
                .foregroundStyle(track.needsAttention ? AnyShapeStyle(Tint.attention(scheme)) : AnyShapeStyle(.secondary))
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.trackSpoken(track, now: plan.nowMs))
        .probed(track.needsAttention ? "plan-theme-entry-\(theme.name)-over-budget" : "plan-theme-entry-\(theme.name)-track")
    }
}
