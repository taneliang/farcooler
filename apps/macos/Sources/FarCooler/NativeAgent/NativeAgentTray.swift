import AgentKit
import SwiftUI

/// The agents at work in this pane, pinned above the composer while any
/// subagent runs (ov-453), as claude's own panel lists them under its box:
/// "main", then each running agent with its type, what it was asked, its
/// newest call, how long it has run and the tokens its newest call used.
///
/// The inline subagent rows stay in the transcript as history; this doesn't
/// scroll away. Its header folds it to one line. An agent the runner can open
/// opens to its own conversation in the pane's place; main, chosen then, goes
/// back.
struct NativeAgentTray: View {
    let store: AgentRowStore
    let drill: AgentDrill
    /// Whether an agent opens to its own rows here (`subagent_rows`).
    let opens: Bool
    let open: (AgentTray.Entry) -> Void
    let back: () -> Void

    /// The most agents listed before the rest are counted.
    static let listed = 6

    var body: some View {
        let entries = AgentTray.entries(store)
        if !entries.isEmpty {
            VStack(alignment: .leading, spacing: Spacing.tight) {
                header(entries)
                if !drill.collapsed {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(entries.prefix(Self.listed + 1)) { entry in
                            row(entry)
                        }
                        if entries.count > Self.listed + 1 {
                            Text("\(entries.count - Self.listed - 1) more running")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .padding(.leading, 14 + Spacing.group * 2)
                                .padding(.vertical, Spacing.tight)
                        }
                    }
                }
            }
            .padding(Spacing.group)
            .surface(.floating, in: .floating)
            .identified("native-agent-tray")
        }
    }

    private func header(_ entries: [AgentTray.Entry]) -> some View {
        Button {
            withAnimation(.snappy(duration: 0.2)) { drill.collapsed.toggle() }
        } label: {
            HStack(spacing: Spacing.group) {
                Image(systemName: "chevron.right")
                    .font(.caption2.weight(.semibold))
                    .rotationEffect(.degrees(drill.collapsed ? 0 : 90))
                    .frame(width: 14)
                Text(AgentTray.summary(entries))
                Spacer(minLength: 0)
            }
            .font(.caption.weight(.medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, Spacing.group)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .help(drill.collapsed ? "Show Agents" : "Hide Agents")
        .accessibilityLabel(AgentTray.summary(entries))
        .accessibilityValue(drill.collapsed ? "Collapsed" : "Expanded")
        .accessibilityHint(drill.collapsed ? "Shows the agents" : "Hides the agents")
        .identified("native-agent-tray-header")
    }

    @ViewBuilder
    private func row(_ entry: AgentTray.Entry) -> some View {
        let selected = entry.isMain ? false : drill.opened?.id == entry.id
        let opensHere = !entry.isMain && opens && entry.agentId != nil
        let acts = opensHere || (entry.isMain && drill.opened != nil)
        Button {
            if entry.isMain { back() } else { open(entry) }
        } label: {
            NativeAgentTrayRow(entry: entry, opens: opensHere)
                .padding(.horizontal, Spacing.group)
                .padding(.vertical, Spacing.tight)
                .background(selected ? Fill.selection(active: true) : Color.clear, in: .control)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        // Not `disabled`, which would gray out main and every agent an
        // older runner can't open: a row that does nothing just takes no
        // click.
        .allowsHitTesting(acts)
        .help(entry.isMain ? (drill.opened != nil ? "Back to the Conversation" : "") : (opensHere ? "Open This Agent’s Conversation" : ""))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(AgentTray.spoken(entry))
        .accessibilityAddTraits(acts ? .isButton : [])
        .accessibilityAddTraits(selected ? .isSelected : [])
        .identified("native-agent-tray-\(entry.id)")
    }
}

/// One agent in the tray: a spinner while it runs, its type, what it was
/// asked and, under that, its newest call; its run time and its tokens at the
/// trailing edge.
private struct NativeAgentTrayRow: View {
    let entry: AgentTray.Entry
    let opens: Bool
    /// Room for the tokens' word: a narrow row gives it to what the agent
    /// was asked.
    @State private var roomy = true

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                Group {
                    if entry.running {
                        ProgressView().controlSize(.mini)
                    } else {
                        Image(systemName: "circle").font(.caption2).foregroundStyle(.tertiary)
                    }
                }
                .frame(width: 14)
                Text(entry.title)
                    .fontWeight(.medium)
                    .foregroundStyle(.primary)
                    .fixedSize()
                Text(entry.isMain || entry.description.isEmpty ? entry.action : entry.description)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(-1)
                Spacer(minLength: Spacing.group)
                figures(words: roomy)
                Image(systemName: "chevron.right")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .opacity(opens ? 1 : 0)
                    .accessibilityHidden(true)
            }
            if !entry.isMain, !entry.description.isEmpty, !entry.action.isEmpty {
                Text(entry.action)
                    .font(.caption.monospaced())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .padding(.leading, 14 + Spacing.group)
            }
        }
        .font(MachineryStyle.font)
        .foregroundStyle(.secondary)
        .onGeometryChange(for: Bool.self) { $0.size.width > 520 } action: { roomy = $0 }
    }

    private func figures(words: Bool) -> some View {
        HStack(spacing: Spacing.tight) {
            if entry.startedMs != nil, entry.running || entry.endedMs != nil {
                RunTimeChip(startedMs: entry.startedMs, endedMs: entry.endedMs)
            }
            if let tokens = entry.tokens {
                Text("·").foregroundStyle(.tertiary)
                Text(words ? AgentTray.tokens(tokens) : AgentTray.tokens(tokens, short: true))
            }
        }
        .font(.caption.monospacedDigit())
        .foregroundStyle(.secondary)
        .fixedSize()
    }
}

/// Over a subagent's own conversation (ov-453): the way back, ⌘[ as in any
/// Mac app that goes back, and which agent this is.
struct NativeAgentHeader: View {
    let subagent: AgentRow.Subagent?
    let isFocused: Bool
    let back: () -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            Button(action: back) {
                Label("Back", systemImage: "chevron.backward")
            }
            .keyboardShortcut(isFocused ? KeyboardShortcut("[", modifiers: .command) : nil)
            .help("Back to the Conversation (⌘[)")
            .identified("native-agent-back")
            if let subagent {
                NativeStatusMark(status: subagent.status)
                Text(AgentTray.agentType(subagent.agentType))
                    .fontWeight(.medium)
                    .fixedSize()
                Text(subagent.description)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(-1)
                Spacer(minLength: Spacing.group)
                RunTimeChip(startedMs: subagent.startedMs, endedMs: subagent.status == .running ? nil : (subagent.endedMs ?? subagent.lastMs))
            } else {
                Spacer(minLength: 0)
            }
        }
        .font(.callout)
        .identified("native-agent-header")
    }
}
