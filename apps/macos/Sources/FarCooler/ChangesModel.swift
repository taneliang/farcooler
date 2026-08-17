import AgentKit
import Foundation
import SwiftUI

/// What a worktree changed.
///
/// Decoded from `farcooler changes … --json`, which is the same path every other
/// read in this app takes. The daemon owns all of it; nothing here computes
/// state, it draws what the runner derived — the same contract the terminal
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
    var status: ChangedFileStatus?
    var oldPath: String?
    var insertions: Int
    var deletions: Int
    var binary: Bool

    var id: String { path }
    var name: String { (path as NSString).lastPathComponent }

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
        case .typeChanged: return "Type changed"
        case .untracked: return "Untracked"
        case .conflicted: return "Conflicted"
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
        case .local: return "Uncommitted"
        }
    }
}

/// What one worktree changed, and the file currently open.
@MainActor
final class ChangesStore: ObservableObject {
    @Published var changeSet: ChangeSet = .empty

    /// Every file's diff, by path, as it is read.
    ///
    /// The whole branch is one scroll, so this holds more than the file being
    /// looked at — but it is filled a file at a time, as each scrolls into view,
    /// rather than up front. A branch that changed forty files would otherwise
    /// pay forty round trips before it drew anything, to show a screenful.
    @Published var fileDiffs: [String: [DiffComputation.Line]] = [:]

    /// Files currently being read, so a section can say so rather than look
    /// empty — an unread file and a file with no hunks are not the same thing.
    @Published var loadingFiles: Set<String> = []

    /// The longest line read so far, in characters.
    ///
    /// The diff's width is set from this rather than measured. A scroll view
    /// that has to work out how wide its content is asks every row, which makes
    /// a lazy stack build all of them — and that is what hung the whole app when
    /// a horizontal scroll wrapped the file list. The font is monospaced, so
    /// characters times one advance is the exact answer without asking anybody.
    @Published private(set) var widestLine = 0

    /// The same files read again with enough context to fill the gaps between
    /// their hunks. Only for files somebody asked to expand.
    @Published var fullDiffs: [String: [DiffComputation.Line]] = [:]

    /// Which gaps have been opened, as `path#index`.
    ///
    /// Per gap rather than per file, because that is what is being asked: a
    /// four-thousand-line file with six hunks has six separate questions in it,
    /// and answering all of them because one was asked buries the change under
    /// the code that did not change — which is the thing a diff exists to
    /// avoid.
    @Published var openGaps: Set<String> = []

    /// Files folded down to their heading. Kept in the store so a tmux layout
    /// switch does not reopen a generated lockfile the user deliberately hid.
    @Published var collapsedFiles: Set<String> = []

    /// Where the scroll was, held OUTSIDE the view.
    ///
    /// A pane belongs to a tmux layout, and switching layouts tears its view
    /// down — so the scroll started at the top every time you came back, which
    /// on a nine-thousand-line diff meant losing your place for looking at
    /// another arrangement for a moment. The store outlives the view, so this
    /// does too.
    ///
    /// Where the scroll was, as an offset the view puts back when it returns.
    ///
    /// The rows are in a LAZY stack, and that is the whole difficulty: when
    /// this pane reappears only a screenful of rows exists, the scroll view's
    /// content is only that tall, and a request to scroll nine thousand lines
    /// down is clamped to the bottom of what has been built. One scroll cannot
    /// get there. So the view asks repeatedly — see `ChangesPane.restore()` —
    /// and each attempt builds more rows for the next one to use.
    ///
    /// A row identity was tried first and is worse, not better. Anchoring to a
    /// row is the operation a lazy stack is supposed to be good at, but the
    /// only rows this view can name are file headings, and the reader is
    /// usually in the middle of a long file whose heading scrolled off screens
    /// ago — unrealized, unreported, and so unavailable to anchor to. It put
    /// every return at the top of whatever file was showing.
    ///
    /// Deliberately NOT `@Published`: it changes on every frame of a scroll,
    /// and republishing that would rebuild the diff under the hand doing the
    /// scrolling.
    var scrollTarget: CGPoint = .zero

    /// The handle used to put the scroll back. Held here for the same reason
    /// the anchor is — the view that would otherwise own it does not survive a
    /// layout switch.
    ///
    /// Not `@Published`, and that matters: the scroll view writes back into
    /// this binding on every frame of a scroll, so publishing it would rebuild
    /// the whole diff — nine thousand rows of it — once per frame while
    /// somebody drags. `$changes.scrollPosition` works regardless; a binding
    /// into an `ObservableObject` does not need the property to be published,
    /// it needs the object to be observed.
    var scrollPosition = ScrollPosition(idType: String.self)

    /// The file the jump bar last went to. Highlights it; hides nothing.
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

    /// The files this scope is about.
    ///
    /// Branch is what the branch COMMITTED — `git diff base…HEAD` — which is
    /// the same thing the sidebar counts, deliberately: two numbers describing
    /// one worktree have to be the same number.
    ///
    /// Local is what has not been committed yet, and it used to be neither. The
    /// scope switched which diff each FILE showed while the file LIST stayed
    /// the branch's committed one — so a file an agent had just written, the
    /// only kind of file Local exists to show, never appeared in the list at
    /// all, and the files that did appear showed an empty local diff. The
    /// working tree already travels in the same response; this reads it.
    var files: [ChangedFile] {
        switch scope {
        case .branch:
            return changeSet.files
        case .local:
            return dirtyPaths.map { path in
                // Counted from the diff once it has been read, because nothing
                // else counts it: `numstat` answers for commits, and asking git
                // a second time per file to fill in a number the hunks already
                // contain would be a round trip to say what is on screen.
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
    /// It has no diff and cannot be given one: `git diff` compares against
    /// something recorded, and there is nothing recorded for a file that was
    /// only just written. It still belongs in the list — a file an agent
    /// created is the most interesting thing in a local change set — so the
    /// row says which kind of nothing it is showing.
    func isUntracked(_ path: String) -> Bool {
        changeSet.workingTree?.untracked.contains(path) ?? false
    }

    /// Every path git calls dirty, in one order: staged, then unstaged, then
    /// untracked, each alphabetical, and never twice — a file can be staged and
    /// modified again, and it is one file.
    private var dirtyPaths: [String] {
        guard let w = changeSet.workingTree else { return [] }
        var seen = Set<String>()
        return (w.staged + w.unstaged + w.conflicted + w.untracked).filter { seen.insert($0).inserted }
    }

    private func localStatus(_ path: String) -> ChangedFileStatus {
        guard let tree = changeSet.workingTree else { return .modified }
        if tree.conflicted.contains(path) { return .conflicted }
        if tree.untracked.contains(path) { return .untracked }
        return tree.changes?.first(where: { $0.path == path })?.status ?? .modified
    }

    /// Whether this store has ever read the worktree.
    private var hasLoaded = false

    /// Read it, but only the first time.
    ///
    /// The pane calls this every time it appears, and it appears every time its
    /// tmux layout is switched back to. `load()` throws away every file diff it
    /// holds — deliberately, since they are diffs against a base that may have
    /// moved — so calling it on each appearance meant glancing at another
    /// arrangement and coming back to an empty diff that had to fetch itself
    /// again, with every expanded gap closed and the scroll at the top. The
    /// poll below is what keeps this current; arriving is not new information.
    func loadIfNeeded() async {
        guard !hasLoaded else { return }
        await load()
    }

    func load(fresh: Bool = false) async {
        hasLoaded = true
        loading = true
        defer { loading = false }

        var args = ["changes", "status", workspace.short, "--json"]
        if fresh { args.append("--fresh") }
        if let data = await client.changesJSON(args) {
            changeSet = (try? JSONDecoder().decode(ChangeSet.self, from: data)) ?? .empty
            error = nil
        } else {
            // A failure is NOT an empty diff. Saying so was a real bug once: a
            // runner whose daemon predates this answered NOT_FOUND to every
            // call, and the pane drew a worktree with no changes.
            changeSet = .empty
            error = client.changesError
        }

        // Read again rather than kept: these are diffs against a base that may
        // have just moved, and a stale hunk is worse than a missing one. The
        // expanded gaps go with them for the same reason — context recovered
        // from a diff that no longer exists is not context.
        fileDiffs = [:]
        fullDiffs = [:]
        fullContext = [:]
        openGaps = []
        tooWide = []
        widestLine = 0
        if let open = selectedFile, !files.contains(where: { $0.path == open }) {
            selectedFile = nil
        }
        let live = Set(files.map(\.path))
        collapsedFiles = collapsedFiles.intersection(live)
    }

    /// Read one file's diff, if it has not been read already.
    ///
    /// Idempotent and safe to call from `onAppear` on every section, which is
    /// exactly how it is called: the scroll decides what gets read.
    func ensure(_ path: String) async {
        // An untracked file has nothing to fetch: git has no recorded version
        // to diff it against, so the call would spend a round trip to come back
        // empty — once per file, and again on every poll.
        guard !isUntracked(path) else { return }
        guard fileDiffs[path] == nil, !loadingFiles.contains(path) else { return }
        await read(path)
    }

    private func read(_ path: String) async {
        loadingFiles.insert(path)
        let lines = await client.changesDiff(
            workspace: workspace.short, path: path, scope: scope)
        loadingFiles.remove(path)
        fileDiffs[path] = lines
        // The expanded copy described the file as it was a moment ago, and its
        // line numbers no longer line up with the hunks around them. Dropped
        // rather than re-fetched: a gap somebody opened once is cheap to open
        // again, and showing the wrong lines in it is not.
        fullDiffs.removeValue(forKey: path)
        fullContext.removeValue(forKey: path)
        openGaps = openGaps.filter { !$0.hasPrefix("\(path)#") }
        tooWide = tooWide.filter { !$0.hasPrefix("\(path)#") }
        widestLine = max(widestLine, lines.map(\.text.count).max() ?? 0)
    }

    /// Open one of the gaps a diff leaves between its hunks.
    ///
    /// The lines are not invented and not cached from anywhere: git is asked
    /// for the same diff again with enough context to cover this gap, and the
    /// answer is kept so opening another gap in the same file usually costs
    /// nothing.
    ///
    /// Enough for THIS gap, not the whole file. Asking for the file — a context
    /// of fifty thousand — collapses every hunk into one, and one hunk the size
    /// of Cargo.lock is past the four-thousand-line cap the daemon puts on a
    /// single file's diff, so the answer came back with no hunks at all and the
    /// gap opened onto nothing. Asked for what the gap actually needs, the
    /// hunks stay separate and the total stays inside the cap.
    func open(gap: Int, of size: Int, in path: String) async {
        let need = min(size + 4, 2_000)
        if (fullContext[path] ?? 0) < need {
            let lines = await client.changesDiff(
                workspace: workspace.short, path: path, scope: scope, context: need)
            guard !lines.isEmpty else {
                // The daemon refused to render a diff that wide. Recorded, so
                // the row can say so rather than being a control that does
                // nothing when clicked.
                tooWide.insert("\(path)#\(gap)")
                return
            }
            fullDiffs[path] = lines
            fullContext[path] = need
            widestLine = max(widestLine, lines.map(\.text.count).max() ?? 0)
        }
        openGaps.insert("\(path)#\(gap)")
    }

    /// How much context each file's expanded copy was read with, so a wider gap
    /// knows it has to ask again.
    private var fullContext: [String: Int] = [:]

    /// Gaps the daemon would not render, so their rows can say why.
    @Published var tooWide: Set<String> = []

    /// The unchanged lines between two line numbers of a file, once it has been
    /// read with full context. Empty until then, which is the same as the gap
    /// not being open.
    func context(in path: String, after: Int, before: Int) -> [DiffComputation.Line] {
        guard let full = fullDiffs[path] else { return [] }
        return full.filter { line in
            guard let n = line.newNumber else { return false }
            return n > after && n < before
        }
    }

    /// Keep the diff current while somebody is looking at it.
    ///
    /// A poll rather than an event, because there is no event to have: the
    /// daemon can see a commit land through two stats, but a file an agent
    /// edited in place is invisible until something reads the worktree — and
    /// watching every worktree in the fleet for that, all the time, is the cost
    /// this product deliberately does not pay to keep a sidebar warm.
    ///
    /// So the one worktree being READ pays it, and only while it is on screen:
    /// the task this runs in is cancelled the moment the pane goes away. The
    /// read is deliberately not `fresh` — a non-fresh change set is answered
    /// from cache after two stats and a digest over the files git already calls
    /// dirty, so a worktree where nothing happened costs almost nothing and one
    /// where something did returns the new set without being asked twice.
    func follow() async {
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled else { return }
            await poll()
        }
    }

    private func poll() async {
        // Not while a full load is running: the two would race to assign
        // `changeSet`, and the loser would be the newer answer half the time.
        guard !loading else { return }
        let args = ["changes", "status", workspace.short, "--json"]
        guard let data = await client.changesJSON(args),
            let next = try? JSONDecoder().decode(ChangeSet.self, from: data),
            next != changeSet
        else { return }

        let before = Dictionary(
            changeSet.files.map { ($0.path, $0) }, uniquingKeysWith: { a, _ in a })
        changeSet = next

        // Only what actually moved is re-read. Dropping every cached diff on
        // any change would re-fetch forty files because one line changed in one
        // of them, and the scroll would empty out and refill under whoever was
        // reading it.
        //
        // Which files those are depends on the scope, because the two ask
        // different questions. A branch file's committed diff changes only when
        // its `numstat` entry does. A local file's diff changes every time
        // anybody saves it, and its entry — a path in a list of dirty paths —
        // says nothing at all about its contents, so every dirty file that has
        // been read is re-read. That set is small by construction: it is the
        // files git already calls dirty, intersected with the ones scrolled to.
        let stale: [String]
        switch scope {
        case .branch:
            stale = next.files.filter { before[$0.path] != $0 }.map(\.path)
        case .local:
            stale = files.map(\.path)
        }

        let live = Set(files.map(\.path))
        for path in fileDiffs.keys where !live.contains(path) {
            fileDiffs.removeValue(forKey: path)
        }
        for path in stale where fileDiffs[path] != nil {
            // Re-read rather than merely forgotten: nothing would ask for it
            // again on its own, because the row that asks does so once, keyed
            // on a path that has not changed.
            await read(path)
        }
        if let open = selectedFile, !live.contains(open) {
            selectedFile = nil
        }
        // The sidebar counts come from a different call, and a diff that has
        // moved on while the row beside it has not is the same worktree
        // described two ways — which is the bug the base fix was about.
        await client.refreshChangesInbox()
    }

    func markRead() async {
        await client.changesMarkRead(workspace: workspace.short)
        await load()
    }
}
