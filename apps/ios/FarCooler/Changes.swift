import Foundation
import SwiftUI

// What a worktree changed, on a phone.
//
// The shapes below are the Mac's `ChangesModel.swift` decodables, unchanged:
// both decode what one Rust function emits (`change_set_json` in
// crates/client/src/session.rs), so there is one definition of what a change
// set looks like on the wire rather than one per platform. What differs is how
// they are FETCHED — the Mac shells out to `farcooler changes … --json`, and a
// phone has no CLI to shell out to, so these go through the same FFI every
// other read on this client takes.
//
// Nothing here computes state. The daemon derives the diff, the counts, and
// which files are dirty; this draws what it derived, the same contract the
// terminal rows keep.

struct ChangeSet: Decodable, Equatable {
    var branch: String
    var baseRef: String
    var baseSource: String?
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
        case baseSource = "base_source"
        case baseCommit = "base_commit"
        case headCommit = "head_commit"
        case insertions, deletions, commits, files
        case workingTree = "working_tree"
    }

    static let empty = ChangeSet(
        branch: "", baseRef: "", baseSource: nil, baseCommit: "", headCommit: "",
        insertions: 0, deletions: 0, commits: [], files: [], workingTree: nil)

    var isDirty: Bool {
        guard let w = workingTree else { return false }
        return !w.staged.isEmpty || !w.unstaged.isEmpty || !w.untracked.isEmpty
            || !w.conflicted.isEmpty
    }

    /// Whether nothing knew what this branch is based on, so a local `main` was
    /// assumed.
    ///
    /// Worth surfacing because it is the only base that can be silently wrong:
    /// a guessed base produces a diff that looks exactly like a right one. See
    /// `BaseSource` in the protocol.
    var baseIsGuessed: Bool { baseSource == "guessed" }
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
    var status: ChangedFileStatus?
    var oldPath: String?
    var insertions: Int
    var deletions: Int
    var binary: Bool

    var id: String { path }
    var name: String { (path as NSString).lastPathComponent }
    /// The directory part, shown under the name. A phone cannot fit
    /// `crates/daemon/src/review_ops.rs` on one line at a readable size, and
    /// the leaf is the part that identifies the file.
    var directory: String {
        let parent = (path as NSString).deletingLastPathComponent
        return parent.isEmpty ? "" : parent
    }

    enum CodingKeys: String, CodingKey {
        case path, status, insertions, deletions, binary
        case oldPath = "old_path"
    }
}

enum ChangedFileStatus: String, Decodable {
    case added, modified, deleted, renamed, copied
    case typeChanged = "type_changed"
    case untracked, conflicted

    var mark: String {
        switch self {
        case .added, .untracked: return "A"
        case .modified: return "M"
        case .deleted: return "D"
        case .renamed: return "R"
        case .copied: return "C"
        case .typeChanged: return "T"
        case .conflicted: return "!"
        }
    }

    var label: String {
        switch self {
        case .added: return "Added"
        case .modified: return "Modified"
        case .deleted: return "Deleted"
        case .renamed: return "Renamed"
        case .copied: return "Copied"
        case .typeChanged: return "Type Changed"
        case .untracked: return "Untracked"
        case .conflicted: return "Conflicted"
        }
    }

    var tint: Color {
        switch self {
        case .added, .untracked: return .green
        case .deleted: return .red
        case .conflicted: return .orange
        default: return .secondary
        }
    }
}

struct WorkingTree: Decodable, Equatable {
    var staged: [String]
    var unstaged: [String]
    var untracked: [String]
    var conflicted: [String]
    var changes: [WorkingTreeFile]?
}

struct WorkingTreeFile: Decodable, Equatable {
    var path: String
    var status: ChangedFileStatus
    var oldPath: String?

    enum CodingKeys: String, CodingKey {
        case path, status
        case oldPath = "old_path"
    }
}

/// One worktree's line in the fleet's changed-since-you-looked list.
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

struct InboxResponse: Decodable {
    var items: [InboxRow]
}

/// Which comparison the diff is showing.
///
/// The Mac's two, and for its reason: `Commit` is deliberately absent because
/// it needs a commit picker, and adding a second way to reach the same view
/// before that exists would mean guessing at the picker twice.
enum DiffScope: String, CaseIterable, Identifiable {
    case branch
    case local

    var id: String { rawValue }

    var label: String {
        switch self {
        case .branch: return "Branch"
        case .local: return "Uncommitted"
        }
    }
}

// MARK: - The store

/// What one worktree changed, and the diffs read so far.
///
/// Deliberately a near-port of the Mac's `ChangesStore` rather than a fresh
/// design: the two apps must agree about what "this worktree changed" means,
/// and the interesting decisions there — read a file at a time as it scrolls
/// into view, re-read only what actually moved on a poll, never mistake a
/// failure for an empty diff — are the same decisions on a phone.
@MainActor
final class ChangesStore: ObservableObject {
    @Published var changeSet: ChangeSet = .empty

    /// Every file's diff, by path, as it is read.
    ///
    /// Filled a file at a time as each scrolls into view rather than up front:
    /// a branch that touched forty files would otherwise pay forty round trips
    /// before drawing a screenful — and on a phone those round trips are over
    /// somebody's cellular link.
    @Published var fileDiffs: [String: [DiffComputation.Line]] = [:]

    /// Files being read right now, so a section can say so rather than look
    /// empty. An unread file and a file with no hunks are not the same thing.
    @Published var loadingFiles: Set<String> = []

    /// Files folded down to their heading.
    ///
    /// Starts holding EVERY file, so the pane opens as a list of what changed
    /// rather than as the first file's patch — the overview is the thing you
    /// come to this screen for, and on a phone one expanded diff fills the
    /// screen and buries the other nineteen files below it. Opening one is a
    /// tap; scrolling past nineteen open ones is not.
    ///
    /// It also means nothing is fetched until it is asked for: `ChangesFileCard`
    /// only calls `ensure` when it is expanded, so a forty-file branch costs one
    /// round trip on arrival instead of forty.
    @Published var collapsedFiles: Set<String> = []

    /// Which files have EVER been seen, so newly-arrived ones can be collapsed
    /// without re-collapsing anything somebody deliberately opened.
    private var known: Set<String> = []

    // No scroll position here. The Mac's `ChangesStore` holds one because its
    // pane is destroyed when a tmux layout is switched; this one's is not —
    // `PaneHost` keeps every visited pane mounted, so the scroll view is never
    // rebuilt and never needs putting back.

    /// Files the daemon would not render, and why. Held so the row can say so
    /// rather than being a control that does nothing when tapped.
    @Published var unsupported: [String: String] = [:]

    @Published var scope: DiffScope = .branch {
        didSet {
            guard oldValue != scope else { return }
            // The diffs on hand answer the OTHER question. Kept, they would
            // show a committed patch under an "Uncommitted" heading.
            fileDiffs = [:]
            unsupported = [:]
            // The two scopes are two different file lists, so which of them is
            // folded has to be worked out again rather than carried across.
            known = []
            adoptFoldState()
        }
    }

    @Published var loading = false
    @Published var error: String?

    private let core: ClientCore
    private let workspace: String
    /// Whether this store has ever read the worktree.
    private var hasLoaded = false

    init(core: ClientCore, workspace: String) {
        self.core = core
        self.workspace = workspace
    }

    /// The files this scope is about.
    ///
    /// Branch is what the branch COMMITTED — the same thing the summary counts,
    /// deliberately: two numbers describing one worktree have to be the same
    /// number. Local is what has not been committed yet, and it has to come
    /// from the working tree rather than from the committed file list, or a
    /// file an agent just wrote — the only kind of file Local exists to show —
    /// would never appear in it.
    var files: [ChangedFile] {
        switch scope {
        case .branch:
            return changeSet.files
        case .local:
            return dirtyPaths.map { path in
                // Counted from the diff once it has been read, because nothing
                // else counts it: `numstat` answers for commits, and asking git
                // again per file for a number the hunks already contain would
                // be a round trip to restate what is on screen.
                let lines = fileDiffs[path] ?? []
                return ChangedFile(
                    path: path,
                    status: localStatus(path),
                    oldPath: nil,
                    insertions: lines.filter { $0.kind == .added }.count,
                    deletions: lines.filter { $0.kind == .removed }.count,
                    binary: false)
            }
        }
    }

    /// Whether git has never seen this file.
    ///
    /// It has no diff and cannot be given one — `git diff` compares against
    /// something recorded, and nothing is recorded for a file only just
    /// written. It still belongs in the list, since a file an agent created is
    /// the most interesting thing in a local change set, so the row says which
    /// kind of nothing it is showing.
    func isUntracked(_ path: String) -> Bool {
        changeSet.workingTree?.untracked.contains(path) ?? false
    }

    /// Every path git calls dirty, in one order: staged, then unstaged, then
    /// conflicted, then untracked — and never twice, because a file can be
    /// staged and modified again and it is still one file.
    private var dirtyPaths: [String] {
        guard let w = changeSet.workingTree else { return [] }
        var seen = Set<String>()
        return (w.staged + w.unstaged + w.conflicted + w.untracked)
            .filter { seen.insert($0).inserted }
    }

    private func localStatus(_ path: String) -> ChangedFileStatus {
        guard let tree = changeSet.workingTree else { return .modified }
        if tree.conflicted.contains(path) { return .conflicted }
        if tree.untracked.contains(path) { return .untracked }
        return tree.changes?.first(where: { $0.path == path })?.status ?? .modified
    }

    /// Read it, but only the first time.
    ///
    /// The view calls this on every appearance and `load` throws away every
    /// diff it holds — deliberately, since they are diffs against a base that
    /// may have moved — so calling it each time would empty the screen and
    /// refetch it whenever somebody glanced at another tab and came back.
    func loadIfNeeded() async {
        guard !hasLoaded else { return }
        await load()
    }

    func load(fresh: Bool = false) async {
        hasLoaded = true
        loading = true
        defer { loading = false }

        do {
            let data = try await core.call(
                "changes.change_set", ["workspace": workspace, "fresh": fresh])
            changeSet = try JSONDecoder().decode(ChangeSet.self, from: data)
            error = nil
        } catch {
            // A failure is NOT an empty diff.
            //
            // Saying so was a real bug on the Mac once: a machine whose daemon
            // predated this answered NOT_FOUND to every call and the pane drew
            // a worktree with no changes in it. An old daemon is the likeliest
            // reason a phone sees this, so the message says so.
            changeSet = .empty
            self.error = Self.message(for: error)
        }

        // Read again rather than kept: these are diffs against a base that may
        // have just moved, and a stale hunk is worse than a missing one.
        fileDiffs = [:]
        unsupported = [:]
        adoptFoldState()
    }

    /// Collapse anything not seen before; leave everything else as the user
    /// left it.
    ///
    /// Not simply "collapse all on load": a poll that picks up one new commit
    /// would then fold away the file somebody was mid-way through reading.
    private func adoptFoldState() {
        let live = Set(files.map(\.path))
        collapsedFiles = collapsedFiles.intersection(live).union(live.subtracting(known))
        known = live
    }

    /// Read one file's diff, if it has not been read already.
    ///
    /// Idempotent and safe to call from `onAppear` on every row, which is
    /// exactly how it is called: the scroll decides what gets read.
    func ensure(_ path: String) async {
        guard !isUntracked(path) else { return }
        guard fileDiffs[path] == nil, !loadingFiles.contains(path) else { return }
        loadingFiles.insert(path)
        defer { loadingFiles.remove(path) }

        do {
            let data = try await core.call(
                "changes.file_diff",
                ["workspace": workspace, "path": path, "scope": scope.rawValue])
            let diff = try JSONDecoder().decode(FileDiff.self, from: data)
            if let why = diff.unsupported {
                unsupported[path] = Self.reason(why)
                fileDiffs[path] = []
            } else {
                fileDiffs[path] = diff.lines()
            }
        } catch {
            // Left unread rather than recorded as empty, so pulling to refresh
            // tries it again instead of showing a permanent blank.
            fileDiffs[path] = nil
        }
    }

    /// Mark this worktree as read, which is what clears its badge everywhere.
    func markRead() async {
        _ = try? await core.call("changes.mark_read", ["workspace": workspace])
        await load()
    }

    private static func reason(_ code: String) -> String {
        switch code {
        case "binary": return "Binary file"
        case "submodule": return "Submodule"
        case "combined_diff": return "A merge commit, shown against its first parent"
        default: return "This patch could not be read"
        }
    }

    /// The core's answer, as something worth putting on a phone screen.
    private static func message(for error: Error) -> String {
        let text = error.localizedDescription.lowercased()
        if text.contains("not found") || text.contains("unknown method") {
            return "This machine's Far Cooler is too old to review changes."
        }
        return "Couldn't read what this worktree changed."
    }
}

/// The stores, kept alive past the views that show them.
///
/// A `changes` pane is one tab among several, and switching tabs tears its view
/// down — so a `@StateObject` inside `ChangesView` was rebuilt from nothing on
/// every return: scroll back to the top, every fold reopened, every diff
/// re-fetched. That is disruptive in exactly the case the pane is for, which is
/// reading a long diff in more than one sitting.
///
/// Keyed by workspace, because what is being reviewed is the worktree. Held for
/// the lifetime of the connection: a handful of change sets is small next to
/// re-reading one over a phone link, and `Connection` going away is the point at
/// which none of them mean anything anyway.
@MainActor
final class ChangesStores {
    private var stores: [String: ChangesStore] = [:]
    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
    }

    func store(for workspace: String) -> ChangesStore {
        if let existing = stores[workspace] { return existing }
        let made = ChangesStore(core: core, workspace: workspace)
        stores[workspace] = made
        return made
    }
}

// MARK: - One file's patch

/// The daemon's answer for one file, before it becomes drawable lines.
///
/// Structured hunks rather than the unified text the Mac parses: the phone's
/// FFI hands back JSON either way, so taking the numbers the daemon already
/// computed beats re-deriving them from `@@` headers — which is the one part of
/// the Mac's path that can silently be off by one.
private struct FileDiff: Decodable {
    var hunks: [Hunk]
    var unsupported: String?

    struct Hunk: Decodable {
        var lines: [Line]
    }

    struct Line: Decodable {
        var kind: String
        var oldNumber: Int?
        var newNumber: Int?
        var text: String
    }

    /// Flattened into the same line model the agent transcript's diffs use.
    ///
    /// Hunk boundaries are not drawn: a phone has no room for a `@@` header,
    /// and the jump in line numbers between two hunks already says a gap is
    /// there — which is what `ChangesFileCard` renders as a divider.
    func lines() -> [DiffComputation.Line] {
        var out: [DiffComputation.Line] = []
        for hunk in hunks {
            for line in hunk.lines {
                out.append(
                    DiffComputation.Line(
                        id: out.count,
                        kind: {
                            switch line.kind {
                            case "added": return .added
                            case "removed": return .removed
                            default: return .context
                            }
                        }(),
                        oldNumber: line.oldNumber,
                        newNumber: line.newNumber,
                        text: line.text))
            }
        }
        return out
    }
}
