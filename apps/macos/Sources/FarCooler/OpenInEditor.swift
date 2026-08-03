import AppKit
import SwiftUI

/// The editors, as menu items.
///
/// Shared by the title bar control and the sidebar row's `…` menu so the two
/// cannot drift into offering different editors, or the same editor under
/// different rules about which ones are greyed out.
struct EditorMenuItems: View {
    let workspace: Workspace
    let onError: (String) -> Void
    /// The title bar button opens the last-used editor on a click, so its menu
    /// does not repeat that as a heading. The sidebar has no primary action and
    /// needs one.
    var showsSettingsItem = true

    @ObservedObject private var editors = Editors.shared

    private var host: String { workspace.host ?? "" }
    private var usable: [Editor] { editors.available.filter { $0.unavailability(host: host) == nil } }

    /// Kept in the menu rather than dropped from it. An editor you have
    /// installed, absent from a list of editors, reads as a bug in Far Cooler;
    /// the same editor greyed out under a heading that says why reads as the
    /// truth about the editor.
    private var unusable: [Editor] {
        editors.available.filter { $0.unavailability(host: host) != nil }
    }

    var body: some View {
        ForEach(usable) { editor in
            Button("Open in \(editor.name)") { open(editor) }
        }

        if !unusable.isEmpty {
            Section("Cannot open a worktree on \(host)") {
                ForEach(unusable) { editor in
                    Button(editor.name) {}.disabled(true)
                }
            }
        }

        if showsSettingsItem {
            Divider()
            Button(editors.available.isEmpty ? "Add an editor…" : "Editors…") {
                EditorSettingsLink.open()
            }
        }
    }

    private func open(_ editor: Editor) {
        editors.remember(editor)
        Task {
            if let problem = await editors.open(workspace, with: editor) { onError(problem) }
        }
    }
}

/// Sending someone to the Editors tab.
///
/// Settings is a scene, not a sheet, so nothing that opens it can hand it a
/// parameter — the tab is set first and read by `SettingsView`. Same shape as
/// "Add a machine…" in `ContentView`.
@MainActor
enum EditorSettingsLink {
    static func open() {
        Preferences.shared.settingsTab = "editors"
        NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
    }
}

/// The control that hands a worktree to an editor.
///
/// A `Menu` with a primary action: clicking opens the last-used editor, and the
/// chevron picks a different one. The common case — you have one editor and you
/// always use it — is one click, and the uncommon one is still one menu away.
///
/// The label is a symbol with a tooltip rather than the editor's name. A title
/// bar control whose width changes when you switch from Zed to Android Studio
/// moves everything beside it, and the name is in the menu anyway.
struct OpenInEditorButton: View {
    let workspace: Workspace
    /// Where a launch failure goes. `ContentView` puts it in the banner.
    let onError: (String) -> Void

    @ObservedObject private var editors = Editors.shared

    private var host: String { workspace.host ?? "" }

    var body: some View {
        Menu {
            EditorMenuItems(workspace: workspace, onError: onError)
        } label: {
            Image(systemName: "chevron.left.forwardslash.chevron.right")
        } primaryAction: {
            guard let editor = editors.preferred(host: host) else {
                EditorSettingsLink.open()
                return
            }
            // Deliberately not remembered. A click uses what is already
            // remembered, and on a remote worktree that may be a stand-in for
            // it — see `Editors.preferred`. Only an explicit pick from the menu
            // changes the preference.
            Task {
                if let problem = await editors.open(workspace, with: editor) {
                    onError(problem)
                }
            }
        }
        // Re-probing here is what makes an editor installed while the app was
        // open show up without a relaunch.
        .onAppear { editors.refresh() }
        .help(tooltip)
    }

    private var tooltip: String {
        guard let editor = editors.preferred(host: host) else {
            return editors.available.isEmpty
                ? "No editors found — add one in Settings"
                : "No editor here can open a worktree on \(host)"
        }
        return "Open in \(editor.name)"
    }
}

/// What the editor said when it would not start.
///
/// Its own surface rather than `client.lastError`. That property is rendered in
/// exactly one place — the placeholder shown while no fleet has loaded
/// (`ContentView.fleetPlaceholder`) — so once you have a worktree on screen,
/// which is the only time this control exists, writing to it displays nothing.
/// Three existing messages already go there and are already invisible; adding a
/// fourth would be writing a failure into a channel with no reader.
///
/// A banner rather than an alert, matching quick-create: this is information,
/// not a decision, and a modal sheet for "code: command not found" stops the
/// window to say something you can act on at your leisure.
struct EditorErrorBanner: View {
    let message: String
    let onClose: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)

            // The launcher's own words, not a paraphrase. It is the thing that
            // knows what went wrong — "Remote-SSH is not installed" is a
            // sentence the user can act on, and "could not open" is not.
            Text(message)
                .font(.callout)
                .textSelection(.enabled)
                .lineLimit(4)
                .fixedSize(horizontal: false, vertical: true)

            Button(action: onClose) {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .semibold))
            }
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .frame(maxWidth: 460, alignment: .leading)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(Color.primary.opacity(0.08)))
        .shadow(radius: 12, y: 4)
    }
}

extension View {
    /// Put the editor control in the window's title bar.
    ///
    /// Applied once, where `ContentView` places the detail column, rather than
    /// in each of the four views that set the window's title. Those views would
    /// each need the failure channel threaded down to them, and a control that
    /// belongs to "whatever worktree you are looking at" should be attached
    /// where that is decided, not in three places that each know one case of it.
    ///
    /// The right side of the title bar is otherwise empty, and the title already
    /// names the worktree — so the action about that worktree belongs beside it.
    func openInEditorToolbar(
        workspace: Workspace?, onError: @escaping (String) -> Void
    ) -> some View {
        toolbar {
            if let workspace {
                ToolbarItem(placement: .primaryAction) {
                    OpenInEditorButton(workspace: workspace, onError: onError)
                }
            }
        }
    }
}
