import SwiftUI

/// A workspace in the detail: its orchestrator's conversation beside its
/// board, and a task (or a worktree opened whole) in a third column
/// (spec §4.3).
///
/// Which columns are drawn is `WorkspaceColumns.layout`'s answer for the
/// detail's own width, measured here and never read from the window, which
/// the detail shares with the sidebar. The columns' contents are the
/// window's: each is handed in whole, so this view decides where things go
/// and nothing about what they are.
struct WorkspaceView<Conversation: View, Rail: View, Board: View, Third: View>: View {
    /// Whether the third column is open.
    let taskOpen: Bool
    /// Whether this workspace has a conversation column at all: not a
    /// repository's implicit workspace on a runner without `workstreams`.
    let hasConversation: Bool
    /// The terminal font's cell width: the minimums are in columns.
    let cell: CGFloat
    /// Focus Column (⌃⌘↩): the third column over the other two.
    let focused: Bool
    /// Which column the one-column form shows.
    @Binding var pick: WorkspacePick
    @ViewBuilder let conversation: () -> Conversation
    @ViewBuilder let rail: () -> Rail
    @ViewBuilder let board: () -> Board
    @ViewBuilder let third: () -> Third

    var body: some View {
        GeometryReader { proxy in
            let arrangement = WorkspaceColumns.layout(
                width: proxy.size.width, taskOpen: taskOpen, cell: cell, hasConversation: hasConversation,
                focused: focused)
            columns(arrangement)
                .frame(width: proxy.size.width, height: proxy.size.height)
                .preference(key: WorkspaceArrangementPreference.self, value: arrangement)
        }
    }

    @ViewBuilder
    private func columns(_ arrangement: WorkspaceColumns.Arrangement) -> some View {
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
                    if pick == .orchestrator { conversation() } else { board() }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .accessibilityIdentifier("workspace-one-column")
        } else {
            HStack(spacing: 0) {
                if arrangement.conversation == .rail {
                    rail()
                        .frame(width: WorkspaceColumns.rail)
                        .frame(maxHeight: .infinity)
                    Divider()
                }
                HSplitView {
                    if arrangement.conversation == .column {
                        conversation()
                            .frame(
                                minWidth: WorkspaceColumns.conversationMinimum(cell: cell),
                                idealWidth: arrangement.task ? WorkspaceColumns.conversationMinimum(cell: cell) : nil,
                                maxWidth: .infinity, maxHeight: .infinity)
                            // It fills while nothing is open beside it; with a
                            // task open, the task does.
                            .layoutPriority(arrangement.task ? 0 : 1)
                            .accessibilityIdentifier("workspace-conversation")
                    }
                    if arrangement.board {
                        board()
                            .frame(
                                minWidth: WorkspaceColumns.boardMinimum,
                                idealWidth: WorkspaceColumns.boardIdeal,
                                maxWidth: arrangement.conversation == .none && !arrangement.task ? .infinity : nil,
                                maxHeight: .infinity)
                            .accessibilityIdentifier("workspace-board")
                    }
                    if arrangement.task {
                        third()
                            .frame(
                                minWidth: arrangement == .taskAlone ? nil : WorkspaceColumns.taskMinimum(cell: cell),
                                maxWidth: .infinity, maxHeight: .infinity)
                            .layoutPriority(1)
                            .accessibilityIdentifier("workspace-task")
                    }
                }
            }
        }
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
