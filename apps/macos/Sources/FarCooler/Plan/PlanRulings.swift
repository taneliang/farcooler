import AgentKit
import AppKit
import SwiftUI

// Decided for you (ov-304): the calls the orchestrator made on the owner's
// behalf, at the foot of the plan canvas (Concept A, `PlanHome`). Only the
// open ones, newest first, each with its short id, what was decided, why and
// what reversing costs (ov-333). The owner acts on each: Keep, Reverse or
// Discuss, on hover and on the context menu, and Keep All on the section. The
// kept and reversed ones fold into Past Decisions, a line each, closed until
// opened.
//
// Keep is the owner's own mark and never reaches the orchestrator. Reverse
// sends it the ruling's recorded reversal; Discuss quotes the ruling into its
// composer, unsent (`PlanRulingActions`, `ContentView.planRulingActions`). No
// count, no notification: rulings ask nothing of the owner. A runner without
// `board_rulings` shows nothing, and neither does a board with no rulings.

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
                standing: model.openRulings, settled: model.pastRulings,
                key: "board.plan.section.rulings.\(plan.host).\(plan.workspace.id)", defaults: defaults, copy: copy)
        }
    }
}

/// The sections themselves, given their rulings: what the canvas draws and a
/// test renders. Decided For You holds the open ones and Past Decisions folds
/// the rest; each appears only when it has something.
struct PlanRulingsList: View {
    let standing: [PlanRuling]
    let settled: [PlanRuling]
    let key: String
    var defaults: UserDefaults = .standard
    var copy: @MainActor (String) -> Void = PlanRulingsList.toPasteboard

    @Environment(\.planRulingActions) private var actions

    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.section) {
            if !standing.isEmpty { open }
            if !settled.isEmpty { past }
        }
    }

    private var open: some View {
        CollapsibleSection(
            PlanWords.decidedForYou, id: "plan.rulings", style: .navigator, key: key, defaults: defaults,
            count: standing.count,
            accessory: {
                // Keep All, once there's more than one to keep: with one, its
                // own Keep says the same.
                if actions.canKeep, standing.count > 1 {
                    Button(PlanWords.keepAllRulings) { actions.keepAll() }
                        .buttonStyle(.borderless)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .help("Keep every open ruling")
                        .padding(.trailing, NavigatorGrid.gap * 2)
                        .identified("plan-rulings-keep-all")
                }
            }
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(standing) { ruling in
                    PlanRulingRow(ruling: ruling, copy: copy)
                        .changeWashed(ruling.id)
                }
            }
            .listChanges(standing.map { ListChangeRow(id: $0.id, signature: PlanWords.rulingSignature($0)) })
        }
        .identified("plan-rulings")
    }

    /// Kept and reversed, most recently settled first, closed until opened:
    /// the history the owner reads when they want to, never a list that grows
    /// in front of them.
    private var past: some View {
        CollapsibleSection(
            PlanWords.pastDecisions, id: "plan.rulings.past", style: .minor, key: key + ".past", defaults: defaults,
            expandedByDefault: false, count: settled.count
        ) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(settled) { ruling in
                    PlanSettledRulingRow(ruling: ruling, copy: copy)
                        .changeWashed(ruling.id)
                }
            }
            .listChanges(settled.map { ListChangeRow(id: $0.id, signature: PlanWords.rulingSignature($0)) })
        }
        .identified("plan-rulings-past")
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

/// An open ruling: its short id and decision, then why, what reversing costs
/// and what it touches. Keep, Reverse and Discuss show while the pointer is on
/// it, and are always on its context menu and its VoiceOver actions.
struct PlanRulingRow: View {
    let ruling: PlanRuling
    let copy: @MainActor (String) -> Void

    @Environment(\.planRulingActions) private var actions
    @State private var hovering = false

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
                    RulingRowActions(ruling: ruling)
                        .opacity(hovering || actions.alwaysShown ? 1 : 0)
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
        .onHover { hovering = $0 }
        .contextMenu {
            RulingMenu(ruling: ruling, copy: copy)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(PlanWords.rulingAccessibility(ruling))
        .accessibilityAction(named: PlanWords.copyReference) { copy(ruling.reference) }
        .accessibilityActions {
            if actions.canKeep {
                Button(PlanWords.keepRuling) { actions.keep(ruling) }
                Button(PlanWords.reverseRuling) { actions.reverse(ruling) }.disabled(!actions.canAsk)
                Button(PlanWords.discussRuling) { actions.discuss(ruling) }.disabled(!actions.canAsk)
            }
        }
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

/// A kept or reversed ruling: one quiet line, "R-1 Kept · Unread stays on the
/// phones.", with its note under it if there is one.
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
                Text("\(PlanWords.rulingSettled(ruling)) · \(ruling.decision)")
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

/// The owner's three actions on an open ruling, as the row's quiet buttons
/// (ov-333). Reverse and Discuss are off, never hidden, while the workspace
/// has no orchestrator to ask, and say what turns them on.
private struct RulingRowActions: View {
    let ruling: PlanRuling
    @Environment(\.planRulingActions) private var actions

    var body: some View {
        if actions.canKeep {
            HStack(spacing: Spacing.group) {
                Button(PlanWords.keepRuling) { actions.keep(ruling) }
                    .help("Keep this call. It stays as precedent for the orchestrator.")
                    .identified("plan-ruling-\(ruling.short)-keep")
                Button(PlanWords.reverseRuling) { actions.reverse(ruling) }
                    .disabled(!actions.canAsk)
                    .help(actions.canAsk ? "Ask the orchestrator to undo it" : PlanWords.rulingNeedsOrchestrator)
                    .identified("plan-ruling-\(ruling.short)-reverse")
                Button(PlanWords.discussRuling) { actions.discuss(ruling) }
                    .disabled(!actions.canAsk)
                    .help(actions.canAsk ? "Start a message to the orchestrator about it" : PlanWords.rulingNeedsOrchestrator)
                    .identified("plan-ruling-\(ruling.short)-discuss")
            }
            .buttonStyle(.borderless)
            .font(.system(size: WorkspaceStyle.PaneText.secondary))
            .foregroundStyle(.secondary)
        }
    }
}

/// The same actions on a ruling's context menu, with Copy Reference.
struct RulingMenu: View {
    let ruling: PlanRuling
    let copy: @MainActor (String) -> Void
    @Environment(\.planRulingActions) private var actions

    var body: some View {
        if actions.canKeep, ruling.isStanding {
            Button(PlanWords.keepRuling) { actions.keep(ruling) }
            Button(PlanWords.reverseRuling) { actions.reverse(ruling) }.disabled(!actions.canAsk)
            Button(PlanWords.discussRuling) { actions.discuss(ruling) }.disabled(!actions.canAsk)
            Divider()  // style-exempt: a menu divider, in the context menu RulingMenu builds
        }
        Button(PlanWords.copyReference) { copy(ruling.reference) }
    }
}
