import AgentKit
import SwiftUI

/// The board's header: its title, what's waiting, and the controls.
///
/// One row on the board column's grid (ov-83): the workspace's title at
/// column B, over the sections' titles below it, and the controls at the
/// trailing edge, each the same 24 pt square — New Task… and Refresh, both
/// icons. (A list/kanban toggle sat between them until the owner removed the
/// kanban; Refresh was a bordered word button beside it, and the row read as
/// crowded.) ⌘R reloads the fleet, from the menu bar, as before.
///
/// A board can be as narrow as `WorkspaceColumns.boardMinimum`, so the header
/// can't assume its sentences fit (ov-81 P1). It offers three arrangements,
/// and `ViewThatFits` takes the first that fits whole: the waiting count and
/// any trouble as sentences, then said short, then the title and one overflow
/// menu. The last always fits, because its title is the one thing allowed to
/// truncate.
struct BoardHeader: View {
    let title: String
    let waiting: Int
    let reading: Bool
    /// What a failed re-read says, when one failed over a board already read.
    let trouble: String?
    let offersWrites: Bool
    @Binding var newTaskOpen: Bool
    let onCreate: (String) async -> Bool
    let onRefresh: () -> Void

    /// Every control's square: the sidebar's, so the app has one size.
    static let control: CGFloat = SidebarGrid.control

    var body: some View {
        ViewThatFits(in: .horizontal) {
            row(.full)
            row(.short)
            row(.overflow)
        }
        // Title at column B; the controls end at column A's distance from
        // the trailing edge.
        .padding(.leading, ColumnGrid.b)
        .padding(.trailing, ColumnGrid.a)
        .padding(.vertical, ColumnGrid.rhythm)
        .background(WorkspaceStyle.paneChrome)
        // On the header rather than on the button, so the same form opens
        // from the plus button and from the overflow menu's item.
        .popover(isPresented: $newTaskOpen, arrowEdge: .bottom) {
            NewTaskForm(onCreate: onCreate, onClose: { newTaskOpen = false })
        }
    }

    /// How much a header can afford to say.
    enum Level { case full, short, overflow }

    private func row(_ level: Level) -> some View {
        HStack(spacing: SidebarGrid.gap) {
            Text(title).font(WorkspaceStyle.sectionTitle).lineLimit(1).truncationMode(.tail)
                .gridMark("header", .text)
            // The one count worth putting in a title bar, and the sentence is
            // the model's like every other one here. Nothing when nothing is
            // waiting — `waitingSentence` is nil at zero, because a badge
            // reading zero teaches people to ignore it.
            //
            // One line always: at a narrow board the sentence wrapped a word
            // to a line into a tall lozenge (checklist F2). Where the whole
            // sentence doesn't fit, it's said short.
            if let sentence = TaskBoardModel.waitingSentence(waiting) {
                if level == .full {
                    ViewThatFits(in: .horizontal) {
                        waitingPill(sentence)
                        waitingPill(Self.waitingShort(waiting))
                    }
                    .layoutPriority(1)
                    .help(sentence)
                } else {
                    waitingPill(Self.waitingShort(waiting)).layoutPriority(1).help(sentence)
                }
            }
            Spacer(minLength: 0)
            if reading { ProgressView().controlSize(.small) }
            // Shown beside the board rather than over it: a failed re-read
            // leaves the last good board on screen, and hiding it behind an
            // error would cost more than the error is worth. Narrower, it's a
            // mark with the sentence for its tooltip.
            if let trouble {
                if level == .full {
                    Text(trouble)
                        .font(.system(size: WorkspaceStyle.PaneText.secondary))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else {
                    Image(systemName: "exclamationmark.triangle")
                        .foregroundStyle(.secondary)
                        .help(trouble)
                        .accessibilityLabel(trouble)
                }
            }
            switch level {
            case .full, .short:
                HStack(spacing: 2) {
                    if offersWrites {
                        iconButton("plus", help: "New Task…") { newTaskOpen = true }
                            .accessibilityIdentifier("board-new-task")
                    }
                    iconButton("arrow.clockwise", help: "Refresh", action: onRefresh)
                        .accessibilityIdentifier("board-refresh")
                }
            case .overflow:
                overflowMenu
            }
        }
        .frame(minHeight: Self.control)
    }

    /// A control: its glyph in a `control`-point square, borderless, so
    /// every one is the same size whatever glyph is inside it.
    private func iconButton(
        _ symbol: String, help: String, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .medium))
                .frame(width: Self.control, height: Self.control)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .help(help)
        .accessibilityLabel(help)
    }

    /// Everything the header does, in one menu, for a board with no room.
    private var overflowMenu: some View {
        Menu {
            if offersWrites {
                Button("New Task…") { newTaskOpen = true }
            }
            Button("Refresh", action: onRefresh)
        } label: {
            Image(systemName: "ellipsis.circle")
                .frame(width: Self.control, height: Self.control)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Board Actions")
        .accessibilityLabel("Board Actions")
        .accessibilityIdentifier("board-overflow")
    }

    private func waitingPill(_ text: String) -> some View {
        Text(text)
            .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .semibold))
            .lineLimit(1)
            .fixedSize()
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .background(Color.accentColor.opacity(0.18), in: Capsule())
    }

    /// The waiting pill's short form, for a board too narrow for the
    /// sentence: "2 waiting".
    static func waitingShort(_ count: Int) -> String { "\(count) waiting" }
}
