import AgentKit
import Foundation

/// What a worktree changed.
///
/// Decoded from `farcooler changes … --json`, which is the same path every other
/// read in this app takes. The daemon owns all of it; nothing here computes
/// state, it draws what the machine derived — the same contract the terminal
/// rows keep.

struct ChangeSet: Decodable, Equatable {
    var branch: String
    var baseRef: String
    var baseCommit: String
    var headCommit: String
    var insertions: Int
    var deletions: Int
    var commits: [ChangeCommit]
    var files: [ChangedFile]
    var workingTree: WorkingTree?

    enum CodingKeys: String, CodingKey {
        case branch
        case baseRef = "base_ref"
        case baseCommit = "base_commit"
        case headCommit = "head_commit"
        case insertions, deletions, commits, files
        case workingTree = "working_tree"
    }

    static let empty = ChangeSet(
        branch: "", baseRef: "", baseCommit: "", headCommit: "",
        insertions: 0, deletions: 0, commits: [], files: [], workingTree: nil)

    var isDirty: Bool {
        guard let w = workingTree else { return false }
        return !w.staged.isEmpty || !w.unstaged.isEmpty || !w.untracked.isEmpty
            || !w.conflicted.isEmpty
    }
}

struct ChangeCommit: Decodable, Equatable, Identifiable {
    var sha: String
    var subject: String
    var author: String
    var timestamp: Int

    var id: String { sha }
    var short: String { String(sha.prefix(8)) }
}

struct ChangedFile: Decodable, Equatable, Identifiable {
    var path: String
    var insertions: Int
    var deletions: Int
    var binary: Bool

    var id: String { path }
    var name: String { (path as NSString).lastPathComponent }
}

struct WorkingTree: Decodable, Equatable {
    var staged: [String]
    var unstaged: [String]
    var untracked: [String]
    var conflicted: [String]
}

/// One worktree's line in the fleet's changed-since-you-looked list.
///
/// This used to carry counts of open and answered comments. Those came from the
/// review buffer, which is gone; what is left is the question the sidebar
/// actually asks.
struct InboxRow: Decodable, Equatable, Identifiable {
    var workspaceId: String
    var changedSinceReviewed: Bool
    var insertions: Int
    var deletions: Int

    var id: String { workspaceId }
    var hasDiff: Bool { insertions > 0 || deletions > 0 }

    enum CodingKeys: String, CodingKey {
        case workspaceId = "workspace_id"
        case insertions, deletions
        case changedSinceReviewed = "changed_since_reviewed"
    }
}

/// Which comparison the diff tile is showing.
///
/// `Commit` is deliberately absent: it needs a commit picker, and the PR tile
/// already has to draw a commit list. Adding a second way to reach the same view
/// before that tile exists would mean guessing at the picker twice.
enum DiffScope: String, CaseIterable, Identifiable, Codable {
    case branch
    case local

    var id: String { rawValue }

    var label: String {
        switch self {
        case .branch: return "Branch"
        case .local: return "Local"
        }
    }
}

/// What one worktree changed, and the file currently open.
@MainActor
final class ChangesStore: ObservableObject {
    @Published var changeSet: ChangeSet = .empty
    @Published var diff: [DiffComputation.Line] = []
    @Published var selectedFile: String?
    @Published var scope: DiffScope = .branch
    @Published var loading = false
    @Published var error: String?

    let client: DaemonClient
    let workspace: Workspace

    init(client: DaemonClient, workspace: Workspace) {
        self.client = client
        self.workspace = workspace
    }

    func load(fresh: Bool = false) async {
        loading = true
        defer { loading = false }

        var args = ["changes", "status", workspace.short, "--json"]
        if fresh { args.append("--fresh") }
        if let data = await client.changesJSON(args) {
            changeSet = (try? JSONDecoder().decode(ChangeSet.self, from: data)) ?? .empty
            error = nil
        } else {
            // A failure is NOT an empty diff. Saying so was a real bug once: a
            // machine whose daemon predates this answered NOT_FOUND to every
            // call, and the pane drew a worktree with no changes.
            changeSet = .empty
            error = client.changesError
        }

        // A file that is no longer in the change set cannot be shown.
        if let open = selectedFile, !changeSet.files.contains(where: { $0.path == open }) {
            closeFile()
        } else if let open = selectedFile {
            await openFile(open)
        }
    }

    func openFile(_ path: String) async {
        selectedFile = path
        diff = await client.changesDiff(workspace: workspace.short, path: path, scope: scope)
    }

    func closeFile() {
        selectedFile = nil
        diff = []
    }

    func markRead() async {
        await client.changesMarkRead(workspace: workspace.short)
        await load()
    }
}
