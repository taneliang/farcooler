import AgentKit
import SwiftUI

// The Plan's glance band (ov-503, design 2): pinned over the canvas, one line
// for Now and one for Next, and what needs the owner as the one amber mark.
// The words are AgentKit's `PlanStrip.band`, so the folded strip, the phones
// and the watch read the same sentence.
//
//   NOW   Train 86 waiting for CI · 2 building · 3 in review        ● 2 need you
//   NEXT  reps — Each person's asks reach the orchestrator, then skill-init

struct PlanGlanceBand: View {
    let band: PlanGlanceSentence
    let needsYou: Int
    @Environment(\.colorScheme) private var scheme

    /// The labels' column: wide enough for "NEXT" in small capitals.
    static let label: CGFloat = 36

    var body: some View {
        HStack(alignment: .top, spacing: Spacing.inset) {
            VStack(alignment: .leading, spacing: NavigatorRhythm.air) {
                line("Now", band.nowLine, quiet: band.now == nil, id: "plan-band-now")
                line("Next", band.nextLine, quiet: band.next == nil, id: "plan-band-next")
            }
            Spacer(minLength: 0)
            if needsYou > 0 { pill }
        }
        .padding(.vertical, Spacing.group)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibility)
        .identified("plan-band")
    }

    private func line(_ label: String, _ words: String, quiet: Bool, id: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Text(label)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold))
                .textCase(.uppercase)
                .kerning(0.4)
                .foregroundStyle(.secondary)
                .frame(width: Self.label, alignment: .leading)
            Text(words)
                .font(.system(size: WorkspaceStyle.PaneText.body, weight: quiet ? .regular : .medium))
                .foregroundStyle(quiet ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
                .lineLimit(2)
                .truncationMode(.tail)
                .fixedSize(horizontal: false, vertical: true)
                .help(words)
        }
        .identified(id)
    }

    private var pill: some View {
        HStack(spacing: Spacing.tight) {
            Circle().fill(Tint.attention(scheme)).frame(width: 6, height: 6).accessibilityHidden(true)
            Text(Self.needsYouWords(needsYou))
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                .foregroundStyle(Tint.attention(scheme))
                .lineLimit(1)
        }
        .padding(.horizontal, Spacing.group)
        .padding(.vertical, Spacing.tight)
        .attentionSurface(in: Capsule())
        .fixedSize()
        .identified("plan-band-needs-you")
    }

    /// "1 needs you", "2 need you".
    static func needsYouWords(_ count: Int) -> String { "\(count) \(count == 1 ? "needs" : "need") you" }

    private var accessibility: String {
        let said = band.text
        return needsYou > 0 ? "\(said) \(Self.needsYouWords(needsYou))." : said
    }
}

/// The band over a plan, reading the store: it moves as the plan does.
struct PlanGlanceBandHost: View {
    @ObservedObject var plan: PlanStore
    let statuses: [String: TaskStatus]
    var activity: [String: Int64] = [:]
    let needsYou: Int

    var body: some View {
        if plan.available, plan.hasRead, !plan.plan.isEmpty {
            let sections = plan.sections(needsYou: needsYou, statuses: statuses, activity: activity)
            PlanGlanceBand(band: PlanGlanceSentence(sections: sections), needsYou: needsYou)
        }
    }
}
