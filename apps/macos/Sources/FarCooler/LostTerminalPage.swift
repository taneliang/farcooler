import AgentKit
import SwiftUI

/// The page a terminal with no running pane opens to: why, its last screen
/// if this Mac kept one, and Restart and Dismiss (ov-191).
///
/// It replaces a sentence that said "The session ended or the runner can’t be
/// reached." for every such state, and a Dismiss button that was offered for
/// an exited terminal too, which the runner refuses. The words and the offer
/// are `LostPane`'s, which the phone reads too.
///
/// Keyboard: Return restarts and Delete dismisses, while the page has the
/// keyboard. Heard on the page rather than as menu shortcuts, so they never
/// reach past it: ⌘⌫ is a permission card's Deny in a chat beside it, and
/// ⌘R is Reload Fleet.
struct LostTerminalPage: View {
    let terminal: Terminal
    /// The worktree, to name the window, when the page has it to itself.
    /// Nil in a tile, which leaves the window's title to the layout.
    var worktree: Worktree? = nil
    var hasKeyboard = true
    let onAction: (TerminalAction) -> Void

    @FocusState private var focused: Bool
    @ObservedObject private var themes = Themes.shared

    private var kind: LostPane.Kind { LostPane.Kind(state: terminal.state) ?? .lost }
    private var actions: [LostPane.Action] { LostPane.actions(for: kind) }

    /// What this Mac last drew of it, when it drew it this session. The
    /// runner keeps no copy: a lost pane's screen went with the pane.
    private var lastScreen: [String]? { TerminalScreens.shared.lastLines(terminal.short) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack(spacing: 10) {
                    StatusGlyph(status: terminal.status)
                    Text(LostPane.title(for: kind)).font(.title3.weight(.semibold))
                }
                Text(terminal.label)
                    .font(.system(size: 13, design: .monospaced))
                    .foregroundStyle(.secondary)

                Text(LostPane.explanation(for: kind))
                    .fixedSize(horizontal: false, vertical: true)

                if let lastScreen, !lastScreen.isEmpty {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("Its last screen, as this Mac last saw it")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        ScreenPreviewText(lines: lastScreen, width: 640, size: 11)
                            .padding(10)
                            .background(
                                RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: Palette.background)))
                            .textSelection(.enabled)
                    }
                }

                VStack(alignment: .leading, spacing: 4) {
                    Text(LostPane.restartNote(preset: terminal.preset))
                    if actions.contains(.dismiss) { Text(LostPane.dismissNote) }
                }
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

                HStack(spacing: 10) {
                    Button("Restart") { onAction(.restart) }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("lost-restart")
                    if actions.contains(.dismiss) {
                        Button("Dismiss") { onAction(.dismissLost) }
                            .accessibilityIdentifier("lost-dismiss")
                    }
                }
            }
            .padding(32)  // grid-exempt: a detail page's margin, as WorktreeDetail's
            .frame(maxWidth: 720, alignment: .leading)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(nsColor: Palette.background))
        // On the terminal's ground, so in the terminal theme's polarity, not
        // the app's: under a light app and a dark theme, system text and
        // buttons came out dark on dark (seen in the after-shot).
        .environment(\.colorScheme, themes.current.dark ? .dark : .light)
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onKeyPress(.return) {
            onAction(.restart)
            return .handled
        }
        .onKeyPress(.delete) {
            guard actions.contains(.dismiss) else { return .ignored }
            onAction(.dismissLost)
            return .handled
        }
        .onChange(of: hasKeyboard, initial: true) { _, keyed in if keyed { focused = true } }
        .modifier(WindowTitled(worktree: worktree))
    }
}

/// The worktree's title on the window, or nothing when there's no worktree.
private struct WindowTitled: ViewModifier {
    let worktree: Worktree?

    func body(content: Content) -> some View {
        if let worktree {
            content
                .navigationTitle(worktree.windowTitle)
                .navigationSubtitle(worktree.windowSubtitle)
        } else {
            content
        }
    }
}
