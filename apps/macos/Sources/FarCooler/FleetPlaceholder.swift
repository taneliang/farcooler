import AgentKit
import SwiftUI

/// What the window's detail says with no workspace to show (ov-178): the
/// fleet's own state before it has anything in it, then "choose one".
///
/// This was the old sidebar's: its loading spinner, "Couldn’t Read the
/// Fleet" with the command's words and Try Again, and "No worktrees" with
/// Add Repository… or New Worktree…. With the sidebar gone, and hidden in
/// every new window before that, the detail is the one place left to say
/// it. Without it, a first launch whose daemon wouldn't answer read "No
/// Workspace Selected", and sent someone to a title-bar menu with nothing
/// in it.
struct FleetPlaceholder: View {
    /// Which of the placeholder's states to draw.
    enum Phase: Equatable {
        /// This Mac's runner hasn't been read yet.
        case loading
        /// It couldn't be read: the command's own words.
        case failed(String)
        /// Read, with no repository registered.
        case noRepositories
        /// Read, with repositories and still no worktree listed.
        case noWorktrees
        /// The fleet has worktrees: a workspace to choose.
        case chooseWorkspace
    }

    /// The phase for the fleet as it is.
    ///
    /// Asked of the local runner, as the sidebar asked it: this Mac is
    /// always present (`FleetStore.hosts` puts it first), and a remote
    /// runner's trouble is the toolbar's runner item's to say, so it never
    /// blanks out a fleet this Mac can show. A fleet with any worktree is
    /// past all of this, whichever runner it came from.
    nonisolated static func phase(
        hasWorktrees: Bool, localLoaded: Bool, localError: String?, hasRepositories: Bool
    ) -> Phase {
        if hasWorktrees { return .chooseWorkspace }
        if localLoaded { return hasRepositories ? .noWorktrees : .noRepositories }
        if let localError { return .failed(localError) }
        return .loading
    }

    let phase: Phase
    /// New Workspace…, where a runner has workspaces to make one in.
    var onNewWorkspace: (() -> Void)?
    let onAddRepository: () -> Void
    let onNewWorktree: () -> Void
    let onTryAgain: () -> Void

    var body: some View {
        switch phase {
        case .loading:
            ProgressView()
                .controlSize(.small)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .failed(let error):
            ContentUnavailableView {
                Label("Couldn’t Read the Fleet", systemImage: "exclamationmark.triangle")
            } description: {
                // The heading, then a sentence, then the box: the command's
                // stderr in a box of its own, never in the app's voice. When
                // this Mac's own daemon won't answer, those words are the
                // only diagnosis anyone has.
                Text("The command that reads it didn’t finish.")
            } actions: {
                DetailBox(text: error)
                    .frame(maxWidth: 420)
                Button("Try Again", action: onTryAgain)
            }
        case .noRepositories:
            ContentUnavailableView {
                Label("No Repositories", systemImage: "rectangle.stack")
            } description: {
                Text("Add a repository to get started. Each worktree is a directory and branch of its own.")
            } actions: {
                Button("Add Repository…", action: onAddRepository)
            }
        case .noWorktrees:
            ContentUnavailableView {
                Label("No Worktrees", systemImage: "rectangle.stack")
            } description: {
                Text("A worktree is a directory and branch of its own.")
            } actions: {
                Button("New Worktree…", action: onNewWorktree)
            }
        case .chooseWorkspace:
            ContentUnavailableView {
                Label("No Workspace Selected", systemImage: "square.stack.3d.up")
            } description: {
                Text(
                    "Choose one from the title bar. A workspace lists its orchestrator, tasks, and worktrees on the left, and shows the one you pick."
                )
            } actions: {
                if let onNewWorkspace {
                    Button("New Workspace…", action: onNewWorkspace)
                }
            }
        }
    }
}
