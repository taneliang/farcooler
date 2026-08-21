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

/// One commit on this branch, as the history list and the commit header draw
/// it.
///
/// ## Everything past `timestamp` is optional, and has to be
///
/// Swift's synthesized `Decodable` throws on a missing key, and this type is
/// decoded inside `ChangeSet` — so a phone that met a runner whose daemon
/// predates any one of these fields would fail to decode the ENTIRE change set,
/// every file and every commit, over one absent key. Optional is therefore not
/// a nicety here; it is what lets an older runner show a commit with no
/// rationale and no counts, which is exactly what such a runner knows.
///
/// ## The counts
///
/// `files_changed`, `insertions` and `deletions` were hardcoded zeroes in
/// `commits_since` (crates/daemon/src/change_set.rs) for as long as that
/// function existed, and `change_set_json` did not project them, so both
/// clients worked around it by summing the file list of whichever commit had
/// been SELECTED — which meant a history row could not say what a commit did
/// until it was opened. `--shortstat` on the same `git log` now answers, with
/// `--diff-merges=first-parent` so a merge's row agrees with the file list it
/// opens.
///
/// **Zero still means unknown**, which is why `counts` below returns nil for
/// it. A runner on a git older than 2.31 rejects `--diff-merges` and the daemon
/// retries without it, leaving merges at the zeroes they always had; and a
/// commit that genuinely changed nothing is rare enough that showing `+0 −0`
/// for it is a worse trade than staying quiet. The number that IS shown is the
/// same number the summed file list produces — `--shortstat` is the sum of the
/// same commit's `--numstat`, renames detected on both sides.
struct ChangeCommit: Decodable, Equatable, Identifiable {
    var sha: String
    var subject: String
    /// Everything the author wrote after the subject line.
    ///
    /// `body = 3` has been in `ChangeCommit` in the protocol since the message
    /// existed, and it was dropped on the way out rather than never carried:
    /// `change_set_json` in crates/client/src/session.rs projected four fields
    /// and this was not one of them.
    ///
    /// It matters more for agent work than for human work. An agent's commit
    /// body is usually the closest thing to a written rationale for what it
    /// did — why this approach, what it decided against, what it could not
    /// finish — and it is the cheapest context available before reading a
    /// single line of diff, which on a phone between two sets is the only
    /// context there is time for.
    var body: String?
    var author: String
    var timestamp: Int
    var filesChanged: Int?
    var insertions: Int?
    var deletions: Int?

    enum CodingKeys: String, CodingKey {
        case sha, subject, body, author, timestamp, insertions, deletions
        case filesChanged = "files_changed"
    }

    var id: String { sha }
    var short: String { String(sha.prefix(8)) }

    /// `+N −M`, when the daemon actually counted them.
    ///
    /// Nil rather than `(0, 0)` for the reason the type comment gives: on this
    /// wire a zero is indistinguishable from "this runner could not tell you",
    /// and a row claiming `+0 −0` about a merge on an old git would be stating
    /// a fact nobody established.
    var counts: (insertions: Int, deletions: Int)? {
        let plus = insertions ?? 0
        let minus = deletions ?? 0
        guard plus > 0 || minus > 0 else { return nil }
        return (plus, minus)
    }

    /// The body with its surrounding whitespace gone, or nil if there was none.
    ///
    /// Nil rather than an empty string so every caller's `if let` is the same
    /// shape whether the field was absent (an older runner) or present and
    /// empty (a commit with only a subject, which is most of them).
    var bodyText: String? {
        let trimmed = (body ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    /// The first paragraph worth putting in a list row.
    ///
    /// The first paragraph rather than the first line, because a body is prose
    /// that was wrapped for a terminal and its first line is half a sentence.
    ///
    /// Trailer-only paragraphs are skipped. Every commit in this repository
    /// ends in `Co-Authored-By:` and most agent commits add more of the same,
    /// and a row whose one line of preview reads `Co-Authored-By: Claude …` has
    /// spent the most valuable line on the screen saying nothing. Skipped, not
    /// stripped: the full body is shown in the header, trailers and all, since
    /// on the screen that is ABOUT one commit they are part of what it says.
    var bodyPreview: String? {
        guard let bodyText else { return nil }
        for paragraph in bodyText.components(separatedBy: "\n\n") {
            let lines = paragraph.split(separator: "\n", omittingEmptySubsequences: true)
            guard !lines.isEmpty else { continue }
            guard lines.allSatisfy({ Self.isTrailer(String($0)) }) else {
                return lines.joined(separator: " ").trimmingCharacters(in: .whitespaces)
            }
        }
        return nil
    }

    /// `Key: value` at the start of a line, which is git's own shape for a
    /// trailer. Deliberately loose — this decides what to show first, not what
    /// to keep, so a false positive costs a paragraph's place in a preview and
    /// never a line of the body.
    private static func isTrailer(_ line: String) -> Bool {
        guard let colon = line.firstIndex(of: ":"), colon > line.startIndex else { return false }
        let key = line[line.startIndex..<colon]
        guard line.index(after: colon) < line.endIndex else { return false }
        guard line[line.index(after: colon)] == " " else { return false }
        return key.allSatisfy { $0.isLetter || $0 == "-" }
    }

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

    /// Whether a tool wrote this file rather than a person or an agent.
    ///
    /// It exists because of what it does to the two numbers at the top of the
    /// screen. A branch that touched eleven source files and regenerated
    /// `Cargo.lock` reads as four thousand lines changed, and the reader — who
    /// has ninety seconds — has no way to tell that from a branch that really
    /// did rewrite four thousand lines. Counted apart, the same branch reads as
    /// `+300 −120`, and a quieter line underneath says a lockfile moved too.
    ///
    /// **This rule belongs on the host, beside `crates/core/src/feed.rs`.** It
    /// is a fact about a repository — what its build regenerates, what its
    /// `.gitattributes` marks `linguist-generated`, what its own conventions
    /// call vendored — and the host is the only place that can read any of
    /// that. Deciding it here means two clients with two lists, and a phone
    /// that is wrong about a repository it has never seen. It is here only
    /// because putting it there is a protocol field plus a daemon rule plus
    /// both clients, which is a larger change than this screen; when that
    /// lands, this becomes a fallback for older runners and nothing else.
    ///
    /// Deliberately conservative in the meantime. Everything below is a name a
    /// tool writes and nobody edits by hand, so a false positive costs a fold
    /// the reader can open with one tap; a rule like "anything under `vendor/`"
    /// would start folding files people wrote.
    var isGenerated: Bool { Self.isGenerated(path) }

    static func isGenerated(_ path: String) -> Bool {
        let name = (path as NSString).lastPathComponent
        if generatedNames.contains(name) { return true }
        // Suffixes rather than whole names, for the families whose stem varies:
        // `pnpm-lock.yaml` is matched above, `schema.generated.ts` here.
        return generatedSuffixes.contains { name.hasSuffix($0) }
    }

    private static let generatedNames: Set<String> = [
        "Cargo.lock", "package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lockb",
        "Gemfile.lock", "Podfile.lock", "poetry.lock", "uv.lock", "composer.lock",
        "go.sum", "Package.resolved", "flake.lock", "gradle.lockfile", "mix.lock",
        // An .xcodeproj is generated state in this repository specifically —
        // `apps/ios/generate-project.py` writes it — and it is the file that
        // most often makes an iOS branch look twice its size.
        "project.pbxproj",
    ]

    private static let generatedSuffixes: [String] = [
        ".generated.swift", ".generated.ts", ".generated.go", ".g.dart", ".pb.go",
        ".pb.rs", "_pb2.py",
    ]
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

    /// Green and red are the diff's own colors — added and removed lines — so
    /// a created or deleted FILE wearing them needs no learning. Orange is
    /// spoken for: across this app it means something wants you, from a blocked
    /// agent to a runner that will not connect, and a conflict is exactly that.
    /// A rename is not, so it is gray like a modification and the letter is
    /// what separates them.
    ///
    /// Written out rather than left to a `default:` arm. Renamed and copied
    /// reached commit rows only once the daemon started merging
    /// `--name-status`, and the Mac was tinting both orange in the meantime —
    /// a divergence nobody could see while the phone never drew one.
    var tint: Color {
        switch self {
        case .added, .untracked: return .green
        case .deleted: return .red
        case .conflicted: return .orange
        case .modified, .renamed, .copied, .typeChanged: return .secondary
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

    /// The one file whose patch is open, or nil when the pane is a list of
    /// headings.
    ///
    /// ONE, not a set, and that is a change of shape rather than a tightening
    /// of a limit. Every file starts folded so the pane opens as a list of what
    /// changed rather than as the first file's patch — the overview is what you
    /// come to this screen for, and on a phone one expanded diff already fills
    /// the screen. Letting a second one open buries the first under it, and by
    /// the fifth the list has stopped being navigable in either direction.
    ///
    /// Holding exactly one is also what makes the rest of this screen possible.
    /// "Where am I" has an answer, so the bar at the bottom can say `7 of 23`;
    /// "the next file" has an answer, so Next and Previous can exist at all;
    /// and the bookmark has one path to write down instead of a set whose
    /// meaning on re-entry would be a guess. A set could express none of that.
    ///
    /// It also keeps the phone's cellular cost at one file: `ChangesFileCard`
    /// calls `ensure` only when it is expanded, so a forty-file branch costs
    /// one round trip on arrival rather than forty.
    @Published private(set) var expandedFile: String?

    // No scroll offset here, and none in the bookmark either — see
    // `ReviewPosition`, which explains at length why a position on this screen
    // is a PATH. The Mac's `ChangesStore` holds an offset because its pane is
    // destroyed when a tmux layout is switched; this one's is not, since
    // `WorkspaceView` keeps every visited pane mounted, so within one run of the app
    // the scroll never moves and there is nothing to put back.

    /// A file the view should scroll to, once.
    ///
    /// The identity is what makes it fire: tapping the same file in the index
    /// twice, or pressing Next into a file that is already the expanded one
    /// after a refresh reshuffled the list, has to move the scroll both times,
    /// and a bare `String?` compares equal and does nothing the second time.
    /// The view clears it after acting, so nothing re-scrolls under somebody
    /// who has since scrolled away.
    @Published var jump: Jump?

    struct Jump: Equatable {
        let id = UUID()
        let path: String
    }

    /// The id of the summary card, so "land at the top and say why" has
    /// somewhere to land.
    ///
    /// A jump names a path and every card answers to one; the summary answers
    /// to this, which is a string no path can be. That keeps one channel for
    /// every scroll on this screen rather than a second mechanism for the one
    /// case that is not a file.
    static let topAnchor = "changes.top"

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
            // Every scope is a different file list, so the open file goes with
            // it rather than being carried across — the same path can exist in
            // both and mean two different patches, which is the one way this
            // screen could show a commit's hunks under the branch's heading.
            expandedFile = nil
            topFile = nil
            // The cards that were on screen belong to the list that is being
            // replaced. A path can exist in both scopes, so a leftover entry
            // here would let a card nobody can see decide where the bookmark
            // says the reader is.
            onScreen = []
            // A commit's file list is FETCHED rather than derived from the
            // change set, so leaving the previous one in place would put the
            // old commit's files under the new one's subject for as long as the
            // round trip takes — which on a phone link is long enough to read.
            commitFiles = []
            commitUnreadable = false
            generation &+= 1
            adoptExpansion()
            rememberPosition()
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
    /// and `ChangesToolbarMenu` belongs to `WorkspaceView`'s toolbar for the reason
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

    /// What the reader wants to tell the agent, collected across the review.
    ///
    /// A separate object rather than more `@Published`s here, because its
    /// lifetime is different in the way that matters: everything else on this
    /// store is derived from the daemon and can be thrown away and re-read,
    /// while a comment is the only thing on this screen that a person typed and
    /// that nothing else in the world has a copy of. It persists itself; see
    /// `ReviewCommentQueue`.
    let comments: ReviewCommentQueue

    /// The file at the top of the screen, when none is expanded.
    ///
    /// Not `@Published`: nothing draws it. It exists only so the bookmark has
    /// something to say about a reader who was going down the list of headings
    /// rather than reading a patch, which is most of the first window of a
    /// review.
    private var topFile: String?

    /// The cards currently on screen, for the same reason and with the same
    /// silence. See `noteVisible`.
    private var onScreen: Set<String> = []

    /// A bookmark from a previous run of the app, offered but not applied.
    ///
    /// Offered, and that is deliberate. Restoring silently would put somebody
    /// who opened this pane to check one thing into the middle of a patch they
    /// were reading yesterday, and — worse — a branch that moved underneath
    /// them in between would land them somewhere that looks like where they
    /// were and is not. So the app says where it thinks they were and waits.
    @Published private(set) var resume: ReviewPosition?

    /// Why the resume did not land exactly where it was aimed.
    ///
    /// A position is a HINT and is allowed to be wrong: an agent that kept
    /// working overnight deletes files, rewrites commits, and renames the thing
    /// that was being read. The rule is to land as close as possible — the top
    /// of the branch, the top of the file — and to say so, rather than to
    /// pretend the bookmark was honored or to refuse to move at all.
    @Published var resumeNote: String?

    private let core: ClientCore
    private let workspace: String
    /// Whether this store has ever read the worktree.
    private var hasLoaded = false
    /// Whether the bookmark has already had its one chance to be offered.
    private var hasOfferedResume = false

    init(core: ClientCore, workspace: String) {
        self.core = core
        self.workspace = workspace
        self.comments = ReviewCommentQueue(core: core, workspace: workspace)
    }

    /// The files this scope is about.
    ///
    /// Branch is what the branch COMMITTED — the same thing the summary card
    /// above this list counts, but no longer the same thing the FLEET counts:
    /// `7927c13` moved `change_set::shortstat` to compare the base against the
    /// WORKING TREE and taught it to count untracked lines, so the `+N −M` on a
    /// dirty worktree's row in Fleet and in Needs You reads higher than this
    /// one. Local is what has not been committed yet, and it has to come
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
        adoptExpansion()
        offerResumeOnce()
    }

    // MARK: - Where you stopped

    /// Read the bookmark, once per run of the app, and offer it.
    ///
    /// After the first load rather than before it, because the offer names a
    /// commit's subject and a file, and both of those are things only the
    /// change set can supply. Offered once: a pull to refresh is somebody
    /// asking about the branch they are already reading, and re-offering to
    /// take them somewhere else would be the app arguing with them.
    private func offerResumeOnce() {
        guard !hasOfferedResume else { return }
        hasOfferedResume = true
        guard error == nil else { return }
        guard let saved = ReviewBookmarks.read(workspace), saved.isSomewhere else { return }
        // Already there. Two ways that happens and both must be caught, or the
        // card is an interruption that resolves to nothing:
        //
        // - the store outlived a tab switch rather than a termination, so every
        //   part of the position still matches what is on screen; or
        // - the position IS the top of the list — somebody who opened the pane,
        //   read the first heading and got called away is not somewhere that
        //   needs restoring to.
        let unmoved =
            saved.scope == scope.wire && saved.file == expandedFile
            && saved.topFile == topFile
        let atTheTop =
            saved.scope == scope.wire && saved.file == nil
            && saved.topFile == reviewOrder.first?.path
        guard !unmoved, !atTheTop else { return }
        resume = saved
    }

    /// Take the offer.
    ///
    /// Every step is allowed to fail, and each failure lands one level further
    /// out with a sentence saying which one gave way. That is the whole
    /// contract of this feature: a bookmark is a hint about a branch that an
    /// agent has probably kept editing, and the alternative to landing nearby
    /// and saying so is either lying about where you are or refusing to move.
    func applyResume() async {
        guard let saved = resume else { return }
        resume = nil
        resumeNote = nil

        if let sha = ReviewPosition.sha(in: saved.scope) {
            guard changeSet.commits.contains(where: { $0.sha == sha }) else {
                // The commit was amended or rebased away overnight, which for
                // an agent-authored branch is not an edge case. The branch as a
                // whole still contains its work, so that is where this lands.
                resumeNote =
                    "That commit isn't on the branch anymore — it was amended or rebased. "
                    + "This is the whole branch instead."
                showWholeBranch()
                jump = Jump(path: Self.topAnchor)
                return
            }
            await select(commit: sha)
        } else if saved.scope == "local" {
            scope = .local
        } else {
            scope = .branch
        }

        let live = files.map(\.path)
        if let file = saved.file {
            guard live.contains(file) else {
                resumeNote =
                    "\((file as NSString).lastPathComponent) isn't in this diff anymore, "
                    + "so this is the top."
                jump = Jump(path: Self.topAnchor)
                return
            }
            expand(file)
        } else if let top = saved.topFile, live.contains(top) {
            jump = Jump(path: top)
        }
    }

    func dismissResume() {
        resume = nil
        // Not forgotten, only declined. Somebody who taps the X wants this
        // screen out of the way, not a bookmark deleted — the next window may
        // well be the one they meant to resume in.
        resumeNote = nil
    }

    /// Write down where the reader is.
    ///
    /// Called on every move rather than on the way out, because there is no way
    /// out to hook: iOS terminates a suspended app without telling it, which in
    /// the situation this screen is built for is the NORMAL way a review window
    /// ends. `UserDefaults` coalesces writes itself, and the record is three
    /// short strings.
    private func rememberPosition() {
        // Not while an offer is on the table. The first cards realize
        // themselves the moment the pane draws, and each one reports its
        // visibility — so without this, arriving at the screen would overwrite
        // the very position the "Continue where you stopped" card is offering,
        // and a reader who tapped it after a second termination would be taken
        // to the top of the branch they had just opened.
        guard resume == nil else { return }
        ReviewBookmarks.write(
            ReviewPosition(
                scope: scope.wire, file: expandedFile, topFile: topFile,
                savedAt: Date().timeIntervalSince1970),
            for: workspace)
    }

    /// A card said whether it is on screen. See `topFile`.
    ///
    /// The set is kept here rather than in a `@State` in the view on purpose:
    /// this fires several times a second while somebody scrolls, and a
    /// `@Published` — or a `@State` in `ChangesView` — would re-evaluate a
    /// forty-card lazy stack on every one of them. Nothing draws this, so
    /// nothing needs to be told about it.
    func noteVisible(_ path: String, isVisible: Bool) {
        if isVisible {
            onScreen.insert(path)
        } else {
            onScreen.remove(path)
        }
        // The topmost visible card, which is the reading-order first one that
        // is on screen — not "the last card that appeared", which depends on
        // which way the scroll was going.
        let top = reviewOrder.first { onScreen.contains($0.path) }?.path
        guard top != topFile else { return }
        topFile = top
        rememberPosition()
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

    // MARK: - The branch, one commit at a time

    /// The commits in the order they are READ, which is the order they were
    /// made: base forward.
    ///
    /// The opposite of `commitsNewestFirst`, and both are right for what they
    /// are for. A picker is reached for with one commit in mind and it is
    /// almost always the newest, so that list leads with it. Working THROUGH a
    /// branch is a different activity: each commit is one intention, and an
    /// agent's intentions only make sense forwards — the third commit fixes
    /// what the second one introduced, and read backwards it is a repair to
    /// something that has not happened yet. `commits_since` already logs with
    /// `--reverse` for exactly this reading, so this is the wire's own order.
    var commitsInOrder: [ChangeCommit] { changeSet.commits }

    var commitIndex: Int? {
        guard let sha = scope.commitSha else { return nil }
        return commitsInOrder.firstIndex { $0.sha == sha }
    }

    /// `Commit 3 of 12`, for the header of a commit being read in sequence.
    ///
    /// Nil for a commit the branch no longer lists, which an amend mid-review
    /// produces: it has no position in a sequence it is not in, and inventing
    /// one would be the header's one claim that could be flatly wrong.
    var commitPositionLabel: String? {
        guard let commitIndex else { return nil }
        return "Commit \(commitIndex + 1) of \(commitsInOrder.count)"
    }

    var nextCommit: ChangeCommit? {
        guard let commitIndex, commitIndex + 1 < commitsInOrder.count else { return nil }
        return commitsInOrder[commitIndex + 1]
    }

    var previousCommit: ChangeCommit? {
        guard let commitIndex, commitIndex > 0 else { return nil }
        return commitsInOrder[commitIndex - 1]
    }

    /// Begin the itinerary: the first commit the branch made.
    ///
    /// The way in that the History sheet is not. A sheet is the right shape for
    /// "which one of these", and the wrong shape for "start at the beginning
    /// and keep going" — which for an agent-authored branch is the reading that
    /// makes it legible, and the reading this screen exists to support.
    func startAtFirstCommit() async {
        guard let first = commitsInOrder.first else { return }
        await select(commit: first.sha)
    }

    /// Move one commit along the branch, without going back out to a sheet.
    ///
    /// This is what makes commit-by-commit a path rather than a detour: the
    /// reader finishes a commit's last file and the same control that has been
    /// moving them through files moves them to the next intention. Returning to
    /// a picker between every commit is twelve extra taps on a branch of twelve
    /// and a reason not to read it this way at all.
    func showNextCommit() async {
        guard let next = nextCommit else { return }
        await select(commit: next.sha)
    }

    func showPreviousCommit() async {
        guard let previous = previousCommit else { return }
        await select(commit: previous.sha)
    }

    /// Whether Next should carry on into the following commit.
    ///
    /// Only at the END of a commit's files, so the control means one thing at a
    /// time: while there are files left it is "next file", and exactly once per
    /// commit it becomes "next commit" and says so on its face.
    var nextIsCommit: Bool {
        scope.commitSha != nil && !hasNextFile && nextCommit != nil
    }

    // MARK: - What a tool wrote

    /// The files a build regenerated, and the ones a person or an agent wrote.
    ///
    /// Split rather than filtered: a lockfile that moved is a real fact about a
    /// branch, and hiding it would be this screen deciding what the reader is
    /// allowed to see. Only its SIZE is misleading — see `ChangedFile.isGenerated`
    /// — so the two are counted apart and both are listed.
    var generatedFiles: [ChangedFile] { files.filter(\.isGenerated) }
    var handWrittenFiles: [ChangedFile] { files.filter { !$0.isGenerated } }

    /// The files in reading order: what somebody wrote, then what a tool wrote.
    ///
    /// The order the screen draws AND the order Next and Previous walk, which
    /// is why it is one property rather than a sort inside the view: `7 of 23`
    /// has to count the list the reader is looking at, and a display order that
    /// differed from the navigation order would make Next jump backwards up the
    /// screen.
    ///
    /// Generated last rather than in the daemon's order, because the reason
    /// they are marked at all is that they are not what the review is about —
    /// and Next dropping somebody into the middle of a regenerated lockfile,
    /// eleven files before the end, is the tap that ends a review.
    var reviewOrder: [ChangedFile] { handWrittenFiles + generatedFiles }

    /// `4,012 lines` across the generated files, for the line under the counts.
    var generatedLineCount: Int {
        generatedFiles.reduce(0) { $0 + $1.insertions + $1.deletions }
    }

    var writtenInsertions: Int { handWrittenFiles.reduce(0) { $0 + $1.insertions } }
    var writtenDeletions: Int { handWrittenFiles.reduce(0) { $0 + $1.deletions } }
    var generatedInsertions: Int { generatedFiles.reduce(0) { $0 + $1.insertions } }
    var generatedDeletions: Int { generatedFiles.reduce(0) { $0 + $1.deletions } }

    /// The two numbers for the commit on screen, generated files held apart.
    ///
    /// Three sources in order of how much they know, which is not
    /// over-engineering but the three things that are actually true at
    /// different moments:
    ///
    /// 1. When this comparison contains a generated file, the headline is the
    ///    hand-written subtotal — summed from the file list, because the row's
    ///    own `--shortstat` counts the whole commit and the lockfile in it is
    ///    the number this split exists to stop showing.
    /// 2. Otherwise the daemon's own count off `ChangeCommit`, which arrives
    ///    with the change set and is therefore right before the commit's file
    ///    list has landed.
    /// 3. Otherwise the sum of that file list, which is what both clients did
    ///    for as long as the daemon's three count fields were hardcoded zeroes.
    ///
    /// All three are the same number when more than one is available:
    /// `--shortstat` is the sum of the same commit's `--numstat`, with renames
    /// detected on both sides and binaries contributing zero to both.
    var commitCounts: (insertions: Int, deletions: Int)? {
        if !generatedFiles.isEmpty {
            guard !commitFiles.isEmpty else { return nil }
            return (writtenInsertions, writtenDeletions)
        }
        if let counts = selectedCommitInfo?.counts { return counts }
        guard !commitFiles.isEmpty else { return nil }
        return (commitInsertions, commitDeletions)
    }

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
        adoptExpansion()
    }

    // MARK: - Moving through the files

    var currentIndex: Int? {
        guard let expandedFile else { return nil }
        return reviewOrder.firstIndex { $0.path == expandedFile }
    }

    /// `7 of 23`, or the count on its own when nothing is open.
    ///
    /// Said in words rather than left to a scrollbar, because a phone's
    /// scrollbar is a hairline that appears while you drag and disappears while
    /// you read — so the one question a long diff raises, "how much of this is
    /// left", has no answer on screen at the moment it is asked.
    var positionLabel: String {
        let total = reviewOrder.count
        guard total > 0 else { return "No files" }
        guard let currentIndex else {
            return total == 1 ? "1 file" : "\(total) files"
        }
        return "\(currentIndex + 1) of \(total)"
    }

    func isExpanded(_ path: String) -> Bool { expandedFile == path }

    /// Open one file, closing whatever was open. See `expandedFile`.
    func expand(_ path: String?) {
        guard expandedFile != path else { return }
        expandedFile = path
        rememberPosition()
        if let path { jump = Jump(path: path) }
    }

    func toggle(_ path: String) {
        if expandedFile == path {
            expandedFile = nil
            rememberPosition()
        } else {
            expand(path)
        }
    }

    var hasNextFile: Bool {
        guard !reviewOrder.isEmpty else { return false }
        guard let currentIndex else { return true }
        return currentIndex + 1 < reviewOrder.count
    }

    var hasPreviousFile: Bool {
        guard let currentIndex else { return false }
        return currentIndex > 0
    }

    /// The next file, or the first one when the reader is still on the list.
    ///
    /// Starting the sequence from "nothing open" is the point: Next is then the
    /// single control that begins a review as well as continuing one, which on
    /// a phone held in one hand is worth more than the symmetry of having it do
    /// nothing until something is selected.
    func showNextFile() {
        let order = reviewOrder
        guard !order.isEmpty else { return }
        guard let currentIndex else { return expand(order[0].path) }
        guard currentIndex + 1 < order.count else { return }
        expand(order[currentIndex + 1].path)
    }

    func showPreviousFile() {
        guard let currentIndex, currentIndex > 0 else { return }
        expand(reviewOrder[currentIndex - 1].path)
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
            commitFiles = answer.files
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

    /// Keep the open file open, unless it has stopped existing.
    ///
    /// Not "collapse everything on load": a poll that picks up one new commit
    /// would then fold away the file somebody is mid-way through reading, which
    /// on this screen is the whole of what they were doing. Only a file that is
    /// no longer in this scope's list gives way, and it has to — nothing would
    /// draw it.
    private func adoptExpansion() {
        guard let open = expandedFile else { return }
        guard !files.contains(where: { $0.path == open }) else { return }
        expandedFile = nil
        rememberPosition()
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
            prefetchAfter(path)
        } catch {
            // Left unread rather than recorded as empty, so pulling to refresh
            // tries it again instead of showing a permanent blank.
            guard asked == generation else { return }
            fileDiffs[path] = nil
        }
    }

    /// Read the file after this one, so Next lands on a patch instead of on a
    /// spinner.
    ///
    /// Exactly ONE file ahead, and only from the file that is actually open.
    /// Since a file is fetched when it is expanded and only one is ever
    /// expanded, pressing Next used to buy a round trip over somebody's
    /// cellular link every single time — which is the tap this whole screen is
    /// built around. One ahead pays for that with one extra read per file and
    /// no runaway: the prefetched file is not expanded, so it does not fetch
    /// the one after it, and the chain stops at one.
    private func prefetchAfter(_ path: String) {
        guard path == expandedFile else { return }
        let order = reviewOrder
        guard let index = order.firstIndex(where: { $0.path == path }),
            index + 1 < order.count
        else { return }
        let next = order[index + 1].path
        guard fileDiffs[next] == nil, !loadingFiles.contains(next) else { return }
        // Not awaited: the file on screen has landed and nothing about drawing
        // it is waiting on this. A detached read that arrives after the reader
        // has moved on is discarded by the generation check inside `ensure`.
        Task { await ensure(next) }
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

/// The answer to `changes.commit_files`, decoded and believed.
///
/// Believed because the host determines it. `commit_files` in
/// crates/daemon/src/file_diff.rs runs `git diff --name-status -z
/// --find-renames` alongside the `--numstat -z` pass, over the SAME left-hand
/// ref, and `apply_name_status_z` merges the status letter onto the counts.
/// So `added`, `deleted`, `renamed` and `type_changed` on this wire are git's
/// verdicts, not a parser's fallback, and this screen shows them.
///
/// It was not always so, and the shape of what it replaced is worth keeping:
/// `commit_files` used to run `--numstat` alone. That counts lines and cannot
/// say whether a path was created or removed, so `parse_numstat_z` wrote
/// `Modified` into every record it did not recognize as a rename — and a file a
/// commit CREATED crossed the wire labeled modified. This type used to throw
/// the status away for that reason. Deleting the guard is the point of keeping
/// the note: nothing here second-guesses the host any more.
///
/// A daemon older than that fix still sends "modified" for everything, and
/// nothing on this side can tell that from a real modification — the wrong
/// value is well-formed. That is left to `farcooler status` and the app's
/// version-skew notice, which already say when a runner is behind, rather than
/// paid for by every current runner losing badges it computed correctly.
private struct CommitFiles: Decodable {
    var files: [ChangedFile]
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
