import SwiftUI
import UIKit

// Decided for you (ov-304) on the iPhone's Plan view: the calls the
// orchestrator made on the owner's behalf, after the plan's other sections.
// Standing rulings first, newest first, each with its short id, decision, why
// and what reversing costs; then the confirmed and reversed ones, a line each.
//
// No edit: the orchestrator is the only writer. Copy Reference, in the row's
// menu and as its one button, puts "ruling R-12: <decision>" on the
// clipboard for telling it. Nothing shows on a runner without
// `board_rulings`, or on a board with no rulings.

/// The section, when there's anything to show.
struct PlanRulingsSection: View {
    let plan: PlanModel
    /// Whether this runner advertises `board_rulings`.
    let keeps: Bool
    var copy: (String) -> Void = { UIPasteboard.general.string = $0 }

    var body: some View {
        if keeps, !plan.rulings.isEmpty {
            Section {
                ForEach(plan.openRulings) { ruling in
                    PlanRulingRow(ruling: ruling, copy: copy)
                }
                ForEach(plan.pastRulings) { ruling in
                    PlanSettledRulingRow(ruling: ruling, copy: copy)
                }
            } header: {
                PlanHeader(title: PlanWords.decidedForYou, count: plan.openRulings.count)
                    .accessibilityIdentifier("plan-rulings")
            }
            // A ruling recorded, confirmed or reversed moves on the list's own
            // spring, as a board row does.
            .animation(.default, value: plan.rulings.map(PlanWords.rulingSignature))
        }
    }
}

/// Copy Reference: a borderless button, with a check while it's just copied.
private struct CopyReferenceButton: View {
    let ruling: PlanRuling
    let copy: (String) -> Void
    @State private var copied = false

    var body: some View {
        Button {
            copy(ruling.reference)
            copied = true
            Task {
                try? await Task.sleep(for: .seconds(1.5))
                copied = false
            }
        } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .contentTransition(.symbolEffect(.replace))
                .frame(minWidth: 44, minHeight: 44)
        }
        .buttonStyle(.borderless)
        // Gray, as on the Mac: color is for what needs attention (review
        // 1005a L5), and a List tints a borderless button's label otherwise.
        .tint(.secondary)
        .accessibilityLabel(PlanWords.copyReference)
        .accessibilityIdentifier("plan-ruling-\(ruling.short)-copy")
    }
}

/// A standing ruling: its short id and decision, why, what reversing costs,
/// and what it touches.
struct PlanRulingRow: View {
    let ruling: PlanRuling
    let copy: (String) -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: PaneMetrics.tight) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(ruling.short)
                        .font(.subheadline.weight(.semibold).monospacedDigit())
                        .foregroundStyle(.secondary)
                    Text(ruling.decision)
                        .font(.body.weight(.medium))
                        .fixedSize(horizontal: false, vertical: true)
                }
                line(PlanWords.rulingWhy, ruling.why)
                line(PlanWords.rulingReversal, ruling.reversal)
                if let touches = PlanWords.rulingTouches(ruling) {
                    Text(touches)
                        .font(.footnote)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
            CopyReferenceButton(ruling: ruling, copy: copy)
        }
        .contextMenu {
            Button(PlanWords.copyReference, systemImage: "doc.on.doc") { copy(ruling.reference) }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.rulingAccessibility(ruling))
        .accessibilityAction(named: PlanWords.copyReference) { copy(ruling.reference) }
        .accessibilityIdentifier("plan-ruling-\(ruling.short)")
    }

    private func line(_ label: String, _ text: String) -> some View {
        (Text("\(label): ").fontWeight(.medium) + Text(text))
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// A confirmed or reversed ruling: one quiet line, with its note.
struct PlanSettledRulingRow: View {
    let ruling: PlanRuling
    let copy: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("\(ruling.short)  \(PlanWords.rulingState(ruling.state)) · \(ruling.decision)")
                .lineLimit(2)
            if !ruling.note.isEmpty {
                Text(ruling.note).foregroundStyle(.tertiary).lineLimit(2)
            }
        }
        .font(.subheadline)
        .foregroundStyle(.secondary)
        .contextMenu {
            Button(PlanWords.copyReference, systemImage: "doc.on.doc") { copy(ruling.reference) }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.rulingAccessibility(ruling))
        .accessibilityAction(named: PlanWords.copyReference) { copy(ruling.reference) }
        .accessibilityIdentifier("plan-ruling-\(ruling.short)")
    }
}
