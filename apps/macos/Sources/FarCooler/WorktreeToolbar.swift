import SwiftUI

/// The toolbar's actions on the worktree on screen (ov-214): Open in Editor,
/// Show Changes and Show Files (ov-189), in one group, so the toolbar draws
/// them in one capsule rather than several. They act on the same thing, and
/// the HIG asks for at most three groups ("Minimize the number of groups").
///
/// Each is still drawn only where it can act: the editor with a worktree on
/// screen, Changes where `WorkspaceScreen.changesTarget` names one and its
/// runner can read changes, Files there too where its runner can read files.
struct WorktreeToolbar: ToolbarContent {
    /// Show Changes, where offered: the worktree it splits, and whether its
    /// Changes pane is open.
    struct Changes {
        var worktree: Worktree
        var open: Bool
    }

    /// Show Files (ov-189), where offered: the worktree whose files the
    /// inspector shows, and whether it's showing them.
    struct Files {
        var worktree: Worktree
        var open: Bool
    }

    let editor: Worktree?
    let onEditorError: (String) -> Void
    let changes: Changes?
    let onChanges: (Worktree) -> Void
    var files: Files? = nil
    var onFiles: (Worktree) -> Void = { _ in }

    var body: some ToolbarContent {
        // Automatic, not primary: with the title gone from the toolbar, only
        // the flexible space `LeadingToolbar` leaves pushes items to the
        // trailing end, and it moves automatic ones only (ov-105).
        ToolbarItemGroup(placement: .automatic) {
            if let editor {
                OpenInEditorButton(worktree: editor, onError: onEditorError)
            }
            if let changes {
                Button {
                    onChanges(changes.worktree)
                } label: {
                    Label("Changes", systemImage: "plusminus")
                }
                // Lit while one is open, the way a toggle in a toolbar says
                // which state you are in. A `Button` rather than a `Toggle`
                // because the two directions aren't symmetrical: opening
                // splits a pane, closing kills one.
                .symbolVariant(changes.open ? .fill : .none)
                .help(changes.open ? "Close the changes pane" : "Show what this worktree changed, in a pane")
            }
            // Its files, read-only, in the inspector (ov-189).
            if let files {
                Button {
                    onFiles(files.worktree)
                } label: {
                    Label("Files", systemImage: "doc.text")
                }
                .symbolVariant(files.open ? .fill : .none)
                .help(files.open ? "Hide this worktree’s files" : "Show this worktree’s files")
            }
        }
    }
}
