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
///   ⌘R        reload, which for a terminal means restart
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
                Item(keys: "⌘R", action: "Restart terminal"),
                Item(keys: "⌘1 … ⌘9", action: "Jump to terminal"),
                Item(keys: "⌘]", action: "Next terminal"),
                Item(keys: "⌘[", action: "Previous terminal"),
                Item(keys: "⌃⌘N", action: "Jump to the next agent that needs you"),
            ]
        ),
        (
            "Workspaces",
            [
                Item(keys: "⇧⌘N", action: "New workspace"),
                Item(keys: "⇧⌘R", action: "Add repository"),
            ]
        ),
        (
            "App",
            [
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
