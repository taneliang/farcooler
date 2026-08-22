import SwiftUI

/// The keyboard shortcuts, in one place.
///
/// Discoverability on macOS is not a cheat sheet — it is the menu bar. Every
/// shortcut here is also a real menu item, so it appears where people already
/// look, is searchable through Help, and shows its key equivalent without
/// anyone having to be told. The sheet on ⌘/ is for the moment you want them
/// all at once, not the primary way to find them.
///
/// The bindings follow what the rest of macOS already means, because a shortcut
/// that has to be learned is one that will not be:
///
///   ⌘T / ⌘W   new and close, as in every tabbed app
///   ⌘1…⌘9     jump to the nth thing, as in every browser
///   ⌘[ / ⌘]   back and forward through them
///   ⌃⇥ / ⇧⌘]  next tab, on both spellings the platform uses — and a layout
///             IS the tab here, which is why these walk the pill bar
///   ⌘0        reload the fleet
///
/// Tiling is the exception, and deliberately so. It uses a tmux PREFIX — ⌃B, then
/// a key — for two reasons. The ⌘ chords are gone: a terminal wants nearly all of
/// them, and what is left is what nobody can remember. And a very large share of
/// the people this is for already have ⌃B z in their fingers, so borrowing tmux's
/// bindings means the tiling has no learning curve for them at all.
///
/// ⌃B ⌃B sends a literal ⌃B through, exactly as tmux does, so running tmux inside
/// a Far Cooler pane still works.
///
/// Moving between panes is the one thing bound WITHOUT the prefix, because it is
/// the one thing done constantly: ⌃hjkl, as vim-tmux-navigator taught everyone.
/// Those four are also backspace, newline, kill-line and clear-screen, so they
/// are only taken while more than one pane is on screen — with a single terminal
/// ⌃L still clears it.
enum Shortcut {
    struct Item: Identifiable {
        let id = UUID()
        let keys: String
        let action: String
    }

    static let groups: [(String, [Item])] = [
        (
            "Terminals",
            [
                Item(keys: "⌘T", action: "New terminal in this workspace"),
                Item(keys: "⌘W", action: "Close terminal"),
                Item(keys: "⌘1 … ⌘9", action: "Jump to terminal"),
                Item(keys: "⌘]", action: "Next terminal"),
                Item(keys: "⌘[", action: "Previous terminal"),
                Item(keys: "⌃⌘N", action: "Jump to the next agent that needs you"),
            ]
        ),
        (
            "Panes",
            [
                Item(keys: "⌃H ⌃L", action: "Move to the pane left / right"),
                Item(keys: "⌃K ⌃J", action: "Move to the pane above / below"),
            ]
        ),
        (
            "Layouts — the tabs across the top",
            [
                Item(keys: "⌃⇥", action: "Next layout"),
                Item(keys: "⌃⇧⇥", action: "Previous layout"),
                Item(keys: "⇧⌘]", action: "The same, on the other convention"),
                Item(keys: "⇧⌘[", action: "Previous layout"),
            ]
        ),
        (
            "Tiling — press ⌃B, then",
            [
                Item(keys: "z", action: "Zoom the focused pane; again to come back"),
                Item(keys: "space", action: "Next arrangement"),
                Item(keys: "o  /  ;", action: "Next / previous pane"),
                Item(keys: "← → ↑ ↓", action: "Move to the pane in that direction"),
                Item(keys: "h j k l", action: "The same, without leaving home row"),
                Item(keys: "1 … 9", action: "Focus pane by number"),
                Item(keys: "%  /  \"", action: "Split right / down — a new pane here"),
                Item(keys: "!", action: "Move the focused pane out of the layout"),
                Item(keys: "x", action: "Close the focused pane’s terminal"),
                Item(keys: "c", action: "New group — several layouts per workspace"),
                Item(keys: "n  /  p", action: "Next / previous group"),
                Item(keys: "&", action: "Close this group"),
                Item(keys: "a", action: "Toggle the focused pane between terminal and chat"),
                Item(keys: "⌃B", action: "Send a literal ⌃B to the program"),
            ]
        ),
        (
            "Tasks",
            [
                Item(keys: "⌘N", action: "New task — describe it and go"),
                Item(keys: "⇧⌘R", action: "Add Repository"),
                Item(keys: "⇧⌘E", action: "Open this worktree in your editor"),
            ]
        ),
        (
            "App",
            [
                Item(keys: "⌘P", action: "Go to anything — a terminal, a workspace, a new task"),
                Item(keys: "⌘B", action: "Show or hide the sidebar"),
                Item(keys: "⌘F", action: "Find a workspace or agent"),
                Item(keys: "⌘,", action: "Settings"),
                Item(keys: "⌘/", action: "Keyboard shortcuts"),
                Item(keys: "⌘0", action: "Reload the fleet"),
            ]
        ),
    ]
}

struct ShortcutsSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Keyboard shortcuts").font(.title2.weight(.semibold))
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }
            .padding(.horizontal, 22)
            .padding(.top, 20)
            .padding(.bottom, 14)

            Divider()

            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    ForEach(Shortcut.groups, id: \.0) { group in
                        VStack(alignment: .leading, spacing: 7) {
                            Text(group.0)
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .textCase(.uppercase)
                            ForEach(group.1) { item in
                                HStack(spacing: 12) {
                                    Text(item.keys)
                                        .font(.system(.callout, design: .monospaced))
                                        .frame(width: 78, alignment: .leading)
                                        .foregroundStyle(.primary)
                                    Text(item.action)
                                        .font(.callout)
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                }
                            }
                        }
                    }
                }
                .padding(22)
            }

            Divider()
            Text("Every shortcut here is also in the menu bar.")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .padding(.horizontal, 22)
                .padding(.vertical, 12)
        }
        .frame(width: 420, height: 460)
    }
}
