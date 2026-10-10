import SwiftUI

extension EnvironmentValues {
    /// Opens a subagent's own conversation in the pane's place, by its row id
    /// and `agentId` (ov-453); nil where the runner can't.
    @Entry var nativeOpenAgent: ((String, String?) -> Void)? = nil
}

/// The agents at work in this pane, pinned above the composer while any
/// subagent runs (ov-453), as claude's own panel lists them under its box:
/// "main", then each running agent with its type, what it was asked, its
/// newest call, how long it has run and the tokens its newest call used.
/// The same list as the Mac's (`NativeAgentTray.swift` there), from the same
/// AgentKit words (`AgentTray`).
///
/// The inline subagent rows stay in the transcript as history; this doesn't
/// scroll away. Its header folds it to one line. An agent the runner can
/// open opens to its own conversation; main, then, goes back.
struct NativeAgentTray: View {
    @ObservedObject var model: NativePaneModel
    @Environment(\.verticalSizeClass) private var height

    /// The most agents listed before the rest are counted: fewer in a
    /// phone held sideways, where five would cover the conversation.
    private var listed: Int { height == .compact ? 2 : 5 }

    var body: some View {
        let drill = model.drill
        let entries = AgentTray.entries(model.store)
        if !entries.isEmpty {
            VStack(alignment: .leading, spacing: Spacing.tight) {
                Button {
                    withAnimation(.snappy(duration: 0.2)) { drill.collapsed.toggle() }
                } label: {
                    HStack(spacing: Spacing.group) {
                        Image(systemName: "chevron.right")
                            .font(.caption2.weight(.semibold))
                            .rotationEffect(.degrees(drill.collapsed ? 0 : 90))
                            .frame(width: 16)
                        Text(AgentTray.summary(entries))
                        Spacer(minLength: 0)
                    }
                    .font(.footnote.weight(.medium))
                    .foregroundStyle(.secondary)
                    .frame(minHeight: 28)
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(AgentTray.summary(entries))
                .accessibilityValue(drill.collapsed ? "Collapsed" : "Expanded")
                .accessibilityIdentifier("native-agent-tray-header")
                if !drill.collapsed {
                    ForEach(entries.prefix(listed + 1)) { entry in
                        row(entry, drill: drill)
                    }
                    if entries.count > listed + 1 {
                        Text("\(entries.count - listed - 1) more running")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .padding(.leading, 16 + Spacing.group)
                    }
                }
            }
            .padding(.horizontal, Spacing.inset)
            .padding(.vertical, Spacing.group)
            .surface(.floating, in: .floating)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("native-agent-tray")
        }
    }

    @ViewBuilder
    private func row(_ entry: AgentTray.Entry, drill: AgentDrill) -> some View {
        let opens = !entry.isMain && model.opensAgents && entry.agentId != nil
        let acts = opens || (entry.isMain && drill.opened != nil)
        let selected = !entry.isMain && drill.opened?.id == entry.id
        Button {
            if entry.isMain { model.closeAgent() } else { model.openAgent(row: entry.id, agentId: entry.agentId) }
        } label: {
            NativeAgentTrayRow(entry: entry, opens: opens)
                .padding(.vertical, Spacing.tight)
                .padding(.horizontal, Spacing.tight)
                .background(selected ? Fill.selection(active: true) : Color.clear, in: .control)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        // Not `disabled`, which would gray out main and every agent an older
        // runner can't open: a row that does nothing just takes no tap.
        .allowsHitTesting(acts)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(AgentTray.spoken(entry))
        .accessibilityAddTraits(acts ? .isButton : [])
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier("native-agent-tray-\(entry.id)")
    }
}

/// One agent in the tray: a spinner while it runs, its type and what it was
/// asked, its run time and tokens; under that, its newest call.
private struct NativeAgentTrayRow: View {
    let entry: AgentTray.Entry
    let opens: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            // Centered, not on the baseline: a spinner has none, and sat
            // below the words.
            HStack(alignment: .center, spacing: Spacing.group) {
                Group {
                    if entry.running {
                        ProgressView().controlSize(.mini)
                    } else {
                        Image(systemName: "circle").font(.caption2).foregroundStyle(.tertiary)
                    }
                }
                .frame(width: 16)
                Text(entry.title)
                    .fontWeight(.medium)
                    .foregroundStyle(.primary)
                    .fixedSize()
                Text(entry.isMain || entry.description.isEmpty ? entry.action : entry.description)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(-1)
                Spacer(minLength: Spacing.tight)
                HStack(spacing: Spacing.tight) {
                    if entry.startedMs != nil, entry.running || entry.endedMs != nil {
                        NativeRunTime(startedMs: entry.startedMs, endedMs: entry.endedMs)
                    }
                    // A phone's row has room for the number alone.
                    if let tokens = entry.tokens {
                        Text("·").foregroundStyle(.tertiary)
                        Text(AgentTray.tokens(tokens, short: true))
                    }
                }
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
                .fixedSize()
                Image(systemName: "chevron.right")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .opacity(opens ? 1 : 0)
            }
            if !entry.isMain, !entry.description.isEmpty, !entry.action.isEmpty {
                Text(entry.action)
                    .font(.caption.monospaced())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .padding(.leading, 16 + Spacing.group)
            }
        }
        .font(MachineryStyle.font)
        .foregroundStyle(.secondary)
    }
}

/// A subagent's own conversation in the pane's place (ov-453): the way back
/// and which agent this is at the top, its rows drawn as the pane's are, and
/// the tray below to go to another. Nothing is sent to an agent, so there is
/// no composer.
struct NativeAgentConversation: View {
    @ObservedObject var model: NativePaneModel
    let opened: AgentDrill.Opened
    let showTerminal: () -> Void

    var body: some View {
        let store = opened.store
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
            .padding(Spacing.section)
        }
        .id(store.key)
        .defaultScrollAnchor(.bottom)
        .accessibilityIdentifier("native-agent-transcript")
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
        .safeAreaInset(edge: .top, spacing: 0) { header }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            NativeAgentTray(model: model)
                .padding(.horizontal, Spacing.section)
                .padding(.bottom, Spacing.group)
        }
        .background(Surface.contentFill.ignoresSafeArea())
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
                    Text(AgentTray.agentType(sub.agentType)).fontWeight(.semibold)
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
        .padding(.horizontal, Spacing.section)
        .padding(.vertical, Spacing.tight)
        .background(Surface.contentFill)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("native-agent-header")
    }
}
