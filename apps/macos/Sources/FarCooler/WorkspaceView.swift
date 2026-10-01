import SwiftUI

/// A workspace in the detail, at one of two levels (spec §4.3): its
/// orchestrator's conversation beside its board, or, drilled into a task or
/// a worktree, that alone with the conversation shrunk to a rail beside it.
///
/// Which is drawn is `WorkspaceColumns.layout`'s answer for the detail's own
/// width, measured here and never read from the window, which the detail
/// shares with the sidebar. The contents are the window's: each is handed in
/// whole, so this view decides where things go and nothing about what they
/// are.
struct WorkspaceView<Conversation: View, Rail: View, Board: View, Drilled: View>: View {
    /// Whether a task or a worktree is open: the drilled level.
    let drilled: Bool
    /// Whether this workspace has a conversation column at all: not a
    /// repository's implicit workspace on a runner without `workstreams`.
    let hasConversation: Bool
    /// The terminal font's cell width: the minimums are in columns.
    let cell: CGFloat
    /// Focus (⌃⌘↩): what's opened alone, without the rail.
    let focused: Bool
    /// The conversation popped open over what's opened.
    let peek: Bool
    /// Which column the one-column form shows.
    @Binding var pick: WorkspacePick
    @ViewBuilder let conversation: () -> Conversation
    @ViewBuilder let rail: () -> Rail
    @ViewBuilder let board: () -> Board
    @ViewBuilder let opened: () -> Drilled

    var body: some View {
        GeometryReader { proxy in
            let arrangement = WorkspaceColumns.layout(
                width: proxy.size.width, drilled: drilled, cell: cell, hasConversation: hasConversation,
                focused: focused, peek: peek)
            // The workspace level stays drawn, hidden, while drilled in, so
            // the board keeps its place, its scroll and its visit (the
            // summary's "last visit" is when you left the workspace, not a
            // task) for Back. Never the conversation: one terminal view per
            // pane, and it's the rail's to pop open.
            let base = WorkspaceColumns.layout(
                width: proxy.size.width, drilled: false, cell: cell, hasConversation: hasConversation)
            ZStack {
                workspaceLevel(base)
                    .opacity(drilled ? 0 : 1)
                    .allowsHitTesting(!drilled)
                    .accessibilityHidden(drilled)
                if arrangement.drilled {
                    drilledLevel(arrangement, width: proxy.size.width)
                }
            }
            .frame(width: proxy.size.width, height: proxy.size.height)
            .preference(key: WorkspaceArrangementPreference.self, value: arrangement)
            .preference(key: WorkspaceWidthPreference.self, value: proxy.size.width)
        }
    }

    private func drilledLevel(_ arrangement: WorkspaceColumns.Arrangement, width: CGFloat) -> some View {
        HStack(spacing: 0) {
            if arrangement.conversation != .none {
                rail()
                    .frame(width: WorkspaceColumns.rail)
                    .frame(maxHeight: .infinity)
                Divider()
            }
            // Over what's opened, never beside it: popping the
            // conversation open doesn't resize a task's terminals, or
            // their tmux windows for every other client.
            opened()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .overlay(alignment: .leading) {
                    if arrangement.conversation == .peek {
                        HStack(spacing: 0) {
                            conversation()
                                .frame(width: WorkspaceColumns.peekWidth(in: width, cell: cell))
                                .frame(maxHeight: .infinity)
                                .background(WorkspaceStyle.canvas)
                            Divider()
                        }
                        .compositingGroup()
                        .shadow(color: .black.opacity(0.18), radius: 8, x: 2)
                        .transition(.move(edge: .leading).combined(with: .opacity))
                        .accessibilityIdentifier("workspace-conversation-peek")
                    }
                }
                .accessibilityIdentifier("workspace-opened")
        }
        .background(WorkspaceStyle.canvas)
    }

    @ViewBuilder
    private func workspaceLevel(_ arrangement: WorkspaceColumns.Arrangement) -> some View {
        if arrangement.switcher {
            VStack(spacing: 0) {
                Picker("Show", selection: $pick) {
                    ForEach(WorkspacePick.allCases) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity)
                .background(WorkspaceStyle.canvas)
                Divider()
                Group {
                    if pick == .board {
                        board()
                    } else if !drilled {
                        conversation()
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .accessibilityIdentifier("workspace-one-column")
        } else {
            HSplitView {
                if arrangement.conversation == .column && !drilled {
                    conversation()
                        .frame(
                            minWidth: WorkspaceColumns.conversationMinimum(cell: cell),
                            maxWidth: .infinity, maxHeight: .infinity)
                        .layoutPriority(1)
                        .accessibilityIdentifier("workspace-conversation")
                }
                board()
                    .frame(
                        minWidth: WorkspaceColumns.boardMinimum,
                        idealWidth: WorkspaceColumns.boardIdeal,
                        maxWidth: arrangement.conversation == .none ? .infinity : nil,
                        maxHeight: .infinity)
                    .accessibilityIdentifier("workspace-board")
            }
        }
    }
}

/// The breadcrumb over a task or a worktree opened: Back, then each level up
/// to the one you're at, every one but that a way back to it.
struct DrillBreadcrumb: View {
    let crumbs: [WorkspaceNavigation.Crumb]
    var onGo: (ContentView.Selection) -> Void
    var onBack: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Button(action: onBack) {
                Image(systemName: "chevron.left")
            }
            .buttonStyle(.borderless)
            .help("Back (⌃⌘←)")
            .accessibilityLabel("Back")
            ForEach(Array(crumbs.enumerated()), id: \.offset) { index, crumb in
                if index > 0 { Text("›").foregroundStyle(.tertiary) }
                if let target = crumb.target {
                    Button(crumb.title) { onGo(target) }
                        .buttonStyle(.plain)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .help("Go to \(crumb.title)")
                } else {
                    Text(crumb.title)
                        .fontWeight(.semibold)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            Spacer(minLength: 0)
        }
        .font(.system(size: 12))
        .padding(.horizontal, 12)
        .frame(height: 30)
        .background(WorkspaceStyle.canvas)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Breadcrumb")
    }
}

/// The one-column form's Orchestrator | Board control.
enum WorkspacePick: String, CaseIterable, Identifiable {
    case orchestrator, board
    var id: String { rawValue }
    var title: String { self == .orchestrator ? "Orchestrator" : "Board" }
}

/// The arrangement a `WorkspaceView` drew, published from the value it
/// switched on, so a test can read which columns a width got.
struct WorkspaceArrangementPreference: PreferenceKey {
    static let defaultValue: WorkspaceColumns.Arrangement? = nil
    static func reduce(value: inout WorkspaceColumns.Arrangement?, nextValue: () -> WorkspaceColumns.Arrangement?) {
        value = nextValue() ?? value
    }
}

/// The detail's width as a `WorkspaceView` measured it: what the window reads
/// to say which of a workspace's columns are on screen.
struct WorkspaceWidthPreference: PreferenceKey {
    static let defaultValue: CGFloat? = nil
    static func reduce(value: inout CGFloat?, nextValue: () -> CGFloat?) {
        value = nextValue() ?? value
    }
}
