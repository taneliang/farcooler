import AgentKit
import AppKit
import SwiftUI

// Decided for you (ov-304): the calls the orchestrator made on the owner's
// behalf, at the foot of the plan canvas (Concept A, `PlanHome`). Standing
// rulings first, newest first, each with its short id, what was decided, why
// and what reversing costs; then the confirmed and reversed ones, a line each.
//
// There's no edit. The orchestrator is the only writer, so the one action is
// Copy Reference, which puts "ruling R-12: <decision>" on the clipboard for
// telling it. A runner without `board_rulings` shows nothing, and neither
// does a board with no rulings.

extension PlanStore {
    /// Whether this runner keeps rulings.
    var keepsRulings: Bool { client.daemonBuild?.can(.boardRulings) == true }
}

/// The Decided For You section, when there's anything to show.
struct PlanRulingsSection: View {
    @ObservedObject var plan: PlanStore
    var defaults: UserDefaults = .standard
    var copy: @MainActor (String) -> Void = PlanRulingsList.toPasteboard

    var body: some View {
        let model = plan.plan
        if plan.keepsRulings, !model.rulings.isEmpty {
            PlanRulingsList(
                standing: model.standingRulings, settled: model.settledRulings,
                key: "board.plan.section.rulings.\(plan.host).\(plan.workspace.id)", defaults: defaults, copy: copy)
        }
    }
}

/// The section itself, given its rulings: what the canvas draws and a test
/// renders.
struct PlanRulingsList: View {
    let standing: [PlanRuling]
    let settled: [PlanRuling]
    let key: String
    var defaults: UserDefaults = .standard
    var copy: @MainActor (String) -> Void = PlanRulingsList.toPasteboard

    var body: some View {
        CollapsibleSection(
            PlanWords.decidedForYou, id: "plan.rulings", style: .navigator, key: key, defaults: defaults,
            count: standing.count
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(standing) { ruling in
                    PlanRulingRow(ruling: ruling, copy: copy)
                        .changeWashed(ruling.id)
                }
                ForEach(settled) { ruling in
                    PlanSettledRulingRow(ruling: ruling, copy: copy)
                        .changeWashed(ruling.id)
                }
            }
            .listChanges((standing + settled).map { ListChangeRow(id: $0.id, signature: PlanWords.rulingSignature($0)) })
        }
        .identified("plan-rulings")
    }

    static func toPasteboard(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}

/// Copy Reference, as a menu item and as the row's one button.
private struct CopyReferenceButton: View {
    let ruling: PlanRuling
    let copy: @MainActor (String) -> Void
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
                .font(.system(size: WorkspaceStyle.PaneText.secondary))
                .foregroundStyle(.secondary)
                .contentTransition(.symbolEffect(.replace))
        }
        .buttonStyle(.borderless)
        .help("\(PlanWords.copyReference): \(ruling.reference)")
        .accessibilityLabel(PlanWords.copyReference)
        .identified("plan-ruling-\(ruling.short)-copy")
    }
}

/// A standing ruling: its short id and decision, then why, what reversing
/// costs and what it touches.
struct PlanRulingRow: View {
    let ruling: PlanRuling
    let copy: @MainActor (String) -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            Text(ruling.short)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold).monospacedDigit())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .fixedSize()
                .frame(width: NavigatorGrid.textInset + Spacing.inset, alignment: .leading)
            VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                    TaskKeyText(keysIn: ruling.decision)
                        .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                        .fixedSize(horizontal: false, vertical: true)
                        .layoutPriority(1)
                    Spacer(minLength: 0)
                    CopyReferenceButton(ruling: ruling, copy: copy)
                }
                line(PlanWords.rulingWhy, ruling.why)
                line(PlanWords.rulingReversal, ruling.reversal)
                if let touches = PlanWords.rulingTouches(ruling) {
                    TaskKeyText(keysIn: touches)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
        }
        .padding(.vertical, NavigatorRhythm.lineGap * 2)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .contextMenu {
            Button(PlanWords.copyReference) { copy(ruling.reference) }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.rulingAccessibility(ruling))
        .accessibilityAction(named: PlanWords.copyReference) { copy(ruling.reference) }
        .identified("plan-ruling-\(ruling.short)")
    }

    /// "Why: It's the one attention color…", the label in the secondary
    /// weight so the words read first.
    private func line(_ label: String, _ text: String) -> some View {
        (Text("\(label): ").fontWeight(.medium) + Text(text))
            .font(.system(size: WorkspaceStyle.PaneText.secondary))
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// A confirmed or reversed ruling: one quiet line, "R-1 Confirmed · Unread
/// stays on the phones.", with its note under it if there is one.
struct PlanSettledRulingRow: View {
    let ruling: PlanRuling
    let copy: @MainActor (String) -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            Text(ruling.short)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold).monospacedDigit())
                .lineLimit(1)
                .fixedSize()
                .frame(width: NavigatorGrid.textInset + Spacing.inset, alignment: .leading)
            VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                Text("\(PlanWords.rulingState(ruling.state)) · \(ruling.decision)")
                    .lineLimit(2)
                if !ruling.note.isEmpty {
                    Text(ruling.note).foregroundStyle(.tertiary).lineLimit(2)
                }
            }
        }
        .font(.system(size: WorkspaceStyle.PaneText.secondary))
        .foregroundStyle(.secondary)
        .padding(.vertical, NavigatorRhythm.lineGap)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .contextMenu {
            Button(PlanWords.copyReference) { copy(ruling.reference) }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.rulingAccessibility(ruling))
        .accessibilityAction(named: PlanWords.copyReference) { copy(ruling.reference) }
        .identified("plan-ruling-\(ruling.short)")
    }
}
