import AgentKit
import SwiftUI

/// The native view of a terminal-mode claude pane (ov-372): its rows as the
/// runner's projector folds them, newest at the bottom, and a box to type
/// into.
///
/// A lazy list keyed by row id, so a long session builds only the rows on
/// screen and a row's change re-renders that row alone. Older rows are paged
/// in as the top comes into view.
struct NativeAgentView: View {
    @ObservedObject var model: NativePaneModel
    let isFocused: Bool
    let showTerminal: () -> Void

    var body: some View {
        let store = model.store
        ScrollView {
            LazyVStack(alignment: .leading, spacing: Spacing.inset) {
                if store.moreBefore {
                    ProgressView()
                        .controlSize(.small)
                        .frame(maxWidth: .infinity)
                        .onAppear { model.loadOlder() }
                }
                let last = store.ids.last
                ForEach(store.ids, id: \.self) { id in
                    if let box = store.box(id) {
                        NativeRowView(box: box, isLast: id == last, showTerminal: showTerminal)
                    }
                }
                ForEach(model.queued.indices, id: \.self) { i in
                    QueuedLine(text: model.queued[i])
                }
                if model.issue == .handoff {
                    HandoffRow(reason: "Claude is showing something only the terminal can.", showTerminal: showTerminal)
                }
            }
            .padding(Spacing.section)
        }
        // Clear of the switch that floats in the pane's top corner.
        .contentMargins(.top, 36, for: .scrollContent)
        .defaultScrollAnchor(.bottom)
        .scrollEdgeEffectStyle(.soft, for: .bottom)
        .overlay {
            if store.ids.isEmpty { emptyState(store.phase) }
        }
        // Rows held and the runner not answering: said over them, so stale
        // rows never pass for live ones, and the box waits (`canSend`).
        .overlay(alignment: .top) {
            if !store.ids.isEmpty, store.isStale { staleBanner(store.phase) }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            NativeComposer(model: model, isFocused: isFocused, showTerminal: showTerminal)
                .padding(Spacing.inset)
        }
        .background(WorkspaceStyle.paper)
        .onChange(of: store.ids.count) { _, _ in model.settleQueued() }
        .identified("native-agent-view")
    }

    private func staleBanner(_ phase: AgentRowStore.Phase) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Text(phase == .unavailable
                ? "This session isn’t being read anymore. The terminal has it."
                : "Can’t reach the runner, so this may be out of date. Trying again…")
            Spacer(minLength: Spacing.group)
            Button("Show Terminal", action: showTerminal)
        }
        .font(.callout)
        .padding(.horizontal, Spacing.inset)
        .padding(.vertical, Spacing.group)
        .attentionSurface(in: .card)
        .surface(.content, in: .card)
        .padding(.horizontal, Spacing.section)
        .padding(.top, 40)
        .identified("native-stale")
    }

    @ViewBuilder
    private func emptyState(_ phase: AgentRowStore.Phase) -> some View {
        switch phase {
        case .loading, .cached: ProgressView().controlSize(.small)
        case .live: Text("Nothing in this session yet.").foregroundStyle(.secondary)
        case .unavailable: Text("This pane’s session can’t be shown here. Use the terminal.").foregroundStyle(.secondary)
        case .trouble: Text("Can’t reach the runner. Trying again…").foregroundStyle(.secondary)
        }
    }
}

/// The native view's box (ov-372). Enter sends through `terminal.compose`:
/// typed into claude's own box and submitted, or taken by claude's queue
/// while it works (R-29).
///
/// Seams for what comes later, each named where it will go:
/// - images and attachments (ov-367): an accessory before the field, sent as
///   `compose_into`'s paths;
/// - slash commands (ov-367): refused here today (`NativePaneModel.send`),
///   the picker to be driven from this field;
/// - interrupt and Send Now (ov-368): a button beside Send while the turn
///   runs, and on each Queued row.
struct NativeComposer: View {
    @ObservedObject var model: NativePaneModel
    let isFocused: Bool
    let showTerminal: () -> Void
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let issue = model.issue, issue != .handoff {
                issueLine(issue)
            }
            HStack(alignment: .bottom, spacing: Spacing.group) {
                TextField("Message Claude", text: $model.draft, axis: .vertical)
                    .textFieldStyle(.plain)
                    .lineLimit(1...8)
                    .focused($focused)
                    .onSubmit { Task { await model.send() } }
                    .identified("native-composer")
                Button {
                    Task { await model.send() }
                } label: {
                    // A filled circle either way, so it reads on light paper
                    // when it can't send yet as well as when it can.
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.title2)
                        .foregroundStyle(model.canSend ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                }
                .buttonStyle(.plain)
                .disabled(!model.canSend)
                .help("Send")
                .accessibilityLabel("Send")
            }
            .padding(.horizontal, Spacing.inset)
            .padding(.vertical, Spacing.group)
            .surface(.floating, in: .floating)
        }
        .onChange(of: isFocused, initial: true) { _, now in focused = now }
    }

    @ViewBuilder
    private func issueLine(_ issue: NativePaneModel.SendIssue) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            switch issue {
            case .draftInTerminal:
                Text("The terminal’s box already holds a draft. Send or clear it there first.")
                Spacer(minLength: Spacing.group)
                Button("Show Terminal", action: showTerminal)
            case .said(let words):
                Text(words)
                Spacer(minLength: Spacing.group)
            case .handoff:
                EmptyView()
            }
            Button("Dismiss") { model.issue = nil }
                .buttonStyle(.link)
        }
        .font(.callout)
        .padding(.horizontal, Spacing.inset)
        .identified("native-send-issue")
    }
}
