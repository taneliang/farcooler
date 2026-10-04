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
        /// The window is going back to where it was, on a runner that
        /// hasn't connected yet (ov-279): the runner, `""` for this Mac.
        case connecting(String)
    }

    /// The phase for the fleet as it is.
    ///
    /// Asked of the local runner, as the sidebar asked it: this Mac is
    /// always present (`FleetStore.hosts` puts it first), and a remote
    /// runner's trouble is the toolbar's runner item's to say, so it never
    /// blanks out a fleet this Mac can show. A fleet with any worktree is
    /// past all of this, whichever runner it came from.
    ///
    /// A window going back to a place on a runner still connecting says so,
    /// whatever else the fleet holds: that place is the one it opens.
    nonisolated static func phase(
        hasWorktrees: Bool, localLoaded: Bool, localError: String?, hasRepositories: Bool,
        restoringOn runner: String? = nil
    ) -> Phase {
        if let runner { return .connecting(runner) }
        if hasWorktrees { return .chooseWorkspace }
        if localLoaded { return hasRepositories ? .noWorktrees : .noRepositories }
        if let localError { return .failed(localError) }
        return .loading
    }

    /// What a workspace is for, by its job: the purpose, then what its
    /// orchestrator does, never where anything sits on screen. Rows, not the
    /// five-line paragraph the owner found "too many words" (ov-205).
    static let workspaceCopy = EmptyStateCopy(
        lede: "A workspace holds one feature or fix.",
        rows: [
            .init(symbol: "bubble.left", text: "Tell the orchestrator what you want"),
            .init(symbol: "checklist", text: "Agents start on the tasks it makes"),
        ])

    /// Under "No Repositories": what to add, then what a worktree does for
    /// you, by its job before its name.
    static let noRepositoriesCopy = EmptyStateCopy(
        lede: "Add the repository you want agents to work in.",
        rows: [
            .init(symbol: "arrow.triangle.branch", text: "Each agent gets its own folder and branch"),
            .init(symbol: "arrow.triangle.merge", text: "Your files don’t change until you merge"),
        ])

    /// Under "No Worktrees".
    static let noWorktreesCopy = EmptyStateCopy(
        lede: "A worktree gives an agent its own folder and branch.",
        rows: [
            .init(symbol: "arrow.triangle.merge", text: "Your files don’t change until you merge"),
        ])

    /// The Main the empty detail's Open Main opens: the one repository's, and
    /// only where a Main exists to open, never a guess between several. A
    /// runner without workspaces has one board per repository, which is its
    /// Main.
    static func mainToOpen(
        in repositories: [(host: String, repository: Repository)], fleet: Fleet
    ) -> (host: String, workspace: WorkspaceSummary)? {
        guard repositories.count == 1, let only = repositories.first else { return nil }
        guard let listed = fleet.runnerWorkspaces[only.host] else {
            return (only.host, .implicit(repository: only.repository.id))
        }
        guard let main = listed.first(where: { $0.isMain && $0.repository == only.repository.id }) else {
            return nil
        }
        return (only.host, main)
    }

    /// The connecting state's title: "Connecting to studio…".
    nonisolated static func connectingTitle(_ runner: String) -> String {
        "Connecting to \(runner.isEmpty ? "this Mac’s runner" : runner)…"
    }

    /// Under it: what happens next, so waiting reads as waiting.
    static let connectingCopy = "This window goes back to where you left it once the runner answers."

    let phase: Phase
    /// Show Needs You, from the connecting state: somewhere to work while
    /// the runner comes. Moving gives up the restore (`WindowRestore`).
    var onShowNeedsYou: (() -> Void)?
    /// Open Main, when the one repository has a Main workspace to open.
    var onOpenMain: (() -> Void)?
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
                EmptyStateTitle("No Repositories", symbol: "rectangle.stack")
            } description: {
                EmptyStateRows(copy: Self.noRepositoriesCopy)
            } actions: {
                Button("Add Repository…", action: onAddRepository)
            }
        case .noWorktrees:
            ContentUnavailableView {
                EmptyStateTitle("No Worktrees", symbol: "rectangle.stack")
            } description: {
                EmptyStateRows(copy: Self.noWorktreesCopy)
            } actions: {
                Button("New Worktree…", action: onNewWorktree)
            }
        case .connecting(let runner):
            ContentUnavailableView {
                Label {
                    Text(Self.connectingTitle(runner))
                } icon: {
                    ProgressView().controlSize(.small)
                }
            } description: {
                Text(Self.connectingCopy)
            } actions: {
                if let onShowNeedsYou {
                    Button("Show Needs You", action: onShowNeedsYou)
                }
            }
        case .chooseWorkspace:
            ContentUnavailableView {
                EmptyStateTitle("No Workspace Selected", symbol: "square.stack.3d.up")
            } description: {
                EmptyStateRows(copy: Self.workspaceCopy)
            } actions: {
                if let onOpenMain {
                    Button("Open Main", action: onOpenMain)
                        .buttonStyle(.borderedProminent)
                }
                if let onNewWorkspace {
                    Button("New Workspace…", action: onNewWorkspace)
                }
            }
        }
    }
}
