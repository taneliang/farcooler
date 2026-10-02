import AgentKit
import SwiftUI

/// The board's header: its title, what's waiting, and the controls.
///
/// A board can be as narrow as `WorkspaceColumns.boardMinimum`, and at 1180 pt
/// it is not much wider, so the header can't assume its controls fit
/// (ov-81 P1: Refresh, the New Task button and the layout toggle ran out of
/// the window's edge). It offers three arrangements, and `ViewThatFits` takes
/// the first that fits whole: every control with its words, then icons only,
/// then the title and one overflow menu. The last always fits, because its
/// title is the one thing allowed to truncate.
struct BoardHeader: View {
    let title: String
    let waiting: Int
    let reading: Bool
    /// What a failed re-read says, when one failed over a board already read.
    let trouble: String?
    let offersWrites: Bool
    @Binding var choice: BoardForm.Choice
    /// The form on screen, for the toggle's marking.
    let drawn: BoardForm?
    @Binding var newTaskOpen: Bool
    let onCreate: (String) async -> Bool
    let onRefresh: () -> Void

    var body: some View {
        ViewThatFits(in: .horizontal) {
            row(.full)
            row(.icons)
            row(.overflow)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(WorkspaceStyle.paneChrome)
        // On the header rather than on the button, so the same form opens
        // from the plus button and from the overflow menu's item.
        .popover(isPresented: $newTaskOpen, arrowEdge: .bottom) {
            NewTaskForm(onCreate: onCreate, onClose: { newTaskOpen = false })
        }
    }

    /// How much of the controls a header can afford.
    enum Level { case full, icons, overflow }

    private func row(_ level: Level) -> some View {
        HStack(spacing: 10) {
            Text(title).font(WorkspaceStyle.sectionTitle).lineLimit(1).truncationMode(.tail)
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
            case .full, .icons:
                if offersWrites { newTaskButton }
                formToggle
                if level == .full {
                    Button("Refresh", action: onRefresh).fixedSize()
                } else {
                    Button(action: onRefresh) { Image(systemName: "arrow.clockwise") }
                        .help("Refresh")
                        .accessibilityLabel("Refresh")
                }
            case .overflow:
                overflowMenu
            }
        }
        .fixedSize(horizontal: level != .overflow, vertical: false)
    }

    private var newTaskButton: some View {
        Button {
            newTaskOpen = true
        } label: {
            Image(systemName: "plus")
        }
        .help("New Task…")
        .accessibilityLabel("New Task…")
        .accessibilityIdentifier("board-new-task")
    }

    /// Everything the header does, in one menu, for a board with no room.
    private var overflowMenu: some View {
        Menu {
            if offersWrites {
                Button("New Task…") { newTaskOpen = true }
            }
            Button("Refresh", action: onRefresh)
            Divider()
            layoutPicker
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Board Actions")
        .accessibilityLabel("Board Actions")
        .accessibilityIdentifier("board-overflow")
    }

    private var layoutPicker: some View {
        Picker("Board Layout", selection: $choice) {
            Text("Automatic").tag(BoardForm.Choice.auto)
            Text("List").tag(BoardForm.Choice.list)
            Text("Kanban").tag(BoardForm.Choice.kanban)
        }
        .pickerStyle(.inline)
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

    /// `≡` List and `▦` Kanban. Clicking one forces it; clicking the one
    /// forced goes back to Automatic, which is also in the control's menu.
    /// The form on screen is always marked, and a forced one more strongly.
    private var formToggle: some View {
        HStack(spacing: 1) {
            formButton(.list, symbol: "list.bullet", name: "List")
            formButton(.kanban, symbol: "rectangle.split.3x1", name: "Kanban")
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.06)))
        .contextMenu { layoutPicker }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board-form-toggle")
    }

    private func formButton(_ form: BoardForm, symbol: String, name: String) -> some View {
        let forced = choice.forced == form
        let shown = drawn == form
        return Button {
            choice = choice.choosing(form)
        } label: {
            Image(systemName: symbol)
                .font(.system(size: WorkspaceStyle.PaneText.secondary, weight: .medium))
                .frame(width: 24, height: 18)
                .foregroundStyle(forced ? Color.white : shown ? Color.primary : Color.secondary)
                .background(
                    RoundedRectangle(cornerRadius: 4)
                        .fill(
                            forced
                                ? Color.accentColor
                                : shown ? Color.primary.opacity(0.1) : Color.clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(
            forced
                ? "Always shown as \(article(form)). Click again to choose by width."
                : "Always show this board as \(article(form)).")
        .accessibilityLabel(name)
        .accessibilityValue(forced ? "Chosen" : shown ? "Shown" : "")
    }

    /// "a list" or "a kanban", for the toggle's tooltips.
    private func article(_ form: BoardForm) -> String {
        form == .list ? "a list" : "a kanban"
    }
}
