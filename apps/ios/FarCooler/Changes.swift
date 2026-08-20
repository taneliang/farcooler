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

/// One commit on this branch, as the history sheet draws it.
///
/// Four fields and no counts, which is not an oversight and cannot be fixed
/// here: `ChangeCommit` in the protocol carries `files_changed`, `insertions`
/// and `deletions`, but `commits_since` in crates/daemon/src/change_set.rs
/// writes 0 into all three, and `change_set_json` in
/// crates/client/src/session.rs does not project them onto the wire at all. So
/// a row says what is actually known — who, when, and what they called it —
/// and the `+N -M` for a commit appears once it is SELECTED, summed from the
/// file list that selection fetches. Two zeroes dressed up as a count would be
/// worse than no count.
struct ChangeCommit: Decodable, Equatable, Identifiable {
    var sha: String
    var subject: String
    var author: String
    var timestamp: Int

    var id: String { sha }
    var short: String { String(sha.prefix(8)) }

    /// How long ago this commit was made, in the same shorthand a working
    /// agent's row already uses: `12m`, `3h`, `2d`.
    ///
    /// `now` is an ARGUMENT rather than a `Date()` read in here, for the reason
    /// `Terminal.displayDuration(at:)` spells out at length — a clock read
    /// inside a property is an input SwiftUI cannot observe, so the string
    /// freezes until something unrelated forces a redraw. Nothing drives this
    /// one on a timer and nothing needs to: `CommitHistorySheet` reads the
    /// clock once, when it opens, which is the only moment anybody sees these.
    func age(at now: Date) -> String {
        let seconds = now.timeIntervalSince1970 - Double(timestamp)
        // A commit dated in the future is a clock that disagrees, not a
        // negative age — which happens for real when a runner and a phone are
        // in different places about NTP. "now" is the smallest honest thing to
        // say about it.
        guard seconds >= 60 else { return "now" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        if seconds < 86_400 { return "\(Int(seconds / 3600))h" }
        return "\(Int(seconds / 86_400))d"
    }

    /// The full date, for the header of the commit that is actually on screen.
    ///
    /// Absolute rather than relative, and deliberately so: the summary card is
    /// not a list being scanned, it is the one line that says WHICH commit this
    /// is, and an absolute date needs no clock — so nothing here freezes at
    /// whatever `2d` happened to be when the card was last built.
    var made: String {
        Date(timeIntervalSince1970: Double(timestamp))
            .formatted(date: .abbreviated, time: .shortened)
    }
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
/// The Mac's three, spelled differently in one place worth defending. There the
/// sha of the commit being shown travels BESIDE the scope, in a second
/// property, because `DiffScope` on the Mac is the tag of a segmented control
/// that also has to be `Codable`, and an associated value takes the synthesized
/// `rawValue` and `Codable` with it.
///
/// Here it is an associated value, and the reason is the wire.
/// `changes.file_diff` takes ONE `scope` string, and `Session::file_diff` in
/// crates/client/src/session.rs matches it as `"branch"`, `"local"`,
/// `"staged"`, `"unstaged"` — and `sha => Kind::Commit(sha)` for anything else.
/// So by the time a comparison reaches this client's FFI it is already a single
/// value carrying both facts, and splitting it in two on the way there would
/// manufacture exactly the state the Mac then has to keep consistent by hand: a
/// `.commit` scope with no sha beside it, which is a diff of nothing. It also
/// makes switching from one commit to the next a change of `scope` like any
/// other, so the `didSet` that throws away the diffs on hand already covers it
/// — on the Mac that is a second, hand-written reset inside `select(commit:)`.
///
/// What the Mac gives up for its spelling, this client does not use.
/// `CaseIterable` is hand-written below exactly as it is there; `Codable` is
/// not needed because no scope is persisted on a phone; and the `rawValue` that
/// used to be the FFI argument is now `wire`, which is the single place the
/// wire's rule is written down.
enum DiffScope: CaseIterable, Hashable, Identifiable {
    case branch
    case local
    /// One commit against its FIRST parent.
    ///
    /// That is what `Selector::Commit` diffs in crates/daemon/src/file_diff.rs
    /// — `{sha}^1` against `{sha}`, falling back to the empty tree for a root
    /// commit — and deliberately not `git show`, which prints a combined diff
    /// for a merge that an ordinary parser reads as nonsense. `changes
    /// commit_files` counts the same comparison, so the file list and the
    /// patches under it cannot disagree about what the commit did.
    case commit(String)

    /// The two the comparison control offers, which is not every case.
    ///
    /// A `Commit` segment would be a control that cannot answer its own
    /// question: tapping it says nothing about WHICH commit, so it would have
    /// to open something, and the thing it would open is the History row
    /// directly beneath it. While a commit is on screen the comparison control
    /// is not drawn at all — see `ChangesView.summary`, where the phone's
    /// answer differs from the Mac's unselected segment and says why.
    static var allCases: [DiffScope] { [.branch, .local] }

    var id: String { wire }

    /// The `scope` argument `changes.file_diff` is given.
    ///
    /// A sha is its own scope name, which is not a coincidence but the
    /// protocol: see the match in `Session::file_diff`, whose arm for anything
    /// unrecognized is `sha => Kind::Commit(sha.to_string())`. Only a
    /// full-length hex sha out of `changes.change_set` can reach that arm from
    /// here, so it cannot collide with the names above.
    var wire: String {
        switch self {
        case .branch: return "branch"
        case .local: return "local"
        case .commit(let sha): return sha
        }
    }

    var label: String {
        switch self {
        case .branch: return "Branch"
        case .local: return "Uncommitted"
        case .commit: return "Commit"
        }
    }

    /// The sha, when a commit is what is being shown, and nil otherwise. The
    /// pattern match written once, since half a dozen places ask.
    var commitSha: String? {
        if case .commit(let sha) = self { return sha }
        return nil
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
            // show a committed patch under an "Uncommitted" heading — or, now
            // that a scope carries its sha, one commit's patch under another
            // commit's subject, which is the same bug wearing a better
            // disguise. Because the sha is part of the value, commit-to-commit
            // is a change of scope like any other and lands here for free; the
            // Mac needs a second reset written out inside `select(commit:)`.
            fileDiffs = [:]
            unsupported = [:]
            // Every scope is a different file list, so which of them is folded
            // has to be worked out again rather than carried across.
            known = []
            // A commit's file list is FETCHED rather than derived from the
            // change set, so leaving the previous one in place would put the
            // old commit's files under the new one's subject for as long as the
            // round trip takes — which on a phone link is long enough to read.
            commitFiles = []
            commitUnreadable = false
            generation &+= 1
            adoptFoldState()
        }
    }

    /// What the commit on screen touched, from `changes.commit_files`.
    ///
    /// A separate call and not a filter over the branch's files, because the
    /// change set carries per-file counts for the WHOLE branch and nothing
    /// per-commit, on purpose: a branch that regenerated a lockfile touches
    /// thousands of files, and shipping that per commit to draw a list is the
    /// cost this product does not pay — least of all over somebody's cellular
    /// link.
    @Published private(set) var commitFiles: [ChangedFile] = []

    /// Set when that call failed, so the pane can say WHICH nothing it is
    /// showing. A commit can genuinely stop being readable while somebody is
    /// looking at it — an amend or a rebase rewrites the branch underneath —
    /// and "nothing changed here" is the wrong sentence for a commit that could
    /// not be read at all.
    @Published private(set) var commitUnreadable = false

    /// Whether the history sheet is up.
    ///
    /// On the store rather than in a view, because the two controls that open
    /// it live in two different view trees: the History row is `ChangesView`'s,
    /// and `ChangesToolbarMenu` belongs to `PaneHost`'s toolbar for the reason
    /// its own comment gives. A `@State` in either is invisible to the other,
    /// and the store is the one thing both already hold.
    @Published var showingHistory = false

    /// Bumped every time the cached diffs are thrown away.
    ///
    /// A read in flight when the reader moves on must not be filed under what
    /// they moved to. Every path in `changes.file_diff` and
    /// `changes.commit_files` answers with a well-formed result, and the stale
    /// one is indistinguishable from the fresh one once it has landed — a
    /// patch keyed on a path alone slots perfectly under a heading that is now
    /// showing a different commit. So every read records the generation it
    /// started in and compares before storing anything.
    ///
    /// It is `@Published` because `ChangesFileCard` keys its `task` on it too:
    /// nothing else in the view asks for a file's diff a second time, so
    /// clearing the cache under a heading that stayed on screen would leave
    /// that file at "Reading…" forever. That is exactly what switching between
    /// two commits that touch the same file does.
    @Published private(set) var generation = 0

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
    /// Commit is one commit against its FIRST parent, which is what the daemon
    /// diffs for `Selector::Commit` and what `changes.commit_files` counts —
    /// never `git show`, whose combined diff for a merge an ordinary parser
    /// reads as nonsense. Both halves of this view come from that same
    /// comparison, so the file list and the patches under it cannot disagree
    /// about what the commit did.
    var files: [ChangedFile] {
        switch scope {
        case .branch:
            return changeSet.files
        case .commit:
            return commitFiles
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
        // Nothing inside a commit is untracked — committing is what tracking IS
        // — and the working tree's list is about right now, not about then.
        // Left to speak here it would refuse to fetch the diff of a file some
        // commit changed and somebody has since deleted and rewritten, and the
        // card would claim git had never seen a file that commit demonstrably
        // contains.
        guard scope.commitSha == nil else { return false }
        return changeSet.workingTree?.untracked.contains(path) ?? false
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
            // Saying so was a real bug on the Mac once: a runner whose daemon
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
        // Before the commit is re-read, so the read below records the
        // generation it will be checked against rather than the one it
        // replaced.
        generation &+= 1
        // A commit's patch is immutable — it is the difference between two
        // objects that already exist — but its FILE LIST is not something this
        // store can recover on its own, and everything after this point asks
        // `files` what is on screen. In this scope `files` IS that list, so a
        // refresh that skipped it would fold and unfold an empty pane.
        if let sha = scope.commitSha {
            await readCommitFiles(sha)
        }
        adoptFoldState()
    }

    // MARK: - One commit at a time

    /// The commits this branch made, newest first.
    ///
    /// `commits_since` in crates/daemon/src/change_set.rs logs them with
    /// `--reverse` so `changes status` reads as a story from the base forward,
    /// which is right for a transcript and wrong for a picker: the commit
    /// somebody reaches for is almost always the one that was just made.
    var commitsNewestFirst: [ChangeCommit] { changeSet.commits.reversed() }

    /// What the change set still knows about the commit on screen, if it knows
    /// anything.
    ///
    /// Nil for a commit that has left the branch, which an amend or a rebase
    /// mid-review does exactly — while the diff itself usually stays readable,
    /// because the object it names is still in the repository until git prunes
    /// it. So this going nil is not an error; it is the one moment the header
    /// has to stop claiming a subject and an author it can no longer support.
    var selectedCommitInfo: ChangeCommit? {
        guard let sha = scope.commitSha else { return nil }
        return changeSet.commits.first { $0.sha == sha }
    }

    /// `+N` for the commit on screen, summed from its own file list.
    ///
    /// Summed rather than read off `ChangeCommit`, for the reason that type's
    /// own comment gives: the daemon hardcodes its three count fields to zero
    /// and the wire drops them. These are real `--numstat` numbers.
    var commitInsertions: Int { commitFiles.reduce(0) { $0 + $1.insertions } }
    var commitDeletions: Int { commitFiles.reduce(0) { $0 + $1.deletions } }

    /// Show one commit, against its first parent.
    func select(commit sha: String) async {
        // Already showing it. Re-reading would spend a round trip to arrive at
        // the same list and would take the reader's scroll position and every
        // file they had opened with it.
        guard scope != .commit(sha) else { return }
        // Everything that has to be forgotten is forgotten by `scope`'s own
        // `didSet`, including the generation bump the read below is checked
        // against.
        scope = .commit(sha)
        let asked = generation
        // The same flag a full load raises, and for the same reason: this
        // scope's file list is empty until the call answers, and an empty list
        // with nothing to say about why is a pane asserting that the commit
        // changed nothing.
        loading = true
        await readCommitFiles(sha)
        // Lowered by hand rather than by a `defer`, because a `defer` would
        // lower it for a selection that has already been superseded — two
        // commits chosen in quick succession finish in the order the daemon
        // answers, not the order they were asked for. The later one owns the
        // spinner, and clearing it here would blink "nothing changed" over its
        // list on the way past.
        guard asked == generation else { return }
        loading = false
        adoptFoldState()
    }

    /// Back to the whole branch: merge base to HEAD, every commit at once.
    ///
    /// No round trip. The change set never stopped being held — only the
    /// commit's patches have to go, and `scope`'s `didSet` is what drops them.
    func showWholeBranch() {
        scope = .branch
    }

    /// Which files one commit touched, from `changes.commit_files`.
    ///
    /// The phone's advantage over the Mac's path, and it is a real one: that
    /// client shells out to `changes files`, which has no `--json` and prints a
    /// fixed-width table it has to parse back — counts off the front, all of
    /// the rest taken as the path so a filename containing a space survives.
    /// Here the FFI hands back the same `file_change_json` the change set's own
    /// files come through, so this is a `JSONDecoder` call and there is no
    /// parser to get wrong.
    private func readCommitFiles(_ sha: String) async {
        let asked = generation
        do {
            let data = try await core.call(
                "changes.commit_files", ["workspace": workspace, "sha": sha])
            let answer = try JSONDecoder().decode(CommitFiles.self, from: data)
            guard asked == generation else { return }
            commitFiles = answer.asDetermined
            commitUnreadable = false
        } catch {
            guard asked == generation else { return }
            // Recorded rather than left as an empty list: a commit that could
            // not be read and a commit that changed nothing are two different
            // things, and only one of them is worth a warning triangle.
            commitFiles = []
            commitUnreadable = true
        }
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
        let asked = generation
        loadingFiles.insert(path)
        defer { loadingFiles.remove(path) }

        do {
            let data = try await core.call(
                "changes.file_diff",
                // The sha IS the scope for a commit — see `DiffScope.wire`,
                // which is the only place that rule is spelled out.
                ["workspace": workspace, "path": path, "scope": scope.wire])
            let diff = try JSONDecoder().decode(FileDiff.self, from: data)
            // What was being compared changed while this was in flight, so
            // these lines answer a question nobody is asking any more. Stored
            // anyway they would file perfectly, under a heading that is now
            // showing a different commit — a wrong diff that looks exactly like
            // a right one.
            guard asked == generation else { return }
            if let why = diff.unsupported {
                unsupported[path] = Self.reason(why)
                fileDiffs[path] = []
            } else {
                fileDiffs[path] = diff.lines()
            }
        } catch {
            // Left unread rather than recorded as empty, so pulling to refresh
            // tries it again instead of showing a permanent blank.
            guard asked == generation else { return }
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
            return "This runner's Far Cooler is too old to review changes."
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

// MARK: - One commit's files

/// The answer to `changes.commit_files`, before its statuses are believed.
///
/// This is the trap on the commit path and it is worse here than on the Mac,
/// because here it is invisible. That client shells out to `changes files`,
/// which prints counts and a path and NO status at all, so it was forced to
/// notice. The FFI projects the same `FileChange` the change set's files come
/// through, `status` and all — and the status is very largely made up.
///
/// `commit_files` in crates/daemon/src/file_diff.rs runs
/// `git diff --numstat -z --find-renames`, and `parse_numstat_z` in
/// change_set.rs turns each record into a `FileChange`. `--numstat` counts
/// lines; it never says added or deleted. So that parser writes
/// `FileStatus::Modified` into every record it does not recognize as a rename,
/// and the wire faithfully carries "modified" for a file the commit CREATED.
/// Decoded as it stands, a brand-new file would be labeled "Modified" and a
/// deleted one likewise — a claim nothing in the pipeline ever checked.
///
/// So the status is thrown away, and `ChangesFileCard` says "Changed" instead.
private struct CommitFiles: Decodable {
    var files: [ChangedFile]

    /// The same files, keeping only the status the daemon actually determined.
    ///
    /// A rename survives, and only a rename: it is the one branch of
    /// `parse_numstat_z` that git itself established, via `--find-renames` and
    /// the two extra NUL-separated path fields a rename record carries.
    /// Everything else is the parser's `else`. `binary` survives for the same
    /// reason — `--numstat` prints `-` for both counts on a file it cannot
    /// count, which is a fact and not a fallback.
    var asDetermined: [ChangedFile] {
        files.map { file in
            var copy = file
            if copy.status != .renamed { copy.status = nil }
            return copy
        }
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
