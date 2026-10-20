import SwiftUI

extension EnvironmentValues {
    /// Opens a subagent's own conversation in the pane's place, by its row id
    /// and `agentId` (ov-453); nil where the runner can't.
    @Entry var nativeOpenAgent: ((String, String?) -> Void)? = nil
}

/// A subagent's own conversation in the pane's place (ov-453): the way back
/// and which agent this is at the top, and its rows drawn as the pane's are.
/// Another agent is a row of the agents view (ov-619). Nothing is sent to an
/// agent, so there is no composer.
struct NativeAgentConversation: View {
    @ObservedObject var model: NativePaneModel
    let opened: AgentDrill.Opened
    let showTerminal: () -> Void

    @Environment(\.screenMargin) private var margin

    var body: some View {
        let store = opened.store
        // The header is its own row above the rows, which are cut at its
        // hairline, not an inset they scroll under (ov-467).
        VStack(spacing: 0) {
            header
            transcript(store)
        }
        .background(Surface.contentFill.ignoresSafeArea())
        .environment(\.selectsRanges, true)
    }

    private func transcript(_ store: AgentRowStore) -> some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: Spacing.inset) {
                let last = store.items.last?.id
                ForEach(store.items) { item in
                    switch item {
                    case .row(let id):
                        if let box = store.box(id) {
                            NativeRowView(box: box, isLast: id == last, showTerminal: showTerminal)
                        }
                    case .tools(let id, let rows):
                        NativeToolGroupRow(id: id, boxes: rows.compactMap(store.box))
                    }
                }
            }
            .padding(.horizontal, margin)
            .padding(.vertical, Spacing.section)
        }
        .id(store.key)
        .defaultScrollAnchor(.bottom)
        .accessibilityIdentifier("native-agent-transcript")
        .clipped()
        .overlay {
            if store.shownIds.isEmpty {
                switch store.phase {
                case .loading, .cached: ProgressView()
                case .unavailable: Text(AgentTray.unopenable).foregroundStyle(.secondary).multilineTextAlignment(.center).padding()
                case .live: Text("This agent hasn’t written anything yet.").foregroundStyle(.secondary)
                case .trouble: Text("Can’t reach the runner. Trying again…").foregroundStyle(.secondary)
                }
            }
        }
    }

    private var header: some View {
        let sub: AgentRow.Subagent? = {
            if case .subagent(let sub)? = model.store.box(opened.id)?.row.kind { return sub }
            return nil
        }()
        return HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Button(action: model.closeAgent) {
                Label("Back", systemImage: "chevron.backward")
                    .labelStyle(.titleAndIcon)
                    .frame(minHeight: 44)
                    .contentShape(.rect)
            }
            .accessibilityIdentifier("native-agent-back")
            if let sub {
                VStack(alignment: .leading, spacing: 0) {
                    Text(AgentTray.agentType(sub.agentType)).fontWeight(.semibold).lineLimit(1)
                    if !sub.description.isEmpty {
                        Text(sub.description).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                .layoutPriority(-1)
                Spacer(minLength: Spacing.group)
                NativeRunTime(startedMs: sub.startedMs, endedMs: sub.status == .running ? nil : (sub.endedMs ?? sub.lastMs))
            } else {
                Spacer(minLength: 0)
            }
        }
        .padding(.horizontal, margin)
        .padding(.vertical, Spacing.tight)
        .background(Surface.contentFill)
        // style-exempt: the owner asked for a hairline under the pinned header (ov-467)
        .overlay(alignment: .bottom) { Divider() }
        .zIndex(1)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-agent-header")
    }
}
