import AppKit
import SwiftUI

/// The editors, as menu items.
///
/// Shared by the title bar control and the sidebar row's `…` menu so the two
/// cannot drift into offering different editors, or the same editor under
/// different rules about which ones are greyed out.
struct EditorMenuItems: View {
    let worktree: Worktree
    let onError: (String) -> Void
    /// The title bar button opens the last-used editor on a click, so its menu
    /// does not repeat that as a heading. The sidebar has no primary action and
    /// needs one.
    var showsSettingsItem = true

    @ObservedObject private var editors = Editors.shared
    @Environment(\.openSettings) private var openSettings

    /// The runner this worktree is on, as its ssh target. Empty means this Mac.
    ///
    /// The `host:` argument labels below keep their spelling: what they take is
    /// the value substituted into an editor's `{host}` template, which is a
    /// token people have already saved in custom editor arguments.
    private var runner: String { worktree.host ?? "" }
    private var usable: [Editor] { editors.available.filter { $0.unavailability(host: runner) == nil } }

    /// Kept in the menu rather than dropped from it. An editor you have
    /// installed, absent from a list of editors, reads as a bug in Far Cooler;
    /// the same editor greyed out under a heading that says why reads as the
    /// truth about the editor.
    private var unusable: [Editor] {
        editors.available.filter { $0.unavailability(host: runner) != nil }
    }

    var body: some View {
        ForEach(usable) { editor in
            Button("Open in \(editor.name)") { open(editor) }
        }

        if !unusable.isEmpty {
            Section("Cannot open a worktree on \(runner)") {
                ForEach(unusable) { editor in
                    Button(editor.name) {}.disabled(true)
                }
            }
        }

        if showsSettingsItem {
            Divider()
            Button(editors.available.isEmpty ? "Add an editor…" : "Editors…") {
                EditorSettingsLink.open(openSettings)
            }
        }
    }

    private func open(_ editor: Editor) {
        editors.remember(editor)
        Task {
            if let problem = await editors.open(worktree, with: editor) { onError(problem) }
        }
    }
}

/// Sending someone to the Editors tab.
///
/// Settings is a scene, not a sheet, so nothing that opens it can hand it a
/// parameter — the tab is set first and read by `SettingsView`. Same shape as
/// "Add a runner…" in `ContentView`.
///
/// Takes the caller's own `openSettings` rather than reaching for
/// `NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)`:
/// that nil-target send searches the responder chain, and nothing in it
/// answers to that selector outside of the real "Settings…" menu item — which
/// AppKit invokes by calling its own bound target directly, never through that
/// search. The call looked plausible and silently did nothing.
@MainActor
enum EditorSettingsLink {
    static func open(_ openSettings: OpenSettingsAction) {
        Preferences.shared.settingsTab = "editors"
        openSettings()
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
    let worktree: Worktree
    /// Where a launch failure goes. `ContentView` puts it in the banner.
    let onError: (String) -> Void

    @ObservedObject private var editors = Editors.shared
    @Environment(\.openSettings) private var openSettings

    /// The runner this worktree is on, as its ssh target. Empty means this Mac.
    private var runner: String { worktree.host ?? "" }

    var body: some View {
        Menu {
            EditorMenuItems(worktree: worktree, onError: onError)
        } label: {
            Image(systemName: "chevron.left.forwardslash.chevron.right")
        } primaryAction: {
            guard let editor = editors.preferred(host: runner) else {
                EditorSettingsLink.open(openSettings)
                return
            }
            // Deliberately not remembered. A click uses what is already
            // remembered, and on a remote worktree that may be a stand-in for
            // it — see `Editors.preferred`. Only an explicit pick from the menu
            // changes the preference.
            Task {
                if let problem = await editors.open(worktree, with: editor) {
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
        guard let editor = editors.preferred(host: runner) else {
            return editors.available.isEmpty
                ? "No editors found — add one in Settings"
                : "No editor here can open a worktree on \(runner)"
        }
        return "Open in \(editor.name)"
    }
}
