import SwiftUI
import UIKit

// Decided for you (ov-304) on the iPhone's Plan view: the calls the
// orchestrator made on the owner's behalf, after the plan's other sections.
// Only the open ones, newest first (ov-333), each with its short id, decision,
// why and what reversing costs. The owner acts on each: swipe for Keep,
// Reverse and Discuss, or press and hold for the same menu, and Keep All sits
// in the section's header. The kept and reversed ones fold into Past
// Decisions, a line each, closed until opened.
//
// Keep is the owner's own mark and never reaches the orchestrator. Reverse
// sends it the ruling's recorded reversal; Discuss quotes the ruling into its
// composer, unsent (`RulingActions`, AgentKit). No count anywhere but the
// header's, and no notification. Nothing shows on a runner without
// `board_rulings`, or on a board with no rulings.

/// What the owner's actions on a ruling do on this phone, handed to the
/// section so a test can read them. Reverse and Discuss answer a sentence to
/// show when something needs saying, or nil.
struct PhoneRulingActions {
    /// Whether this runner takes the owner's marks (`board_ruling_actions`).
    var canMark = false
    /// Whether the workspace has an orchestrator running to ask.
    var canAsk = false
    var keep: (PlanRuling) -> Void = { _ in }
    var keepAll: () -> Void = {}
    var reverse: (PlanRuling) async -> String? = { _ in nil }
    var discuss: (PlanRuling) async -> String? = { _ in nil }
}

/// The sections, when there's anything to show.
struct PlanRulingsSection: View {
    let plan: PlanModel
    /// Whether this runner advertises `board_rulings`.
    let keeps: Bool
    var actions = PhoneRulingActions()
    var copy: (String) -> Void = { UIPasteboard.general.string = $0 }

    /// Whether Past Decisions is open: the board's, as Landed Today's is, so a
    /// re-read of the plan can't close it.
    @Binding var pastOpen: Bool
    /// What the last Reverse or Discuss had to say, under the open rulings.
    @State private var notice: String?

    var body: some View {
        if keeps, !plan.rulings.isEmpty {
            let open = plan.openRulings
            let past = plan.pastRulings
            if !open.isEmpty {
                Section {
                    ForEach(open) { ruling in
                        PlanRulingRow(ruling: ruling, actions: actions, copy: copy, notice: $notice)
                    }
                } header: {
                    HStack(spacing: PaneMetrics.tight) {
                        PlanHeader(title: PlanWords.decidedForYou, count: open.count)
                            .accessibilityIdentifier("plan-rulings")
                        if actions.canMark, open.count > 1 {
                            Button(PlanWords.keepAllRulings) { actions.keepAll() }
                                .font(.subheadline)
                                .textCase(nil)
                                .tint(.secondary)
                                .accessibilityIdentifier("plan-rulings-keep-all")
                        }
                    }
                } footer: {
                    if let notice {
                        Text(notice).accessibilityIdentifier("plan-rulings-notice")
                    }
                }
                // A ruling recorded or kept moves on the list's own spring, as a
                // board row does.
                .animation(.default, value: plan.rulings.map(PlanWords.rulingSignature))
            }
            if !past.isEmpty {
                Section {
                    // A row of its own rather than the header's button: a tap on
                    // a list section's header didn't open it under XCUITest.
                    Button { withAnimation { pastOpen.toggle() } } label: {
                        PlanHeader(title: PlanWords.pastDecisions, count: past.count, open: pastOpen)
                            .frame(minHeight: 44)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("\(PlanWords.pastDecisions), \(past.count)")
                    .accessibilityValue(pastOpen ? "Expanded" : "Collapsed")
                    .accessibilityIdentifier("plan-rulings-past-header")
                    if pastOpen {
                        ForEach(past) { ruling in
                            PlanSettledRulingRow(ruling: ruling, copy: copy)
                        }
                    }
                }
            }
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

/// An open ruling: its short id and decision, why, what reversing costs, and
/// what it touches. Swipe for Keep, Reverse and Discuss.
struct PlanRulingRow: View {
    let ruling: PlanRuling
    var actions = PhoneRulingActions()
    let copy: (String) -> Void
    @Binding var notice: String?
    /// Reverse asks first (ruling R-18): it sends the orchestrator off to change
    /// things. Keep and Discuss don't.
    @State private var confirming = false

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
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            if actions.canMark {
                // Gray, as everywhere on the phone: color is for what needs
                // attention.
                Button(PlanWords.keepRuling, systemImage: "checkmark") { actions.keep(ruling) }
                    .tint(.gray)
                Button(PlanWords.reverseRuling, systemImage: "arrow.uturn.backward") { confirming = true }
                    .tint(.gray)
                    .disabled(!actions.canAsk)
                Button(PlanWords.discussRuling, systemImage: "text.bubble") { ask(actions.discuss) }
                    .tint(.gray)
                    .disabled(!actions.canAsk)
            }
        }
        .contextMenu {
            if actions.canMark {
                Button(PlanWords.keepRuling, systemImage: "checkmark") { actions.keep(ruling) }
                Button(PlanWords.reverseRuling, systemImage: "arrow.uturn.backward") { confirming = true }
                    .disabled(!actions.canAsk)
                Button(PlanWords.discussRuling, systemImage: "text.bubble") { ask(actions.discuss) }
                    .disabled(!actions.canAsk)
            }
            Button(PlanWords.copyReference, systemImage: "doc.on.doc") { copy(ruling.reference) }
        }
        .confirmationDialog(RulingActions.confirmTitle(ruling), isPresented: $confirming, titleVisibility: .visible) {
            Button(PlanWords.reverseRuling) { ask(actions.reverse) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(RulingActions.confirmMessage(ruling))
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.rulingAccessibility(ruling))
        .accessibilityAction(named: PlanWords.copyReference) { copy(ruling.reference) }
        .accessibilityActions {
            if actions.canMark {
                Button(PlanWords.keepRuling) { actions.keep(ruling) }
                Button(PlanWords.reverseRuling) { confirming = true }.disabled(!actions.canAsk)
                Button(PlanWords.discussRuling) { ask(actions.discuss) }.disabled(!actions.canAsk)
            }
        }
        .accessibilityIdentifier("plan-ruling-\(ruling.short)")
    }

    private func ask(_ act: @escaping (PlanRuling) async -> String?) {
        Task { @MainActor in notice = await act(ruling) }
    }

    private func line(_ label: String, _ text: String) -> some View {
        (Text("\(label): ").fontWeight(.medium) + Text(text))
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// A kept or reversed ruling: one quiet line, with its note.
struct PlanSettledRulingRow: View {
    let ruling: PlanRuling
    let copy: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("\(ruling.short)  \(PlanWords.rulingSettled(ruling)) · \(ruling.decision)")
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
