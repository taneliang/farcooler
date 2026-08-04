import SwiftUI

/// Which editors this Mac has, which one opens a worktree, and any you add.
///
/// Its own tab rather than a row under Behavior: a list of applications with a
/// form for adding to it is not a checkbox, and the reason someone comes here is
/// usually the one question the rest of the app cannot answer for them — "why is
/// my editor not in the menu?" That question is answered by showing the list of
/// what was found, so the list is the tab.
struct EditorsSettings: View {
    @ObservedObject private var editors = Editors.shared
    @ObservedObject private var preferences = Preferences.shared

    @State private var name = ""
    @State private var local = ""
    @State private var remote = ""

    var body: some View {
        Form {
            Section {
                if editors.available.isEmpty {
                    Text("No editors found. Add one below.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                ForEach(editors.available) { editor in
                    row(editor)
                }
            } header: {
                Text("Editors")
            } footer: {
                // The remote column needs explaining once, here, rather than in
                // every disabled menu item.
                Text(
                    "Found by looking for the applications themselves, so an editor "
                        + "installed anywhere is found. Only some can open a worktree "
                        + "on another machine — the others are offered for local "
                        + "worktrees only."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }

            Section {
                TextField("Name", text: $name)
                TextField("Command", text: $local)
                    .autocorrectionDisabled()
                TextField("Command for another machine", text: $remote)
                    .autocorrectionDisabled()
                HStack {
                    Spacer()
                    Button("Add", action: add)
                        .disabled(
                            name.trimmingCharacters(in: .whitespaces).isEmpty
                                || local.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            } header: {
                Text("Add an editor")
            } footer: {
                Text(
                    "{path} becomes the worktree's path, {host} the machine it is on. "
                        + "Leave the second field empty if the editor cannot open a "
                        + "worktree on another machine."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        // Someone who came here because their editor was missing has very often
        // just installed it.
        .onAppear { editors.refresh() }
    }

    private func row(_ editor: Editor) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(editor.name).font(.body)
                Text(editor.opensRemote ? "This Mac and other machines" : "This Mac only")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            if isDefault(editor) {
                Label("Default", systemImage: "checkmark.circle.fill")
                    .labelStyle(.iconOnly)
                    .foregroundStyle(.green)
                    .help("Opens when you click the button")
            } else {
                Button("Make default") { editors.remember(editor) }
            }

            if editor.id.hasPrefix("custom:") {
                Button {
                    editors.removeCustom(editor)
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(.borderless)
                .help("Remove \(editor.name)")
            }
        }
        .padding(.vertical, 2)
    }

    /// Marks the editor a click would actually use for a local worktree.
    ///
    /// Local, specifically. `Editors.preferred` substitutes another editor for a
    /// worktree on a machine this one cannot reach, and showing that substitute
    /// here as "Default" would claim the preference changed when it did not.
    private func isDefault(_ editor: Editor) -> Bool {
        editors.preferred(host: "")?.id == editor.id
    }

    private func add() {
        editors.addCustom(name: name, local: local, remote: remote)
        name = ""
        local = ""
        remote = ""
    }
}
